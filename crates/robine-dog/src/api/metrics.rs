//! Metrics Summary (metadata, tags) and Metrics Explorer (queries).

use super::{ApiError, ApiResult};
use crate::AppState;
use crate::metrics::query::{self, MetricQuery};
use crate::metrics::{FLUSH_INTERVAL_MS, Kind};
use crate::store::greptime::{f64_at, i64_at, lit, parse_tags_column, str_at};
use crate::store::pg::MetricMeta;
use axum::Json;
use axum::extract::{Path, Query, State};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};

pub async fn list(State(st): State<AppState>) -> ApiResult<Vec<MetricMeta>> {
    Ok(Json(st.store.pg.metrics().await?))
}

#[derive(Deserialize)]
pub struct Window {
    from: i64,
    to: i64,
}

#[derive(Serialize)]
pub struct MetricDetail {
    #[serde(flatten)]
    meta: MetricMeta,
    hosts: Vec<String>,
    /// Tag key → values seen over the window.
    tags: BTreeMap<String, BTreeSet<String>>,
    series: usize,
}

pub async fn detail(
    State(st): State<AppState>,
    Path(name): Path<String>,
    Query(w): Query<Window>,
) -> ApiResult<MetricDetail> {
    let meta = st.store.pg.metric(&name).await?.ok_or(ApiError::NotFound)?;
    let rows = st
        .store
        .gt
        .query(&format!(
            "SELECT DISTINCT host, tags FROM metric_points WHERE metric = {} AND ts >= {} AND ts <= {} LIMIT 5000",
            lit(&name),
            w.from,
            w.to
        ))
        .await?;
    let mut hosts = BTreeSet::new();
    let mut tags: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for r in &rows {
        if let Some(h) = str_at(r, 0) {
            hosts.insert(h);
        }
        for t in parse_tags_column(&str_at(r, 1).unwrap_or_default()) {
            let (k, v) = t.split_once(':').unwrap_or((t.as_str(), ""));
            tags.entry(k.to_string()).or_default().insert(v.to_string());
        }
    }
    Ok(Json(MetricDetail {
        meta,
        hosts: hosts.into_iter().collect(),
        tags,
        series: rows.len(),
    }))
}

#[derive(Deserialize)]
pub struct MetaUpdate {
    unit: Option<String>,
    description: Option<String>,
}

pub async fn update(
    State(st): State<AppState>,
    Path(name): Path<String>,
    Json(u): Json<MetaUpdate>,
) -> ApiResult<MetricMeta> {
    let clean = |s: Option<String>| s.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    let (unit, description) = (clean(u.unit), clean(u.description));
    if !st
        .store
        .pg
        .update_metric(&name, unit.as_deref(), description.as_deref())
        .await?
    {
        return Err(ApiError::NotFound);
    }
    st.store
        .pg
        .metric(&name)
        .await?
        .map(Json)
        .ok_or(ApiError::NotFound)
}

#[derive(Deserialize)]
pub struct QueryReq {
    queries: Vec<String>,
    from: i64,
    to: i64,
    /// Series kept per query (the largest ones): 50 for a graph, more for
    /// the infrastructure lists, which have a row per group.
    max_series: Option<usize>,
}

/// The hosts Agents described: their tags, aliases, Agent version.
pub async fn hosts(State(st): State<AppState>) -> ApiResult<Vec<crate::hosts::Host>> {
    Ok(Json(st.hosts.list()))
}

#[derive(Serialize)]
pub struct Series {
    query_index: usize,
    /// `by` tag → value; empty without `by`.
    group: BTreeMap<String, String>,
    /// `[bucket start ms, value]`, only buckets with data.
    points: Vec<(i64, f64)>,
}

#[derive(Serialize)]
pub struct QueryResult {
    query: String,
    metric: Option<String>,
    #[serde(rename = "type")]
    typ: Option<String>,
    unit: Option<String>,
    error: Option<String>,
}

#[derive(Serialize)]
pub struct QueryResp {
    interval_ms: i64,
    results: Vec<QueryResult>,
    series: Vec<Series>,
}

/// A `by` group's values and its points.
type Group = (Vec<String>, Vec<(i64, f64)>);

/// At most this many series per query, the largest ones.
const MAX_SERIES: usize = 50;
const MAX_SERIES_LIMIT: usize = 5000;

pub async fn query(State(st): State<AppState>, Json(req): Json<QueryReq>) -> ApiResult<QueryResp> {
    if req.queries.len() > 10 {
        return Err(ApiError::BadQuery("at most 10 queries".into()));
    }
    let (from, to) = (req.from, req.to);
    let max_series = req.max_series.unwrap_or(MAX_SERIES).min(MAX_SERIES_LIMIT);
    let metas: HashMap<String, MetricMeta> = st
        .store
        .pg
        .metrics()
        .await?
        .into_iter()
        .map(|m| (m.name.clone(), m))
        .collect();
    let parsed: Vec<Result<MetricQuery, String>> =
        req.queries.iter().map(|q| query::parse(q)).collect();
    // One interval for every query, so they share an x axis.
    let interval = parsed
        .iter()
        .filter_map(|q| q.as_ref().ok())
        .map(|q| query::interval_ms(q, from, to))
        .max()
        .unwrap_or(FLUSH_INTERVAL_MS);
    let gt = &st.store.gt;
    let runs = futures::future::join_all(parsed.iter().map(|q| async {
        let q = q.as_ref().map_err(Clone::clone)?;
        let kind = metas
            .get(&q.metric)
            .map(|m| Kind::parse(&m.typ))
            .unwrap_or(Kind::Gauge);
        gt.query(&query::build_sql(q, kind, from, to, interval))
            .await
            .map_err(|e| format!("{e:#}"))
    }))
    .await;

    let mut results = Vec::new();
    let mut series = Vec::new();
    for (i, (raw, (q, run))) in req.queries.iter().zip(parsed.iter().zip(runs)).enumerate() {
        let meta = q.as_ref().ok().and_then(|q| metas.get(&q.metric));
        let error = match (&run, q) {
            (Err(e), _) => Some(e.clone()),
            (Ok(_), Ok(q)) if meta.is_none() => Some(format!("no data for metric {}", q.metric)),
            _ => None,
        };
        results.push(QueryResult {
            query: raw.clone(),
            metric: q.as_ref().ok().map(|q| q.metric.clone()),
            typ: meta.map(|m| m.typ.clone()),
            unit: meta.and_then(|m| m.unit.clone()),
            error,
        });
        let (Ok(rows), Ok(q)) = (run, q) else {
            continue;
        };
        let n = q.by.len();
        let mut groups: BTreeMap<Vec<String>, Vec<(i64, f64)>> = BTreeMap::new();
        for r in &rows {
            let key: Vec<String> = (0..n)
                .map(|g| str_at(r, 1 + g).unwrap_or_else(|| "N/A".into()))
                .collect();
            if let Some(v) = f64_at(r, 1 + n) {
                groups.entry(key).or_default().push((i64_at(r, 0), v));
            }
        }
        let mut ranked: Vec<Group> = groups.into_iter().collect();
        ranked.sort_by(|a, b| {
            let sum = |p: &Vec<(i64, f64)>| p.iter().map(|x| x.1.abs()).sum::<f64>();
            sum(&b.1).total_cmp(&sum(&a.1))
        });
        for (key, points) in ranked.into_iter().take(max_series) {
            series.push(Series {
                query_index: i,
                group: q.by.iter().cloned().zip(key).collect(),
                points,
            });
        }
    }
    Ok(Json(QueryResp {
        interval_ms: interval,
        results,
        series,
    }))
}
