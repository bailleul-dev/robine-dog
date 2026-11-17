//! APM service catalog and service pages, computed in GreptimeDB from service
//! entry spans (`top_level`), like Datadog's `trace.<operation>.*` metrics.

use super::{ApiResult, interval_ms, time_clause};
use crate::AppState;
use crate::query::sql::Dataset;
use crate::store::greptime::{Row, f64_at, i64_at, lit, str_at};
use axum::Json;
use axum::extract::{Path, Query, State};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

#[derive(Deserialize)]
pub struct Range {
    from: i64,
    to: i64,
    env: Option<String>,
}

impl Range {
    /// Entry spans of the range (and env), optionally of one service.
    fn entry_filter(&self, service: Option<&str>) -> String {
        let mut w = format!(
            "top_level AND {}",
            time_clause(Dataset::Spans, self.from, self.to)
        );
        if let Some(env) = self.env.as_ref().filter(|e| !e.is_empty() && *e != "*") {
            w.push_str(&format!(" AND env = {}", lit(env)));
        }
        if let Some(s) = service {
            w.push_str(&format!(" AND service = {}", lit(s)));
        }
        w
    }
}

#[derive(Serialize, Default)]
pub struct Stats {
    hits: i64,
    errors: i64,
    error_rate: f64,
    p50: i64,
    p75: i64,
    p90: i64,
    p95: i64,
    p99: i64,
    max: i64,
    avg: i64,
    total_time: i64,
}

/// The 11 columns [`stats_at`] reads, then `min(duration)`.
const STATS: &str = "count(*), sum(CASE WHEN error THEN 1 ELSE 0 END),
    approx_percentile_cont(duration, 0.5), approx_percentile_cont(duration, 0.75),
    approx_percentile_cont(duration, 0.9), approx_percentile_cont(duration, 0.95),
    approx_percentile_cont(duration, 0.99), max(duration), avg(duration),
    sum(duration), min(duration)";

fn stats_at(r: &Row, i: usize) -> Stats {
    let hits = i64_at(r, i);
    let errors = i64_at(r, i + 1);
    let n = |k: usize| f64_at(r, i + k).unwrap_or(0.0) as i64;
    Stats {
        hits,
        errors,
        error_rate: if hits > 0 {
            errors as f64 / hits as f64
        } else {
            0.0
        },
        p50: n(2),
        p75: n(3),
        p90: n(4),
        p95: n(5),
        p99: n(6),
        max: n(7),
        avg: n(8),
        total_time: n(9),
    }
}

/// Per service: the most common span type and operation name.
async fn kinds(
    st: &AppState,
    filter: &str,
) -> anyhow::Result<HashMap<String, (String, Vec<String>)>> {
    let rows = st
        .store
        .gt
        .query(&format!(
            "SELECT service, type, name, count(*) AS n FROM spans WHERE {filter}
             GROUP BY service, type, name ORDER BY n DESC"
        ))
        .await?;
    let mut out: HashMap<String, (String, Vec<String>)> = HashMap::new();
    for r in &rows {
        let service = str_at(r, 0).unwrap_or_default();
        let e = out
            .entry(service)
            .or_insert_with(|| (str_at(r, 1).unwrap_or_else(|| "custom".into()), Vec::new()));
        let op = str_at(r, 2).unwrap_or_default();
        if !e.1.contains(&op) {
            e.1.push(op);
        }
    }
    Ok(out)
}

#[derive(Serialize)]
pub struct ServiceSummary {
    name: String,
    #[serde(rename = "type")]
    typ: String,
    operation: String,
    #[serde(flatten)]
    stats: Stats,
    requests_per_second: f64,
    /// Hits per bucket, for the sparkline.
    sparkline: Vec<i64>,
}

