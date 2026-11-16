use super::{ApiError, ApiResult, Scope, compile, interval_ms};
use crate::AppState;
use crate::query::sql::Dataset;
use axum::Json;
use axum::extract::{Path, State};
use rusqlite::types::Value as SqlValue;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Serialize)]
pub struct Log {
    pub id: i64,
    pub timestamp: i64,
    pub status: String,
    pub service: Option<String>,
    pub host: Option<String>,
    pub source: Option<String>,
    pub message: String,
    pub tags: Vec<String>,
    pub attributes: Value,
    pub trace_id: Option<String>,
    pub span_id: Option<String>,
}

const COLUMNS: &str =
    "id, ts, status, service, host, source, message, tags, attributes, trace_id, span_id";

fn row(r: &rusqlite::Row) -> rusqlite::Result<Log> {
    let tags: String = r.get(7)?;
    let attrs: String = r.get(8)?;
    Ok(Log {
        id: r.get(0)?,
        timestamp: r.get(1)?,
        status: r.get(2)?,
        service: r.get(3)?,
        host: r.get(4)?,
        source: r.get(5)?,
        message: r.get(6)?,
        tags: serde_json::from_str(&tags).unwrap_or_default(),
        attributes: serde_json::from_str(&attrs).unwrap_or(Value::Null),
        trace_id: r.get(9)?,
        span_id: r.get(10)?,
    })
}

#[derive(Deserialize)]
pub struct SearchReq {
    #[serde(flatten)]
    scope: Scope,
    limit: Option<i64>,
    /// `"<ts>:<id>"` of the last log of the previous page.
    cursor: Option<String>,
    /// Live tail: only logs inserted after this id.
    after_id: Option<i64>,
}

#[derive(Serialize)]
pub struct SearchResp {
    logs: Vec<Log>,
    next_cursor: Option<String>,
    total: Option<i64>,
}

pub async fn search(
    State(st): State<AppState>,
    Json(req): Json<SearchReq>,
) -> ApiResult<SearchResp> {
    let f = compile(&req.scope.query, Dataset::Logs)?;
    let limit = req.limit.unwrap_or(100).clamp(1, 1000);
    let out = st
        .db
        .read(move |c| {
            let mut sql = format!(
                "SELECT {COLUMNS} FROM logs WHERE ts >= ? AND ts <= ? AND {}",
                f.sql
            );
            let mut params: Vec<SqlValue> = vec![req.scope.from.into(), req.scope.to.into()];
            params.extend(f.params.iter().cloned());
            let count_sql = format!(
                "SELECT COUNT(*) FROM logs WHERE ts >= ? AND ts <= ? AND {}",
                f.sql
            );
            let count_params = params.clone();
            if let Some((ts, id)) = req.cursor.as_deref().and_then(|c| c.split_once(':')) {
                sql.push_str(" AND (ts < ? OR (ts = ? AND id < ?))");
                let ts: i64 = ts.parse().unwrap_or(0);
                params.extend([ts.into(), ts.into(), id.parse::<i64>().unwrap_or(0).into()]);
            }
            if let Some(after) = req.after_id {
                sql.push_str(" AND id > ?");
                params.push(after.into());
            }
            sql.push_str(" ORDER BY ts DESC, id DESC LIMIT ?");
            params.push(limit.into());
            let logs: Vec<Log> = c
                .prepare(&sql)?
                .query_map(rusqlite::params_from_iter(params), row)?
                .collect::<Result<_, _>>()?;
            let total = if req.cursor.is_none() && req.after_id.is_none() {
                Some(
                    c.query_row(&count_sql, rusqlite::params_from_iter(count_params), |r| {
                        r.get(0)
                    })?,
                )
            } else {
                None
            };
            let next_cursor = (logs.len() as i64 == limit)
                .then(|| logs.last().map(|l| format!("{}:{}", l.timestamp, l.id)))
                .flatten();
            Ok(SearchResp {
                logs,
                next_cursor,
                total,
            })
        })
        .await?;
    Ok(Json(out))
}

pub async fn get(State(st): State<AppState>, Path(id): Path<i64>) -> ApiResult<Log> {
    let log = st
        .db
        .read(move |c| {
            use rusqlite::OptionalExtension;
            Ok(c.query_row(
                &format!("SELECT {COLUMNS} FROM logs WHERE id = ?"),
                [id],
                row,
            )
            .optional()?)
        })
        .await?;
    log.map(Json).ok_or(ApiError::NotFound)
}

#[derive(Deserialize)]
pub struct HistogramReq {
    #[serde(flatten)]
    scope: Scope,
    buckets: Option<i64>,
}

#[derive(Serialize)]
pub struct Bucket {
    t: i64,
    counts: BTreeMap<String, i64>,
}

#[derive(Serialize)]
pub struct HistogramResp {
    interval_ms: i64,
    buckets: Vec<Bucket>,
}

/// Fills every bucket of the range, including the empty ones.
pub fn fill(from: i64, to: i64, interval: i64, rows: Vec<(i64, String, i64)>) -> Vec<Bucket> {
    let start = from - from.rem_euclid(interval);
    let mut map: BTreeMap<i64, BTreeMap<String, i64>> = BTreeMap::new();
    let mut t = start;
    while t <= to {
        map.insert(t, BTreeMap::new());
        t += interval;
    }
    for (b, k, n) in rows {
        *map.entry(b).or_default().entry(k).or_default() += n;
    }
    map.into_iter()
        .map(|(t, counts)| Bucket { t, counts })
        .collect()
}

pub async fn histogram(
    State(st): State<AppState>,
    Json(req): Json<HistogramReq>,
) -> ApiResult<HistogramResp> {
    let f = compile(&req.scope.query, Dataset::Logs)?;
    let (from, to) = (req.scope.from, req.scope.to);
    let interval = interval_ms(from, to, req.buckets.unwrap_or(80));
    let rows = st
        .db
        .read(move |c| {
            let sql = format!(
                "SELECT (ts / ?) * ? AS b, status, COUNT(*) FROM logs WHERE ts >= ? AND ts <= ? AND {} GROUP BY b, status",
                f.sql
            );
            let mut params: Vec<SqlValue> = vec![interval.into(), interval.into(), from.into(), to.into()];
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
