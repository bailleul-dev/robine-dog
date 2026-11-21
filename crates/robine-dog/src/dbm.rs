//! Database Monitoring: what the Agent's Postgres and MySQL checks send
//! with `dbm: true`: normalized query metrics (`/api/v2/dbmmetrics`), and
//! samples, explain plans and active sessions (`/api/v2/databasequery`).

use crate::AppState;
use crate::api::{ApiError, ApiResult};
use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::Read;
use std::sync::{LazyLock, Mutex};

/// Each host's latest active sessions, with when they were received.
type Sessions = HashMap<String, (i64, String, Vec<Value>)>;
static ACTIVITY: LazyLock<Mutex<Sessions>> = LazyLock::new(|| Mutex::new(HashMap::new()));

fn decode(headers: &HeaderMap, body: &[u8]) -> Result<Vec<Value>, String> {
    let enc = headers
        .get("content-encoding")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let mut raw = Vec::new();
    if enc.contains("gzip") || body.starts_with(&[0x1f, 0x8b]) {
        flate2::read::MultiGzDecoder::new(body)
            .read_to_end(&mut raw)
            .map_err(|e| format!("gzip: {e}"))?;
    } else if enc.contains("deflate") {
        flate2::read::ZlibDecoder::new(body)
            .read_to_end(&mut raw)
            .map_err(|e| format!("deflate: {e}"))?;
    } else if enc.contains("zstd") {
        raw = zstd::decode_all(body).map_err(|e| format!("zstd: {e}"))?;
    } else {
        raw = body.to_vec();
    }
    match serde_json::from_slice(&raw).map_err(|e| format!("JSON: {e}"))? {
        Value::Array(a) => Ok(a),
        v => Ok(vec![v]),
    }
}

/// A query's metrics over one collection interval (the Agent sends deltas).
#[derive(Debug, Clone, PartialEq)]
pub struct QueryRow {
    pub ts: i64,
    pub host: String,
    pub dbms: String,
    pub db: Option<String>,
    pub user: Option<String>,
    pub signature: String,
    pub query: String,
    pub calls: f64,
    pub total_time_ms: f64,
    pub rows: f64,
}

pub fn parse_metrics(p: &Value, now: i64) -> Vec<QueryRow> {
    let host = p["host"].as_str().unwrap_or("unknown").to_string();
    let ts = p["timestamp"].as_f64().map(|t| t as i64).unwrap_or(now);
    let f = |r: &Value, k: &str| r[k].as_f64().unwrap_or(0.0);
    let s = |r: &Value, k: &str| r[k].as_str().filter(|x| !x.is_empty()).map(String::from);
    let mut out = Vec::new();
    for r in p["postgres_rows"].as_array().into_iter().flatten() {
        let Some(sig) = s(r, "query_signature") else {
            continue;
        };
        out.push(QueryRow {
            ts,
            host: host.clone(),
            dbms: "postgres".into(),
            db: s(r, "datname"),
            user: s(r, "rolname"),
            query: s(r, "query").unwrap_or_default(),
            calls: f(r, "calls"),
            total_time_ms: r["total_exec_time"]
                .as_f64()
                .or(r["total_time"].as_f64())
                .unwrap_or(0.0),
            rows: f(r, "rows"),
            signature: sig,
        });
    }
    for r in p["mysql_rows"].as_array().into_iter().flatten() {
        let Some(sig) = s(r, "query_signature") else {
            continue;
        };
        out.push(QueryRow {
            ts,
            host: host.clone(),
            dbms: "mysql".into(),
            db: s(r, "schema_name"),
            user: None,
            query: s(r, "digest_text").or(s(r, "query")).unwrap_or_default(),
            calls: f(r, "count_star"),
            // Picoseconds.
            total_time_ms: f(r, "sum_timer_wait") / 1e9,
            rows: f(r, "sum_rows_sent"),
            signature: sig,
        });
    }
    out
}

