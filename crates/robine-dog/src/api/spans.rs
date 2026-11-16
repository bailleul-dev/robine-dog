use super::logs::{Bucket, fill};
use super::{ApiError, ApiResult, Scope, compile, interval_ms, time_clause};
use crate::AppState;
use crate::query::sql::Dataset;
use crate::store::greptime::{Row, bool_at, i64_at, json_at, lit, str_at};
use axum::Json;
use axum::extract::{Path, State};
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

pub const COLUMNS: &str = "id, trace_id, span_id, parent_id, service, name, resource, type, env, version, CAST(ts AS BIGINT) AS start_ns, duration, error, top_level, meta, metrics";

pub fn row(r: &Row) -> Span {
    Span {
        id: i64_at(r, 0),
        trace_id: str_at(r, 1).unwrap_or_default(),
        span_id: str_at(r, 2).unwrap_or_default(),
        parent_id: str_at(r, 3).unwrap_or_default(),
        service: str_at(r, 4).unwrap_or_default(),
        name: str_at(r, 5).unwrap_or_default(),
        resource: str_at(r, 6).unwrap_or_default(),
        typ: str_at(r, 7),
        env: str_at(r, 8),
        version: str_at(r, 9),
        start: i64_at(r, 10),
        duration: i64_at(r, 11),
        error: bool_at(r, 12),
        top_level: bool_at(r, 13),
        meta: json_at(r, 14),
        metrics: json_at(r, 15),
    }
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
    let filter = compile(&req.scope.query, Dataset::Spans)?;
    let limit = req.limit.unwrap_or(100).clamp(1, 1000);
    let by_duration = req.sort.as_deref() == Some("duration");
    let where_ = format!(
        "{} AND {filter}",
        time_clause(Dataset::Spans, req.scope.from, req.scope.to)
    );
    let mut sql = format!("SELECT {COLUMNS} FROM spans WHERE {where_}");
    let mut offset = 0i64;
    if by_duration {
        offset = req
            .cursor
            .as_deref()
            .and_then(|c| c.parse().ok())
            .unwrap_or(0)
            .max(0);
        sql.push_str(&format!(
            " ORDER BY duration DESC LIMIT {limit} OFFSET {offset}"
        ));
    } else {
        if let Some((t, id)) = req.cursor.as_deref().and_then(|c| c.split_once(':')) {
            let t: i64 = t.parse().unwrap_or(0);
            let id: i64 = id.parse().unwrap_or(0);
            sql.push_str(&format!(" AND (ts < {t} OR (ts = {t} AND id < {id}))"));
        }
        sql.push_str(&format!(" ORDER BY ts DESC, id DESC LIMIT {limit}"));
    }
    let count_sql = format!("SELECT count(*) FROM spans WHERE {where_}");
    let want_total = req.cursor.is_none();
    let (rows, total) = tokio::try_join!(st.store.gt.query(&sql), async {
        if want_total {
            Ok(Some(st.store.gt.query(&count_sql).await?))
        } else {
            Ok(None)
        }
    })?;
    let spans: Vec<Span> = rows.iter().map(row).collect();
    let full = spans.len() as i64 == limit;
    let next_cursor = if !full {
        None
    } else if by_duration {
        Some((offset + limit).to_string())
    } else {
        spans.last().map(|s| format!("{}:{}", s.start, s.id))
    };
    Ok(Json(SearchResp {
        spans,
        next_cursor,
        total: total.and_then(|r| r.first().map(|r| i64_at(r, 0))),
    }))
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
    let filter = compile(&req.scope.query, Dataset::Spans)?;
    let (from, to) = (req.scope.from, req.scope.to);
    let interval = interval_ms(from, to, req.buckets.unwrap_or(80));
    let rows = st
        .store
        .gt
        .query(&format!(
            "SELECT CAST(date_bin(INTERVAL '{interval} milliseconds', ts) AS BIGINT) AS b,
                    CASE WHEN error THEN 'error' ELSE 'ok' END AS s, count(*)
             FROM spans WHERE {} AND {filter} GROUP BY b, s",
            time_clause(Dataset::Spans, from, to)
        ))
        .await?;
    let rows = rows
        .iter()
        .map(|r| {
            (
                i64_at(r, 0) / 1_000_000,
                str_at(r, 1).unwrap_or_default(),
                i64_at(r, 2),
            )
        })
        .collect();
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
    let rows = st
        .store
        .gt
        .query(&format!(
            "SELECT {COLUMNS} FROM spans WHERE trace_id = {} ORDER BY ts, id",
            lit(&trace_id)
        ))
        .await?;
    if rows.is_empty() {
        return Err(ApiError::NotFound);
    }
    Ok(Json(TraceResp {
        trace_id,
        spans: rows.iter().map(row).collect(),
    }))
}
