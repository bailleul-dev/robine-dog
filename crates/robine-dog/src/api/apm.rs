//! APM service catalog and service pages, computed from service entry spans
//! (`top_level`), like Datadog's trace metrics `trace.<operation>.hits/errors/duration`.

use super::{ApiResult, interval_ms};
use crate::AppState;
use axum::Json;
use axum::extract::{Path, Query, State};
use rusqlite::types::Value as SqlValue;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

#[derive(Deserialize)]
pub struct Range {
    from: i64,
    to: i64,
    env: Option<String>,
}

fn pct(sorted: &[i64], p: f64) -> i64 {
    if sorted.is_empty() {
        return 0;
    }
    let idx = ((sorted.len() as f64 - 1.0) * p).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

#[derive(Default, Clone)]
struct Acc {
    durations: Vec<i64>,
    errors: i64,
    total: i64,
}

impl Acc {
    fn add(&mut self, d: i64, e: bool) {
        self.durations.push(d);
        self.total += d;
        self.errors += e as i64;
    }

    fn stats(mut self) -> Stats {
        self.durations.sort_unstable();
        let hits = self.durations.len() as i64;
        Stats {
            hits,
            errors: self.errors,
            error_rate: if hits > 0 {
                self.errors as f64 / hits as f64
            } else {
                0.0
            },
            p50: pct(&self.durations, 0.50),
            p75: pct(&self.durations, 0.75),
            p90: pct(&self.durations, 0.90),
            p95: pct(&self.durations, 0.95),
            p99: pct(&self.durations, 0.99),
            max: self.durations.last().copied().unwrap_or(0),
            avg: if hits > 0 { self.total / hits } else { 0 },
            total_time: self.total,
        }
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

struct EntrySpan {
    service: String,
    name: String,
    resource: String,
    typ: Option<String>,
    start: i64,
    duration: i64,
    error: bool,
}

fn entry_spans(
    c: &rusqlite::Connection,
    r: &Range,
    service: Option<&str>,
) -> anyhow::Result<Vec<EntrySpan>> {
    let mut sql = String::from(
        "SELECT service, name, resource, type, start, duration, error FROM spans WHERE top_level = 1 AND start >= ? AND start <= ?",
    );
    let mut params: Vec<SqlValue> = vec![
        (r.from * 1_000_000).into(),
        (r.to * 1_000_000 + 999_999).into(),
    ];
    if let Some(env) = r.env.as_ref().filter(|e| !e.is_empty() && *e != "*") {
        sql.push_str(" AND env = ?");
        params.push(env.clone().into());
    }
    if let Some(s) = service {
        sql.push_str(" AND service = ?");
        params.push(s.to_string().into());
    }
    Ok(c.prepare(&sql)?
        .query_map(rusqlite::params_from_iter(params), |row| {
            Ok(EntrySpan {
                service: row.get(0)?,
                name: row.get(1)?,
                resource: row.get(2)?,
                typ: row.get(3)?,
                start: row.get(4)?,
                duration: row.get(5)?,
                error: row.get(6)?,
            })
        })?
        .collect::<Result<_, _>>()?)
}

/// Per service: latencies, span types, operation names, sparkline buckets.
type ServiceAcc = (Acc, HashMap<String, i64>, HashMap<String, i64>, Vec<i64>);

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

fn most_common(counts: HashMap<String, i64>) -> String {
    counts
        .into_iter()
        .max_by_key(|(_, n)| *n)
        .map(|(k, _)| k)
        .unwrap_or_default()
}

pub async fn services(
    State(st): State<AppState>,
    Query(r): Query<Range>,
) -> ApiResult<Vec<ServiceSummary>> {
    let out = st
        .db
        .read(move |c| {
            let spans = entry_spans(c, &r, None)?;
            let buckets = 24i64;
            let width = ((r.to - r.from).max(1) * 1_000_000) / buckets;
            let mut by: BTreeMap<String, ServiceAcc> = BTreeMap::new();
            for s in spans {
                let e = by.entry(s.service.clone()).or_insert_with(|| {
                    (
                        Acc::default(),
                        HashMap::new(),
                        HashMap::new(),
                        vec![0; buckets as usize],
                    )
                });
                e.0.add(s.duration, s.error);
                *e.1.entry(s.typ.clone().unwrap_or_else(|| "custom".into()))
                    .or_default() += 1;
                *e.2.entry(s.name.clone()).or_default() += 1;
                let b =
                    ((s.start - r.from * 1_000_000) / width.max(1)).clamp(0, buckets - 1) as usize;
                e.3[b] += 1;
            }
            let seconds = ((r.to - r.from) as f64 / 1000.0).max(1.0);
            Ok(by
                .into_iter()
                .map(|(name, (acc, types, ops, sparkline))| {
                    let stats = acc.stats();
                    ServiceSummary {
                        name,
                        typ: most_common(types),
                        operation: most_common(ops),
                        requests_per_second: stats.hits as f64 / seconds,
                        stats,
                        sparkline,
                    }
                })
                .collect::<Vec<_>>())
        })
        .await?;
    Ok(Json(out))
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
}

#[derive(Serialize)]
pub struct Dependency {
    service: String,
    calls: i64,
    errors: i64,
    avg: i64,
}

pub async fn service(
    State(st): State<AppState>,
    Path(name): Path<String>,
    Query(r): Query<Range>,
) -> ApiResult<ServiceDetail> {
    let out = st
        .db
        .read(move |c| {
            let spans = entry_spans(c, &r, Some(&name))?;
            let interval = interval_ms(r.from, r.to, 60);
            let mut total = Acc::default();
            let mut series: BTreeMap<i64, Acc> = BTreeMap::new();
            let mut t = r.from - r.from.rem_euclid(interval);
            while t <= r.to {
                series.insert(t, Acc::default());
                t += interval;
            }
            let mut resources: HashMap<(String, String), Acc> = HashMap::new();
            let mut types = HashMap::new();
            let mut ops = HashMap::new();
            let mut min_d = i64::MAX;
            let mut max_d = 0;
            for s in &spans {
                total.add(s.duration, s.error);
                let ms = s.start / 1_000_000;
                series.entry(ms - ms.rem_euclid(interval)).or_default().add(s.duration, s.error);
                resources.entry((s.resource.clone(), s.name.clone())).or_default().add(s.duration, s.error);
                *types.entry(s.typ.clone().unwrap_or_else(|| "custom".into())).or_default() += 1;
                *ops.entry(s.name.clone()).or_default() += 1i64;
                min_d = min_d.min(s.duration.max(1));
                max_d = max_d.max(s.duration);
            }

            let mut distribution = Vec::new();
            if !spans.is_empty() {
                let lo = (min_d as f64).log10().floor();
                let hi = (max_d.max(1) as f64).log10().ceil().max(lo + 1.0);
                let n = 40;
                let step = (hi - lo) / n as f64;
                let mut counts = vec![0i64; n];
                for s in &spans {
                    let x = ((s.duration.max(1) as f64).log10() - lo) / step;
                    counts[(x as usize).min(n - 1)] += 1;
                }
                distribution = counts
                    .into_iter()
                    .enumerate()
                    .map(|(i, c)| (10f64.powf(lo + step * (i + 1) as f64) as i64, c))
                    .collect();
            }

            // Calls from this service's spans into other services' entry spans.
            let mut dsql = String::from(
                "SELECT child.service, COUNT(*), SUM(child.error), AVG(child.duration)
                 FROM spans child JOIN spans parent ON parent.trace_id = child.trace_id AND parent.span_id = child.parent_id
                 WHERE parent.service = ? AND child.service != parent.service AND child.start >= ? AND child.start <= ?",
            );
            let mut dparams: Vec<SqlValue> =
                vec![name.clone().into(), (r.from * 1_000_000).into(), (r.to * 1_000_000 + 999_999).into()];
            if let Some(env) = r.env.as_ref().filter(|e| !e.is_empty() && *e != "*") {
                dsql.push_str(" AND child.env = ?");
                dparams.push(env.clone().into());
            }
            dsql.push_str(" GROUP BY child.service ORDER BY 2 DESC");
            let downstream = c
                .prepare(&dsql)?
                .query_map(rusqlite::params_from_iter(dparams), |row| {
                    Ok(Dependency {
                        service: row.get(0)?,
                        calls: row.get(1)?,
                        errors: row.get(2)?,
                        avg: row.get::<_, f64>(3)? as i64,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;

            let mut operations: Vec<(String, i64)> = ops.clone().into_iter().collect();
            operations.sort_by_key(|o| std::cmp::Reverse(o.1));
            let mut resources: Vec<ResourceRow> = resources
                .into_iter()
                .map(|((resource, operation), acc)| ResourceRow { resource, operation, stats: acc.stats() })
                .collect();
            resources.sort_by_key(|r| std::cmp::Reverse(r.stats.hits));
            Ok(ServiceDetail {
                name,
                typ: most_common(types),
                operations: operations.into_iter().map(|(o, _)| o).collect(),
                summary: total.stats(),
                interval_ms: interval,
                timeseries: series
                    .into_iter()
                    .map(|(t, acc)| {
                        let s = acc.stats();
                        Point { t, hits: s.hits, errors: s.errors, p50: s.p50, p95: s.p95, p99: s.p99, max: s.max }
                    })
                    .collect(),
                resources,
                distribution,
                downstream,
            })
        })
        .await?;
    Ok(Json(out))
}

pub async fn envs(State(st): State<AppState>) -> ApiResult<Vec<String>> {
    let envs = st
        .db
        .read(|c| {
            Ok(
                c.prepare("SELECT DISTINCT env FROM spans WHERE env IS NOT NULL ORDER BY env")?
                    .query_map([], |r| r.get(0))?
                    .collect::<Result<Vec<String>, _>>()?,
            )
        })
        .await?;
    Ok(Json(envs))
}