/// `POST /api/v2/dbmmetrics`.
pub async fn metrics_intake(
    State(st): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let payloads = match decode(&headers, &body) {
        Ok(p) => p,
        Err(e) => return (StatusCode::BAD_REQUEST, e).into_response(),
    };
    let now = crate::intake::logs::now_ms();
    let rows: Vec<QueryRow> = payloads
        .iter()
        .flat_map(|p| parse_metrics(p, now))
        .collect();
    let r = async {
        let c = st.store.pg.conn().await?;
        for q in &rows {
            if q.calls <= 0.0 {
                continue;
            }
            c.execute(
                "INSERT INTO dbm_query_metrics (ts, host, dbms, db, db_user, query_signature, query, calls, total_time_ms, rows)
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)",
                &[&q.ts, &q.host, &q.dbms, &q.db, &q.user, &q.signature, &q.query, &q.calls, &q.total_time_ms, &q.rows],
            )
            .await?;
        }
        anyhow::Ok(())
    }
    .await;
    match r {
        Ok(()) => StatusCode::ACCEPTED.into_response(),
        Err(e) => {
            tracing::warn!(error = %e, "storing DBM metrics");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// `POST /api/v2/databasequery`: samples and plans, or active sessions.
pub async fn samples_intake(
    State(st): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let events = match decode(&headers, &body) {
        Ok(p) => p,
        Err(e) => return (StatusCode::BAD_REQUEST, e).into_response(),
    };
    let now = crate::intake::logs::now_ms();
    let r = async {
        let c = st.store.pg.conn().await?;
        for e in &events {
            let host = e["host"].as_str().unwrap_or("unknown").to_string();
            let dbms = e["ddsource"].as_str().unwrap_or("postgres").to_string();
            let ts = e["timestamp"].as_f64().map(|t| t as i64).unwrap_or(now);
            let sessions = e["postgres_activity"].as_array().or(e["mysql_activity"].as_array());
            if e["dbm_type"].as_str() == Some("activity") || sessions.is_some() {
                if let Ok(mut a) = ACTIVITY.lock() {
                    a.insert(host, (ts, dbms, sessions.cloned().unwrap_or_default()));
                }
                continue;
            }
            let db = &e["db"];
            c.execute(
                "INSERT INTO dbm_samples (ts, host, dbms, db, query_signature, statement, duration_ns, plan, body)
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)",
                &[
                    &ts,
                    &host,
                    &dbms,
                    &db["instance"].as_str().or(e["postgres"]["datname"].as_str()),
                    &db["query_signature"].as_str(),
                    &db["statement"].as_str(),
                    &e["duration"].as_f64().map(|d| d as i64),
                    &db.get("plan").filter(|p| !p.is_null()),
                    e,
                ],
            )
            .await?;
        }
        anyhow::Ok(())
    }
    .await;
    match r {
        Ok(()) => StatusCode::ACCEPTED.into_response(),
        Err(e) => {
            tracing::warn!(error = %e, "storing DBM samples");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct Filter {
    pub from: i64,
    pub to: i64,
    pub host: Option<String>,
}

/// `GET /api/ui/databases`: top queries, hosts and active sessions.
pub async fn overview(State(st): State<AppState>, Query(f): Query<Filter>) -> ApiResult<Value> {
    let c = st.store.pg.conn().await?;
    let err = |e: tokio_postgres::Error| ApiError::Internal(e.into());
    let rows = c
        .query(
            "SELECT query_signature, max(query), max(dbms), max(db), sum(calls), sum(total_time_ms), sum(rows),
                count(DISTINCT host)
             FROM dbm_query_metrics WHERE ts >= $1 AND ts <= $2 AND ($3::TEXT IS NULL OR host = $3)
             GROUP BY query_signature ORDER BY 6 DESC LIMIT 200",
            &[&f.from, &f.to, &f.host],
        )
        .await
        .map_err(err)?;
    let hosts = c
        .query(
            "SELECT host, max(dbms), sum(calls), sum(total_time_ms) FROM dbm_query_metrics
             WHERE ts >= $1 AND ts <= $2 GROUP BY host ORDER BY host",
            &[&f.from, &f.to],
        )
        .await
        .map_err(err)?;
    let now = crate::intake::logs::now_ms();
    let activity: Vec<Value> = ACTIVITY
        .lock()
        .map(|a| {
            a.iter()
                .filter(|(h, (ts, ..))| {
                    now - ts < 120_000 && f.host.as_deref().is_none_or(|x| x == *h)
                })
                .flat_map(|(h, (_, dbms, s))| {
                    s.iter().map(move |x| {
                        let mut x = x.clone();
                        x["host"] = json!(h);
                        x["dbms"] = json!(dbms);
                        x
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(Json(json!({
        "queries": rows.iter().map(|r| {
            let calls: f64 = r.get(4);
            let total: f64 = r.get(5);
            json!({"signature": r.get::<_, String>(0), "query": r.get::<_, String>(1), "dbms": r.get::<_, String>(2),
                "db": r.get::<_, Option<String>>(3), "calls": calls, "total_time_ms": total,
                "avg_ms": if calls > 0.0 { total / calls } else { 0.0 }, "rows": r.get::<_, f64>(6),
                "hosts": r.get::<_, i64>(7)})
        }).collect::<Vec<_>>(),
        "hosts": hosts.iter().map(|r| json!({"host": r.get::<_, String>(0), "dbms": r.get::<_, String>(1),
            "calls": r.get::<_, f64>(2), "total_time_ms": r.get::<_, f64>(3)})).collect::<Vec<_>>(),
        "activity": activity,
    })))
}

/// `GET /api/ui/databases/queries/{signature}`: a query's metrics over time,
/// samples and plans.
pub async fn query(
    State(st): State<AppState>,
    Path(sig): Path<String>,
    Query(f): Query<Filter>,
) -> ApiResult<Value> {
    let c = st.store.pg.conn().await?;
    let err = |e: tokio_postgres::Error| ApiError::Internal(e.into());
    let step = ((f.to - f.from) / 60).max(60_000);
    let series = c
        .query(
            "SELECT (ts / $4) * $4, sum(calls), sum(total_time_ms), sum(rows) FROM dbm_query_metrics
             WHERE query_signature = $1 AND ts >= $2 AND ts <= $3 GROUP BY 1 ORDER BY 1",
            &[&sig, &f.from, &f.to, &step],
        )
        .await
        .map_err(err)?;
    let samples = c
        .query(
            "SELECT ts, host, db, statement, duration_ns, plan FROM dbm_samples
             WHERE query_signature = $1 AND ts >= $2 AND ts <= $3 ORDER BY ts DESC LIMIT 50",
            &[&sig, &f.from, &f.to],
        )
        .await
        .map_err(err)?;
    Ok(Json(json!({
        "interval_ms": step,
        "series": series.iter().map(|r| json!({"t": r.get::<_, i64>(0), "calls": r.get::<_, f64>(1),
            "total_time_ms": r.get::<_, f64>(2), "rows": r.get::<_, f64>(3)})).collect::<Vec<_>>(),
        "samples": samples.iter().map(|r| json!({"ts": r.get::<_, i64>(0), "host": r.get::<_, String>(1),
            "db": r.get::<_, Option<String>>(2), "statement": r.get::<_, Option<String>>(3),
            "duration_ns": r.get::<_, Option<i64>>(4), "plan": r.get::<_, Option<Value>>(5)})).collect::<Vec<_>>(),
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn postgres_and_mysql_rows() {
        let p = json!({"host": "db-1", "timestamp": 1700000000000.0, "postgres_rows": [
            {"query_signature": "abc", "query": "SELECT * FROM t WHERE id = ?", "calls": 10, "total_exec_time": 25.5,
             "rows": 10, "datname": "shop", "rolname": "app"},
            {"query": "no signature", "calls": 1}]});
        let r = parse_metrics(&p, 0);
        assert_eq!(r.len(), 1);
        assert_eq!(
            (r[0].ts, r[0].calls, r[0].total_time_ms, r[0].db.as_deref()),
            (1_700_000_000_000, 10.0, 25.5, Some("shop"))
        );
        let m = json!({"host": "db-2", "mysql_rows": [{"query_signature": "x", "digest_text": "SELECT ?", "count_star": 4,
            "sum_timer_wait": 2_000_000_000u64, "sum_rows_sent": 4, "schema_name": "app"}]});
        let r = parse_metrics(&m, 5);
        assert_eq!(
            (r[0].dbms.as_str(), r[0].total_time_ms, r[0].ts),
            ("mysql", 2.0, 5)
        );
    }
}
