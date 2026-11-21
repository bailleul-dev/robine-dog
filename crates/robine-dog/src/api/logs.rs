use super::{ApiError, ApiResult, Scope, compile, interval_ms, time_clause};
use crate::AppState;
use crate::query::sql::Dataset;
use crate::store::greptime::{Row, i64_at, json_at, parse_tags_column, str_at};
use axum::Json;
use axum::extract::{Path, State};
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

fn row(r: &Row) -> Log {
    Log {
        id: i64_at(r, 0),
        timestamp: i64_at(r, 1),
        status: str_at(r, 2).unwrap_or_default(),
        service: str_at(r, 3),
        host: str_at(r, 4),
        source: str_at(r, 5),
        message: str_at(r, 6).unwrap_or_default(),
        tags: parse_tags_column(&str_at(r, 7).unwrap_or_default()),
        attributes: json_at(r, 8),
        trace_id: str_at(r, 9),
        span_id: str_at(r, 10),
    }
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

/// Logs, or a historical view's (`index:<view>`).
pub async fn search(st: State<AppState>, req: Json<SearchReq>) -> ApiResult<SearchResp> {
    match crate::logconfig::archives::view_table(&req.scope.query) {
        Some(t) => search_table(st, req, &t).await,
        None => search_table(st, req, "logs").await,
    }
}

/// Events have the columns of logs, in their own table.
pub async fn search_table(
    State(st): State<AppState>,
    Json(req): Json<SearchReq>,
    table: &str,
) -> ApiResult<SearchResp> {
    let filter = compile(&req.scope.query, Dataset::Logs)?;
    let limit = req.limit.unwrap_or(100).clamp(1, 1000);
    let where_ = format!(
        "{} AND {filter}",
        time_clause(Dataset::Logs, req.scope.from, req.scope.to)
    );
    let mut sql = format!("SELECT {COLUMNS} FROM {table} WHERE {where_}");
    if let Some((ts, id)) = req.cursor.as_deref().and_then(|c| c.split_once(':')) {
        let ts: i64 = ts.parse().unwrap_or(0);
        let id: i64 = id.parse().unwrap_or(0);
        sql.push_str(&format!(" AND (ts < {ts} OR (ts = {ts} AND id < {id}))"));
    }
    if let Some(after) = req.after_id {
        sql.push_str(&format!(" AND id > {after}"));
    }
    sql.push_str(&format!(" ORDER BY ts DESC, id DESC LIMIT {limit}"));
    let want_total = req.cursor.is_none() && req.after_id.is_none();
    let count_sql = format!("SELECT count(*) FROM {table} WHERE {where_}");
    let (rows, total) = tokio::try_join!(st.store.gt.query(&sql), async {
        if want_total {
            Ok(Some(st.store.gt.query(&count_sql).await?))
        } else {
            Ok(None)
        }
    })?;
    let logs: Vec<Log> = rows.iter().map(row).collect();
    let next_cursor = (logs.len() as i64 == limit)
        .then(|| logs.last().map(|l| format!("{}:{}", l.timestamp, l.id)))
        .flatten();
    Ok(Json(SearchResp {
        logs,
        next_cursor,
        total: total.and_then(|r| r.first().map(|r| i64_at(r, 0))),
    }))
}

pub async fn get(State(st): State<AppState>, Path(id): Path<i64>) -> ApiResult<Log> {
    let rows = st
        .store
        .gt
        .query(&format!(
            "SELECT {COLUMNS} FROM logs WHERE id = {id} LIMIT 1"
        ))
        .await?;
    rows.first().map(row).map(Json).ok_or(ApiError::NotFound)
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

pub async fn histogram(st: State<AppState>, req: Json<HistogramReq>) -> ApiResult<HistogramResp> {
    match crate::logconfig::archives::view_table(&req.scope.query) {
        Some(t) => histogram_table(st, req, &t).await,
        None => histogram_table(st, req, "logs").await,
    }
}

pub async fn histogram_table(
    State(st): State<AppState>,
    Json(req): Json<HistogramReq>,
    table: &str,
) -> ApiResult<HistogramResp> {
    let filter = compile(&req.scope.query, Dataset::Logs)?;
    let (from, to) = (req.scope.from, req.scope.to);
    let interval = interval_ms(from, to, req.buckets.unwrap_or(80));
    let rows = st
        .store
        .gt
        .query(&format!(
            "SELECT CAST(date_bin(INTERVAL '{interval} milliseconds', ts) AS BIGINT) AS b, status, count(*)
             FROM {table} WHERE {} AND {filter} GROUP BY b, status",
            time_clause(Dataset::Logs, from, to)
        ))
        .await?;
    let rows = rows
        .iter()
        .map(|r| (i64_at(r, 0), str_at(r, 1).unwrap_or_default(), i64_at(r, 2)))
        .collect();
    Ok(Json(HistogramResp {
        interval_ms: interval,
        buckets: fill(from, to, interval, rows),
    }))
}
