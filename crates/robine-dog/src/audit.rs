//! Audit Trail: every change made through the UI or the API (a mutating
//! request that is not a search), with who made it and its outcome.

use crate::AppState;
use crate::api::{ApiError, ApiResult};
use crate::auth::Identity;
use axum::Json;
use axum::extract::{Query, Request, State};
use axum::http::Method;
use axum::middleware::Next;
use axum::response::Response;
use serde::Deserialize;
use serde_json::{Value, json};

/// Last path segments of POST routes that only read.
const READS: &[&str] = &[
    "search",
    "histogram",
    "values",
    "validate",
    "preview",
    "performance",
    "sessions",
    "frustrations",
    "retention",
    "aggregate",
    "patterns",
    "issues",
    "timeseries",
    "distribution",
    "funnel",
    "scalar",
    "grok-test",
    "query",
    "flamegraph",
    "test",
    "trigger",
    "mcp",
];

pub fn is_change(method: &Method, path: &str) -> bool {
    match *method {
        Method::PUT | Method::PATCH | Method::DELETE => true,
        Method::POST => {
            let last = path.trim_end_matches('/').rsplit('/').next().unwrap_or("");
            !READS.contains(&last)
        }
        _ => false,
    }
}

async fn record(st: &AppState, req: Request, next: Next, source: &'static str) -> Response {
    let method = req.method().clone();
    let path = req.uri().path().to_string();
    if !is_change(&method, &path) {
        return next.run(req).await;
    }
    let id = req.extensions().get::<Identity>().cloned();
    let resp = next.run(req).await;
    let (actor, name) = match id {
        Some(i) => (i.email.clone().unwrap_or(i.sub), i.name),
        None => ("api-key".to_string(), None),
    };
    let status = resp.status().as_u16() as i32;
    let st = st.clone();
    tokio::spawn(async move {
        let r = async {
            let c = st.store.pg.conn().await?;
            c.execute(
                "INSERT INTO audit_events (ts, actor, actor_name, source, method, path, status)
                 VALUES ($1, $2, $3, $4, $5, $6, $7)",
                &[
                    &crate::intake::logs::now_ms(),
                    &actor,
                    &name,
                    &source,
                    &method.as_str(),
                    &path,
                    &status,
                ],
            )
            .await?;
            anyhow::Ok(())
        }
        .await;
        if let Err(e) = r {
            tracing::warn!(error = %e, "writing the audit trail");
        }
    });
    resp
}

/// Middleware for the UI API (after authentication).
pub async fn ui(State(st): State<AppState>, req: Request, next: Next) -> Response {
    record(&st, req, next, "ui").await
}

/// Middleware for the public API.
pub async fn api(State(st): State<AppState>, req: Request, next: Next) -> Response {
    record(&st, req, next, "api").await
}

#[derive(Debug, Deserialize)]
pub struct Filter {
    #[serde(alias = "filter[from]")]
    pub from: Option<i64>,
    #[serde(alias = "filter[to]")]
    pub to: Option<i64>,
    #[serde(alias = "filter[query]")]
    pub query: Option<String>,
}

/// `GET /api/v2/audit/events` (and the UI's `/audit/events`): newest first.
pub async fn events(State(st): State<AppState>, Query(f): Query<Filter>) -> ApiResult<Value> {
    let now = crate::intake::logs::now_ms();
    let c = st.store.pg.conn().await?;
    let pattern = f
        .query
        .as_deref()
        .filter(|q| !q.trim().is_empty())
        .map(|q| format!("%{}%", q.trim()));
    let rows = c
        .query(
            "SELECT id, ts, actor, actor_name, source, method, path, status FROM audit_events
             WHERE ts >= $1 AND ts <= $2
               AND ($3::TEXT IS NULL OR actor ILIKE $3 OR path ILIKE $3 OR method ILIKE $3)
             ORDER BY ts DESC LIMIT 500",
            &[
                &f.from.unwrap_or(now - 7 * 86_400_000),
                &f.to.unwrap_or(now),
                &pattern,
            ],
        )
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    let data: Vec<Value> = rows
        .iter()
        .map(|r| {
            let method: String = r.get(5);
            let path: String = r.get(6);
            json!({"id": r.get::<_, i64>(0).to_string(), "type": "audit", "attributes": {
                "timestamp": r.get::<_, i64>(1), "service": "robine-dog",
                "message": format!("{method} {path}"),
                "attributes": {
                    "evt": {"name": "Request", "source": r.get::<_, String>(4)},
                    "http": {"method": method, "url_details": {"path": path}, "status_code": r.get::<_, i32>(7)},
                    "usr": {"id": r.get::<_, String>(2), "name": r.get::<_, Option<String>>(3)},
                },
            }})
        })
        .collect();
    Ok(Json(json!({"data": data})))
}

pub fn spawn_purge(st: AppState) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(3600));
        loop {
            tick.tick().await;
            // Kept 90 days, as Datadog's Audit Trail.
            let before = crate::intake::logs::now_ms() - 90 * 86_400_000;
            if let Ok(c) = st.store.pg.conn().await {
                let _ = c
                    .execute("DELETE FROM audit_events WHERE ts < $1", &[&before])
                    .await;
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changes_and_reads() {
        assert!(is_change(&Method::POST, "/api/v1/monitor"));
        assert!(is_change(&Method::DELETE, "/monitors/3"));
        assert!(!is_change(&Method::POST, "/logs/search"));
        assert!(!is_change(&Method::POST, "/api/v2/query/timeseries"));
        assert!(!is_change(&Method::GET, "/monitors"));
    }
}