pub async fn services(
    State(st): State<AppState>,
    Query(r): Query<Range>,
) -> ApiResult<Vec<ServiceSummary>> {
    let filter = r.entry_filter(None);
    let buckets = 24i64;
    let from_ns = r.from.saturating_mul(1_000_000);
    let width = ((r.to - r.from).max(1) * 1_000_000 / buckets).max(1);
    let gt = &st.store.gt;
    let stats_sql = format!(
        "SELECT service, {STATS} FROM spans WHERE {filter} GROUP BY service ORDER BY service"
    );
    let spark_sql = format!(
        "SELECT service, (CAST(ts AS BIGINT) - {from_ns}) / {width} AS i, count(*)
         FROM spans WHERE {filter} GROUP BY service, i"
    );
    let (stats, spark, kinds) = tokio::try_join!(
        gt.query(&stats_sql),
        gt.query(&spark_sql),
        kinds(&st, &filter)
    )?;
    let mut sparks: HashMap<String, Vec<i64>> = HashMap::new();
    for row in &spark {
        let i = i64_at(row, 1).clamp(0, buckets - 1) as usize;
        sparks
            .entry(str_at(row, 0).unwrap_or_default())
            .or_insert_with(|| vec![0; buckets as usize])[i] += i64_at(row, 2);
    }
    let seconds = ((r.to - r.from) as f64 / 1000.0).max(1.0);
    Ok(Json(
        stats
            .iter()
            .map(|row| {
                let name = str_at(row, 0).unwrap_or_default();
                let stats = stats_at(row, 1);
                let (typ, ops) = kinds.get(&name).cloned().unwrap_or_default();
                ServiceSummary {
                    typ: if typ.is_empty() { "custom".into() } else { typ },
                    operation: ops.first().cloned().unwrap_or_default(),
                    requests_per_second: stats.hits as f64 / seconds,
                    sparkline: sparks
                        .remove(&name)
                        .unwrap_or_else(|| vec![0; buckets as usize]),
                    stats,
                    name,
                }
            })
            .collect(),
    ))
}

#[derive(Serialize)]
pub struct Point {
    t: i64,
    hits: i64,
    errors: i64,
    p50: i64,
    p95: i64,
    p99: i64,
    max: i64,
}

#[derive(Serialize)]
pub struct ResourceRow {
    resource: String,
    operation: String,
    #[serde(flatten)]
    stats: Stats,
}

#[derive(Serialize)]
pub struct ServiceDetail {
    name: String,
    #[serde(rename = "type")]
    typ: String,
    operations: Vec<String>,
    summary: Stats,
    interval_ms: i64,
    timeseries: Vec<Point>,
    resources: Vec<ResourceRow>,
    /// Latency distribution: `[bucket upper bound in ns, count]`, log scale.
    distribution: Vec<(i64, i64)>,
    downstream: Vec<Dependency>,
    /// The versions that served requests in the range, latest first.
    deployments: Vec<Deployment>,
}

/// One version of the service (the `version` of its spans).
#[derive(Serialize)]
pub struct Deployment {
    version: String,
    /// Epoch milliseconds, looking back 30 days.
    first_seen: i64,
    last_seen: i64,
    #[serde(flatten)]
    stats: Stats,
}

#[derive(Serialize)]
pub struct Dependency {
    service: String,
    calls: i64,
    errors: i64,
    avg: i64,
}

const DISTRIBUTION_BUCKETS: i64 = 40;

