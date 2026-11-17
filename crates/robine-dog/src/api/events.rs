//! Events explorer, and Datadog's `POST /api/v1/events`.

use super::logs::{HistogramReq, HistogramResp, SearchReq, SearchResp};
use super::{ApiError, ApiResult};
use crate::AppState;
use crate::intake::logs::{api_key_ok, now_ms};
use axum::Json;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};
use std::collections::HashMap;

pub async fn search(st: State<AppState>, req: Json<SearchReq>) -> ApiResult<SearchResp> {
    super::logs::search_table(st, req, "events").await
}

pub async fn histogram(st: State<AppState>, req: Json<HistogramReq>) -> ApiResult<HistogramResp> {
    super::logs::histogram_table(st, req, "events").await
}

async fn store(st: &AppState, v: &Value) -> Result<Value, ApiError> {
    let row = crate::events::from_json(v, None, now_ms())
        .ok_or_else(|| ApiError::BadQuery("an event needs a title".into()))?;
    let event = json!({
        "title": row.attributes["evt"]["title"],
        "date_happened": row.ts / 1000,
        "tags": row.tags,
        "host": row.host,
    });
    crate::events::emit(st, vec![row]).await;
    Ok(json!({"status": "ok", "event": event}))
}

/// From the UI.
pub async fn create(State(st): State<AppState>, Json(v): Json<Value>) -> ApiResult<Value> {
    Ok(Json(store(&st, &v).await?))
}

/// `POST /api/v1/events`, with an API key.
pub async fn intake(
    State(st): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
    Json(v): Json<Value>,
) -> Response {
    if !api_key_ok(&st, &headers, &q) {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({"errors": ["Forbidden"]})),
        )
            .into_response();
    }
    match store(&st, &v).await {
        Ok(v) => (StatusCode::ACCEPTED, Json(v)).into_response(),
        Err(e) => e.into_response(),
    }
}
