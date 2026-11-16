use super::logs::{Bucket, fill};
use super::{ApiError, ApiResult, Scope, compile, interval_ms};
use crate::AppState;
use crate::query::sql::Dataset;
use axum::Json;
use axum::extract::{Path, State};
use rusqlite::types::Value as SqlValue;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Serialize)]
pub struct Span {
    pub id: i64,
    pub trace_id: String,
    pub span_id: String,
    pub parent_id: String,
    pub service: String,
    pub name: String,
    pub resource: String,
    #[serde(rename = "type")]
    pub typ: Option<String>,
    pub env: Option<String>,
    pub version: Option<String>,
    /// Epoch nanoseconds.
    pub start: i64,
    /// Nanoseconds.
    pub duration: i64,
    pub error: bool,
    pub top_level: bool,
    pub meta: Value,
    pub metrics: Value,
}

const COLUMNS: &str = "id, trace_id, span_id, parent_id, service, name, resource, type, env, version, start, duration, error, top_level, meta, metrics";

pub fn row(r: &rusqlite::Row) -> rusqlite::Result<Span> {
    let meta: String = r.get(14)?;
    let metrics: String = r.get(15)?;
    Ok(Span {
        id: r.get(0)?,
        trace_id: r.get(1)?,
        span_id: r.get(2)?,
        parent_id: r.get(3)?,
        service: r.get(4)?,
        name: r.get(5)?,
        resource: r.get(6)?,
        typ: r.get(7)?,
        env: r.get(8)?,
        version: r.get(9)?,
        start: r.get(10)?,
        duration: r.get(11)?,
        error: r.get(12)?,
        top_level: r.get(13)?,
        meta: serde_json::from_str(&meta).unwrap_or(Value::Null),
        metrics: serde_json::from_str(&metrics).unwrap_or(Value::Null),
    })
}

fn ns(ms: i64) -> i64 {
    ms * 1_000_000
}

#[derive(Deserialize)]
pub struct SearchReq {
    #[serde(flatten)]
    scope: Scope,
    limit: Option<i64>,
    /// `"<start>:<id>"` of the last span of the previous page.
    cursor: Option<String>,
    /// `"start"` (default) or `"duration"`.
    sort: Option<String>,
}

#[derive(Serialize)]
pub struct SearchResp {
    spans: Vec<Span>,
    next_cursor: Option<String>,
    total: Option<i64>,
}

pub async fn search(
    State(st): State<AppState>,
    Json(req): Json<SearchReq>,
) -> ApiResult<SearchResp> {
    let f = compile(&req.scope.query, Dataset::Spans)?;
    let limit = req.limit.unwrap_or(100).clamp(1, 1000);
    let by_duration = req.sort.as_deref() == Some("duration");
    let out = st
        .db
        .read(move |c| {
            let where_ = format!("start >= ? AND start <= ? AND {}", f.sql);
            let mut params: Vec<SqlValue> = vec![
                ns(req.scope.from).into(),
                (ns(req.scope.to) + 999_999).into(),
            ];
            params.extend(f.params.iter().cloned());
            let count_params = params.clone();
            let mut sql = format!("SELECT {COLUMNS} FROM spans WHERE {where_}");
            let mut offset = 0i64;
            if by_duration {
                offset = req
                    .cursor
                    .as_deref()
                    .and_then(|c| c.parse().ok())
                    .unwrap_or(0);
                sql.push_str(" ORDER BY duration DESC LIMIT ? OFFSET ?");
                params.extend([limit.into(), offset.into()]);
            } else {
                if let Some((t, id)) = req.cursor.as_deref().and_then(|c| c.split_once(':')) {
                    sql.push_str(" AND (start < ? OR (start = ? AND id < ?))");
                    let t: i64 = t.parse().unwrap_or(0);
                    params.extend([t.into(), t.into(), id.parse::<i64>().unwrap_or(0).into()]);
                }
                sql.push_str(" ORDER BY start DESC, id DESC LIMIT ?");
                params.push(limit.into());
            }
            let spans: Vec<Span> = c
                .prepare(&sql)?
                .query_map(rusqlite::params_from_iter(params), row)?
                .collect::<Result<_, _>>()?;
            let total = if req.cursor.is_none() {
                Some(c.query_row(
                    &format!("SELECT COUNT(*) FROM spans WHERE {where_}"),
                    rusqlite::params_from_iter(count_params),
                    |r| r.get(0),
                )?)
            } else {
                None
            };
            let full = spans.len() as i64 == limit;
            let next_cursor = if !full {
                None
            } else if by_duration {
                Some((offset + limit).to_string())
            } else {
                spans.last().map(|s| format!("{}:{}", s.start, s.id))
            };
            Ok(SearchResp {
                spans,
                next_cursor,
                total,
            })
        })
        .await?;
    Ok(Json(out))
}

#[derive(Deserialize)]
pub struct HistogramReq {
    #[serde(flatten)]
    scope: Scope,
    buckets: Option<i64>,
}

#[derive(Serialize)]
pub struct HistogramResp {
    interval_ms: i64,
    buckets: Vec<Bucket>,
}

pub async fn histogram(
    State(st): State<AppState>,
    Json(req): Json<HistogramReq>,
) -> ApiResult<HistogramResp> {
    let f = compile(&req.scope.query, Dataset::Spans)?;
    let (from, to) = (req.scope.from, req.scope.to);
    let interval = interval_ms(from, to, req.buckets.unwrap_or(80));
    let rows = st
        .db
        .read(move |c| {
            let sql = format!(
                "SELECT ((start / 1000000) / ?) * ? AS b, CASE WHEN error THEN 'error' ELSE 'ok' END, COUNT(*)
                 FROM spans WHERE start >= ? AND start <= ? AND {} GROUP BY 1, 2",
                f.sql
            );
            let mut params: Vec<SqlValue> = vec![interval.into(), interval.into(), ns(from).into(), (ns(to) + 999_999).into()];
            params.extend(f.params);
            let rows: Vec<(i64, String, i64)> = c
                .prepare(&sql)?
                .query_map(rusqlite::params_from_iter(params), |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                .collect::<Result<_, _>>()?;
            Ok(rows)
        })
        .await?;
    Ok(Json(HistogramResp {
        interval_ms: interval,
        buckets: fill(from, to, interval, rows),
    }))
}

#[derive(Serialize)]
pub struct TraceResp {
    trace_id: String,
    spans: Vec<Span>,
}

pub async fn trace(
    State(st): State<AppState>,
    Path(trace_id): Path<String>,
) -> ApiResult<TraceResp> {
    let id = trace_id.clone();
    let spans = st
        .db
        .read(move |c| {
            Ok(c.prepare(&format!(
                "SELECT {COLUMNS} FROM spans WHERE trace_id = ? ORDER BY start, id"
            ))?
            .query_map([&id], row)?
            .collect::<Result<Vec<_>, _>>()?)
        })
        .await?;
    if spans.is_empty() {
        return Err(ApiError::NotFound);
    }
    Ok(Json(TraceResp { trace_id, spans }))
}