pub async fn service(
    State(st): State<AppState>,
    Path(name): Path<String>,
    Query(r): Query<Range>,
) -> ApiResult<ServiceDetail> {
    let filter = r.entry_filter(Some(&name));
    let interval = interval_ms(r.from, r.to, 60);
    let gt = &st.store.gt;
    let span_window = time_clause(Dataset::Spans, r.from, r.to);
    let env = r
        .env
        .as_ref()
        .filter(|e| !e.is_empty() && *e != "*")
        .map(|e| format!(" AND c.env = {}", lit(e)))
        .unwrap_or_default();
    // Calls from this service's spans into other services' entry spans.
    let downstream_sql = format!(
        "SELECT c.service, count(*) AS n, sum(CASE WHEN c.error THEN 1 ELSE 0 END), avg(c.duration)
         FROM spans c JOIN spans p ON p.trace_id = c.trace_id AND p.span_id = c.parent_id
         WHERE p.service = {svc} AND c.service != {svc} AND c.top_level{env}
           AND c.{w1} AND p.{w2}
         GROUP BY c.service ORDER BY n DESC",
        svc = lit(&name),
        w1 = span_window.replace(" AND ts", " AND c.ts"),
        w2 = span_window.replace(" AND ts", " AND p.ts"),
    );
    let summary_sql = format!("SELECT {STATS} FROM spans WHERE {filter}");
    let series_sql = format!(
        "SELECT CAST(date_bin(INTERVAL '{interval} milliseconds', ts) AS BIGINT) AS b, {STATS}
         FROM spans WHERE {filter} GROUP BY b"
    );
    let resources_sql =
        format!("SELECT resource, name, {STATS} FROM spans WHERE {filter} GROUP BY resource, name");
    let versions_sql = format!(
        "SELECT version, CAST(max(ts) AS BIGINT), {STATS} FROM spans
         WHERE {filter} AND version IS NOT NULL AND version != '' GROUP BY version"
    );
    // When each version first served a request, beyond the range shown.
    let first_seen_sql = format!(
        "SELECT version, CAST(min(ts) AS BIGINT) FROM spans
         WHERE top_level AND service = {} AND version IS NOT NULL AND {} GROUP BY version",
        lit(&name),
        time_clause(Dataset::Spans, r.to - 30 * 86_400_000, r.to)
    );
    let (summary, series, resources, kinds, downstream, versions, first_seen) = tokio::try_join!(
        gt.query(&summary_sql),
        gt.query(&series_sql),
        gt.query(&resources_sql),
        kinds(&st, &filter),
        gt.query(&downstream_sql),
        gt.query(&versions_sql),
        gt.query(&first_seen_sql),
    )?;
    let first_seen: HashMap<String, i64> = first_seen
        .iter()
        .filter_map(|row| Some((str_at(row, 0)?, i64_at(row, 1) / 1_000_000)))
        .collect();
    let mut deployments: Vec<Deployment> = versions
        .iter()
        .filter_map(|row| {
            let version = str_at(row, 0)?;
            let last_seen = i64_at(row, 1) / 1_000_000;
            Some(Deployment {
                first_seen: first_seen.get(&version).copied().unwrap_or(last_seen),
                last_seen,
                stats: stats_at(row, 2),
                version,
            })
        })
        .collect();
    deployments.sort_by_key(|d| std::cmp::Reverse(d.first_seen));
    let summary_row = summary.first().cloned().unwrap_or_default();
    let summary = stats_at(&summary_row, 0);
    let min_d = f64_at(&summary_row, 10).unwrap_or(1.0).max(1.0);
    let max_d = (summary.max.max(1)) as f64;

    let mut distribution = Vec::new();
    if summary.hits > 0 {
        let lo = min_d.log10().floor();
        let hi = max_d.log10().ceil().max(lo + 1.0);
        let step = (hi - lo) / DISTRIBUTION_BUCKETS as f64;
        let rows = gt
            .query(&format!(
                "SELECT LEAST(CAST(floor((log10(CAST(GREATEST(duration, 1) AS DOUBLE)) - {lo:?}) / {step:?}) AS BIGINT), {last}) AS i, count(*)
                 FROM spans WHERE {filter} GROUP BY i",
                last = DISTRIBUTION_BUCKETS - 1
            ))
            .await?;
        let mut counts = vec![0i64; DISTRIBUTION_BUCKETS as usize];
        for row in &rows {
            counts[i64_at(row, 0).clamp(0, DISTRIBUTION_BUCKETS - 1) as usize] += i64_at(row, 1);
        }
        distribution = counts
            .into_iter()
            .enumerate()
            .map(|(i, c)| (10f64.powf(lo + step * (i + 1) as f64) as i64, c))
            .collect();
    }

    let mut by_bucket: BTreeMap<i64, Point> = BTreeMap::new();
    let mut t = r.from - r.from.rem_euclid(interval);
    while t <= r.to {
        by_bucket.insert(
            t,
            Point {
                t,
                hits: 0,
                errors: 0,
                p50: 0,
                p95: 0,
                p99: 0,
                max: 0,
            },
        );
        t += interval;
    }
    for row in &series {
        let t = i64_at(row, 0) / 1_000_000;
        let s = stats_at(row, 1);
        by_bucket.insert(
            t,
            Point {
                t,
                hits: s.hits,
                errors: s.errors,
                p50: s.p50,
                p95: s.p95,
                p99: s.p99,
                max: s.max,
            },
        );
    }

    let mut resources: Vec<ResourceRow> = resources
        .iter()
        .map(|row| ResourceRow {
            resource: str_at(row, 0).unwrap_or_default(),
            operation: str_at(row, 1).unwrap_or_default(),
            stats: stats_at(row, 2),
        })
        .collect();
    resources.sort_by_key(|r| std::cmp::Reverse(r.stats.hits));
    let (typ, operations) = kinds.get(&name).cloned().unwrap_or_default();

    Ok(Json(ServiceDetail {
        typ: if typ.is_empty() { "custom".into() } else { typ },
        operations,
        summary,
        interval_ms: interval,
        timeseries: by_bucket.into_values().collect(),
        resources,
        distribution,
        deployments,
        downstream: downstream
            .iter()
            .map(|row| Dependency {
                service: str_at(row, 0).unwrap_or_default(),
                calls: i64_at(row, 1),
                errors: i64_at(row, 2),
                avg: f64_at(row, 3).unwrap_or(0.0) as i64,
            })
            .collect(),
        name,
    }))
}

