//! Log and span analytics, shaped like Datadog's aggregate API
//! (`POST /api/v2/logs/analytics/aggregate`, `/api/v2/spans/analytics/aggregate`):
//! `compute` (count, cardinality, sum/avg/min/max, median, pcNN of a measure,
//! as a total or a timeseries), `group_by` facets with top-N limits, and
//! buckets back as `{by: {facet: value}, computes: {c0: …}}`. The explorers'
//! Timeseries / Top List / Table views use the same engine.

use super::{ApiError, interval_ms, parse_query, time_clause};
use crate::AppState;
use crate::query::sql::{self, Dataset, Target};
use crate::store::greptime::{f64_at, lit, str_at};
use axum::Json;
use axum::extract::State;
use serde::Deserialize;
use serde_json::{Map, Value, json};
use std::collections::HashMap;

#[derive(Debug, Clone, Deserialize)]
pub struct Compute {
    #[serde(default = "count")]
    pub aggregation: String,
    #[serde(default)]
    pub metric: Option<String>,
    /// `total` (default) or `timeseries`.
    #[serde(default, rename = "type")]
    pub typ: Option<String>,
    /// `5m`, `1h`, `30s`… or milliseconds.
    #[serde(default)]
    pub interval: Option<Value>,
}

fn count() -> String {
    "count".into()
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Sort {
    #[serde(default)]
    pub aggregation: Option<String>,
    #[serde(default)]
    pub metric: Option<String>,
    #[serde(default)]
    pub order: Option<String>,
    /// `measure` (default) or `alphabetical`.
    #[serde(default, rename = "type")]
    pub typ: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GroupBy {
    pub facet: String,
    #[serde(default)]
    pub limit: Option<i64>,
    #[serde(default)]
    pub sort: Option<Sort>,
    /// Value given to events without the facet (they are left out otherwise).
    #[serde(default)]
    pub missing: Option<Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Filter {
    #[serde(default)]
    pub query: String,
    #[serde(default = "default_from")]
    pub from: Value,
    #[serde(default = "default_to")]
    pub to: Value,
}

fn default_from() -> Value {
    json!("now-15m")
}

fn default_to() -> Value {
    json!("now")
}

#[derive(Debug, Clone, Deserialize)]
pub struct AggregateReq {
    #[serde(default)]
    pub compute: Vec<Compute>,
    #[serde(default = "all")]
    pub filter: Filter,
    #[serde(default)]
    pub group_by: Vec<GroupBy>,
}

fn all() -> Filter {
    Filter {
        query: String::new(),
        from: default_from(),
        to: default_to(),
    }
}

/// `now-15m`, `now`, an ISO date, or epoch milliseconds (number or digits).
pub fn parse_time(v: &Value, now: i64) -> Option<i64> {
    match v {
        Value::Number(n) => n.as_f64().map(|f| f as i64),
        Value::String(s) => {
            let s = s.trim();
            if let Some(rest) = s.strip_prefix("now") {
                if rest.is_empty() {
                    return Some(now);
                }
                let (sign, d) = match rest.split_at(1) {
                    ("-", d) => (-1, d),
                    ("+", d) => (1, d),
                    _ => return None,
                };
                return Some(now + sign * parse_span_ms(d)?);
            }
            if s.chars().all(|c| c.is_ascii_digit()) {
                return s.parse().ok();
            }
            chrono::DateTime::parse_from_rfc3339(s)
                .ok()
                .map(|d| d.timestamp_millis())
        }
        _ => None,
    }
}

/// `30s`, `5m`, `1h`, `2d`, `1w` (or bare milliseconds).
pub fn parse_span_ms(s: &str) -> Option<i64> {
    let s = s.trim();
    let i = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    let n: i64 = s[..i].parse().ok()?;
    let unit = match &s[i..] {
        "" | "ms" => 1,
        "s" => 1_000,
        "m" => 60_000,
        "h" => 3_600_000,
        "d" => 86_400_000,
        "w" => 604_800_000,
        _ => return None,
    };
    Some(n * unit)
}

fn table(ds: Dataset) -> &'static str {
    match ds {
        Dataset::Spans => "spans",
        _ => "logs",
    }
}

/// A facet's value as text (NULL when absent).
fn text_expr(facet: &str, ds: Dataset) -> String {
    match sql::resolve(facet, ds) {
        Target::Tag(key) => {
            let needle = format!(",{key}:");
            format!(
                "(CASE WHEN strpos(tags, {}) > 0 THEN regexp_replace(tags, {}, '${{1}}') END)",
                lit(&needle),
                lit(&format!("^.*,{}:([^,]*),.*$", regex::escape(&key)))
            )
        }
        Target::Duration => "CAST(duration AS STRING)".into(),
        t => format!("CAST({} AS STRING)", sql::target_expr(&t)),
    }
}

/// The SQL of one aggregation.
fn aggregate_sql(aggregation: &str, metric: Option<&str>, ds: Dataset) -> Result<String, ApiError> {
    let measure = || -> Result<String, ApiError> {
        let m =
            metric.ok_or_else(|| ApiError::BadQuery(format!("{aggregation} needs a metric")))?;
        Ok(sql::number_expr(&sql::resolve(m, ds)))
    };
    Ok(match aggregation {
        "count" => match metric {
            None => "CAST(count(*) AS DOUBLE)".into(),
            Some(m) => format!("CAST(count({}) AS DOUBLE)", text_expr(m, ds)),
        },
        "cardinality" => {
            let m =
                metric.ok_or_else(|| ApiError::BadQuery("cardinality needs a metric".into()))?;
            format!("CAST(count(DISTINCT {}) AS DOUBLE)", text_expr(m, ds))
        }
        "sum" | "avg" | "min" | "max" => format!("CAST({aggregation}({}) AS DOUBLE)", measure()?),
        "median" => format!("approx_percentile_cont({}, 0.5)", measure()?),
        p if p.starts_with("pc") => {
            let q: f64 = p[2..]
                .parse()
                .ok()
                .filter(|q| (0.0..=100.0).contains(q))
                .ok_or_else(|| ApiError::BadQuery(format!("unknown aggregation {p}")))?;
            format!("approx_percentile_cont({}, {})", measure()?, q / 100.0)
        }
        other => return Err(ApiError::BadQuery(format!("unknown aggregation {other}"))),
    })
}

fn missing_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// Spans are timestamped in nanoseconds, logs in milliseconds.
fn bucket_ms(ds: Dataset, b: f64) -> i64 {
    match ds {
        Dataset::Spans => (b / 1_000_000.0) as i64,
        _ => b as i64,
    }
}

fn iso(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|d| d.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string())
        .unwrap_or_default()
}

pub struct Outcome {
    pub buckets: Vec<Value>,
    pub interval_ms: Option<i64>,
}

pub async fn run(
    st: &AppState,
    ds: Dataset,
    req: &AggregateReq,
    now: i64,
) -> Result<Outcome, ApiError> {
    let from = parse_time(&req.filter.from, now)
        .ok_or_else(|| ApiError::BadQuery("bad filter.from".into()))?;
    let to = parse_time(&req.filter.to, now)
        .ok_or_else(|| ApiError::BadQuery("bad filter.to".into()))?;
    if req.group_by.len() > 4 {
        return Err(ApiError::BadQuery("at most 4 group_by facets".into()));
    }
    let computes = if req.compute.is_empty() {
        vec![Compute {
            aggregation: "count".into(),
            metric: None,
            typ: None,
            interval: None,
        }]
    } else {
        req.compute.clone()
    };
    let filter = sql::compile(&parse_query(&req.filter.query)?, ds);
    let base = format!(
        "FROM {} WHERE {} AND {filter}",
        table(ds),
        time_clause(ds, from, to)
    );
    let gt = &st.store.gt;

    // Group expressions, and each facet's top values (overall, by its sort).
    let mut groups = Vec::new();
    let mut ranking = Vec::new();
    let mut conds = Vec::new();
    for g in &req.group_by {
        let raw = text_expr(&g.facet, ds);
        let e = match &g.missing {
            Some(m) => format!("COALESCE({raw}, {})", lit(&missing_text(m))),
            None => raw,
        };
        let limit = g.limit.unwrap_or(10).clamp(1, 1000);
        let sort = g.sort.clone().unwrap_or_default();
        let desc = sort.order.as_deref() != Some("asc");
        let order_by = if sort.typ.as_deref() == Some("alphabetical") {
            e.clone()
        } else {
            aggregate_sql(
                sort.aggregation.as_deref().unwrap_or("count"),
                sort.metric.as_deref(),
                ds,
            )?
        };
        let rows = gt
            .query(&format!(
                "SELECT {e} AS v {base} AND {e} IS NOT NULL GROUP BY {e} ORDER BY {order_by} {} NULLS LAST LIMIT {limit}",
                if desc { "DESC" } else { "ASC" }
            ))
            .await?;
        let top: Vec<String> = rows.iter().filter_map(|r| str_at(r, 0)).collect();
        if top.is_empty() {
            return Ok(Outcome {
                buckets: vec![],
                interval_ms: None,
            });
        }
        conds.push(format!(
            "{e} IN ({})",
            top.iter().map(|v| lit(v)).collect::<Vec<_>>().join(", ")
        ));
        groups.push((g.facet.clone(), e, top));
        ranking.push((order_by, desc, limit));
    }
    let where_groups: String = conds.iter().map(|c| format!(" AND {c}")).collect();
    // Totals over several facets are hierarchical, as in Datadog: the top
    // values of the last facet within each combination of the others.
    let nested = groups.len() >= 2;
    let where_parents: String = conds[..conds.len().saturating_sub(1)]
        .iter()
        .map(|c| format!(" AND {c}"))
        .collect();
    let group_cols: Vec<String> = groups
        .iter()
        .enumerate()
        .map(|(i, (_, e, _))| format!("{e} AS g{i}"))
        .collect();
    let group_names: Vec<String> = (0..groups.len()).map(|i| format!("g{i}")).collect();

    type Key = Vec<String>;
    let mut totals: HashMap<Key, Map<String, Value>> = HashMap::new();
    let mut series: HashMap<Key, Map<String, Value>> = HashMap::new();
    let mut interval_out = None;
    let mut last_rank: HashMap<Key, usize> = HashMap::new();

    let totals_idx: Vec<usize> = (0..computes.len())
        .filter(|i| computes[*i].typ.as_deref() != Some("timeseries"))
        .collect();
    let series_idx: Vec<usize> = (0..computes.len())
        .filter(|i| computes[*i].typ.as_deref() == Some("timeseries"))
        .collect();

    let key_of = |r: &crate::store::greptime::Row| -> Key {
        (0..groups.len())
            .map(|i| str_at(r, i).unwrap_or_default())
            .collect()
    };

    if !totals_idx.is_empty() {
        let aggs = totals_idx
            .iter()
            .map(|i| {
                aggregate_sql(
                    &computes[*i].aggregation,
                    computes[*i].metric.as_deref(),
                    ds,
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut select: Vec<String> = group_cols.iter().cloned().chain(aggs).collect();
        let group = if group_names.is_empty() {
            String::new()
        } else {
            format!(" GROUP BY {}", group_names.join(", "))
        };
        let sql = if nested {
            let (order_by, desc, limit) = &ranking[groups.len() - 1];
            let parents: Vec<&str> = groups[..groups.len() - 1]
                .iter()
                .map(|(_, e, _)| e.as_str())
                .collect();
            select.push(format!(
                "ROW_NUMBER() OVER (PARTITION BY {} ORDER BY {order_by} {}) AS rn",
                parents.join(", "),
                if *desc { "DESC" } else { "ASC" }
            ));
            let last = &groups[groups.len() - 1].1;
            format!(
                "SELECT * FROM (SELECT {} {base}{where_parents} AND {last} IS NOT NULL{group}) WHERE rn <= {limit}",
                select.join(", ")
            )
        } else {
            format!("SELECT {} {base}{where_groups}{group}", select.join(", "))
        };
        for r in gt.query(&sql).await? {
            if nested {
                let rn = f64_at(&r, groups.len() + totals_idx.len()).unwrap_or(0.0);
                last_rank.insert(key_of(&r), rn as usize);
            }
            let entry = totals.entry(key_of(&r)).or_default();
            for (j, i) in totals_idx.iter().enumerate() {
                entry.insert(format!("c{i}"), json!(f64_at(&r, groups.len() + j)));
            }
        }
    }

    if !series_idx.is_empty() {
        let interval = series_idx
            .iter()
            .find_map(|i| match &computes[*i].interval {
                Some(Value::Number(n)) => n.as_i64(),
                Some(Value::String(s)) => parse_span_ms(s),
                _ => None,
            })
            .unwrap_or_else(|| interval_ms(from, to, 60))
            .max(1_000);
        interval_out = Some(interval);
        let aggs = series_idx
            .iter()
            .map(|i| {
                aggregate_sql(
                    &computes[*i].aggregation,
                    computes[*i].metric.as_deref(),
                    ds,
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        let bucket =
            format!("CAST(date_bin(INTERVAL '{interval} milliseconds', ts) AS BIGINT) AS b");
        let select: Vec<String> = group_cols
            .iter()
            .cloned()
            .chain(std::iter::once(bucket))
            .chain(aggs)
            .collect();
        let group: Vec<String> = group_names
            .iter()
            .cloned()
            .chain(std::iter::once("b".into()))
            .collect();
        let rows = gt
            .query(&format!(
                "SELECT {} {base}{where_groups} GROUP BY {} ORDER BY b",
                select.join(", "),
                group.join(", ")
            ))
            .await?;
        for r in rows {
            let t = bucket_ms(ds, f64_at(&r, groups.len()).unwrap_or(0.0));
            let entry = series.entry(key_of(&r)).or_default();
            for (j, i) in series_idx.iter().enumerate() {
                let v = f64_at(&r, groups.len() + 1 + j);
                if let Value::Array(points) = entry
                    .entry(format!("c{i}"))
                    .or_insert_with(|| Value::Array(vec![]))
                {
                    points.push(json!({"time": iso(t), "value": v}));
                }
            }
        }
    }

    // Buckets in the facets' top order.
    let rank: Vec<HashMap<&str, usize>> = groups
        .iter()
        .map(|(_, _, top)| {
            top.iter()
                .enumerate()
                .map(|(i, v)| (v.as_str(), i))
                .collect()
        })
        .collect();
    let mut keys: Vec<Key> = totals.keys().chain(series.keys()).cloned().collect();
    keys.sort_by_key(|k| {
        k.iter()
            .enumerate()
            .map(|(i, v)| match last_rank.get(k) {
                Some(r) if i == k.len() - 1 => *r,
                _ => rank[i].get(v.as_str()).copied().unwrap_or(usize::MAX),
            })
            .collect::<Vec<_>>()
    });
    keys.dedup();
    let buckets = keys
        .into_iter()
        .map(|k| {
            let by: Map<String, Value> = groups
                .iter()
                .zip(&k)
                .map(|((facet, _, _), v)| (facet.clone(), json!(v)))
                .collect();
            let mut computes = totals.remove(&k).unwrap_or_default();
            computes.extend(series.remove(&k).unwrap_or_default());
            json!({"by": by, "computes": computes})
        })
        .collect();
    Ok(Outcome {
        buckets,
        interval_ms: interval_out,
    })
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

async fn respond(st: AppState, ds: Dataset, req: AggregateReq) -> Result<Json<Value>, ApiError> {
    let started = std::time::Instant::now();
    let out = run(&st, ds, &req, now_ms()).await?;
    Ok(Json(json!({
        "data": {"buckets": out.buckets},
        "meta": {
            "status": "done",
            "elapsed": started.elapsed().as_millis() as i64,
            "interval_ms": out.interval_ms,
        },
    })))
}

pub async fn logs(
    State(st): State<AppState>,
    Json(req): Json<AggregateReq>,
) -> Result<Json<Value>, ApiError> {
    respond(st, Dataset::Logs, req).await
}

pub async fn spans(
    State(st): State<AppState>,
    Json(req): Json<AggregateReq>,
) -> Result<Json<Value>, ApiError> {
    respond(st, Dataset::Spans, req).await
}

/// Datadog's spans aggregate API wraps the request in `data.attributes`.
pub async fn spans_public(
    State(st): State<AppState>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    let inner = body.pointer("/data/attributes").cloned().unwrap_or(body);
    let req: AggregateReq =
        serde_json::from_value(inner).map_err(|e| ApiError::BadQuery(e.to_string()))?;
    respond(st, Dataset::Spans, req).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times() {
        let now = 1_794_846_660_000;
        assert_eq!(parse_time(&json!("now"), now), Some(now));
        assert_eq!(parse_time(&json!("now-15m"), now), Some(now - 900_000));
        assert_eq!(parse_time(&json!("now-1d"), now), Some(now - 86_400_000));
        assert_eq!(parse_time(&json!(1_000), now), Some(1_000));
        assert_eq!(parse_time(&json!("1000"), now), Some(1_000));
        assert_eq!(parse_time(&json!("2026-11-16T16:31:00Z"), now), Some(now));
        assert_eq!(parse_time(&json!("yesterday"), now), None);
    }

    #[test]
    fn aggregations() {
        assert_eq!(
            aggregate_sql("count", None, Dataset::Logs).unwrap(),
            "CAST(count(*) AS DOUBLE)"
        );
        assert!(
            aggregate_sql("pc95", Some("@duration"), Dataset::Spans)
                .unwrap()
                .contains("approx_percentile_cont(duration, 0.95)")
        );
        assert!(aggregate_sql("avg", None, Dataset::Logs).is_err());
        assert!(aggregate_sql("pcx", Some("@d"), Dataset::Logs).is_err());
        assert!(text_expr("env", Dataset::Logs).contains("strpos(tags, ',env:')"));
    }
}