#[derive(Serialize)]
pub struct Edge {
    source: String,
    target: String,
    calls: i64,
    errors: i64,
    avg: i64,
}

/// The service map's edges: calls from a span of one service into an entry
/// span of another, in the same trace.
pub async fn map(State(st): State<AppState>, Query(r): Query<Range>) -> ApiResult<Vec<Edge>> {
    let span_window = time_clause(Dataset::Spans, r.from, r.to);
    let env = r
        .env
        .as_ref()
        .filter(|e| !e.is_empty() && *e != "*")
        .map(|e| format!(" AND c.env = {}", lit(e)))
        .unwrap_or_default();
    let sql = format!(
        "SELECT p.service, c.service, count(*) AS n, sum(CASE WHEN c.error THEN 1 ELSE 0 END), avg(c.duration)
         FROM spans c JOIN spans p ON p.trace_id = c.trace_id AND p.span_id = c.parent_id
         WHERE c.service != p.service AND c.top_level{env}
           AND c.{w1} AND p.{w2}
         GROUP BY p.service, c.service ORDER BY n DESC LIMIT 500",
        w1 = span_window.replace(" AND ts", " AND c.ts"),
        w2 = span_window.replace(" AND ts", " AND p.ts"),
    );
    let rows = st.store.gt.query(&sql).await?;
    Ok(Json(
        rows.iter()
            .map(|row| Edge {
                source: str_at(row, 0).unwrap_or_default(),
                target: str_at(row, 1).unwrap_or_default(),
                calls: i64_at(row, 2),
                errors: i64_at(row, 3),
                avg: f64_at(row, 4).unwrap_or(0.0) as i64,
            })
            .collect(),
    ))
}

pub async fn envs(State(st): State<AppState>) -> ApiResult<Vec<String>> {
    let rows = st
        .store
        .gt
        .query("SELECT DISTINCT env FROM spans WHERE env IS NOT NULL ORDER BY env")
        .await?;
    Ok(Json(rows.iter().filter_map(|r| str_at(r, 0)).collect()))
}
