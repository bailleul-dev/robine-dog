//! Log- and span-based metrics, with Datadog's JSON:API shapes:
//! `/api/v2/logs/config/metrics` (`logs_metrics`) and
//! `/api/v2/apm/config/metrics` (`spans_metrics`).

use super::ApiError;
use crate::AppState;
use crate::metrics::generated::{self, LOGS_KIND, MetricDef, SPANS_KIND};
use axum::Json;
use axum::extract::{Path, State};
use serde_json::{Value, json};

#[derive(Clone, Copy)]
pub enum Source {
    Logs,
    Spans,
}

impl Source {
    fn kind(self) -> &'static str {
        match self {
            Source::Logs => LOGS_KIND,
            Source::Spans => SPANS_KIND,
        }
    }

    fn typ(self) -> &'static str {
        match self {
            Source::Logs => "logs_metrics",
            Source::Spans => "spans_metrics",
        }
    }
}

pub async fn stored(st: &AppState, src: Source) -> anyhow::Result<Vec<(String, MetricDef)>> {
    Ok(st
        .store
        .pg
        .docs(src.kind())
        .await?
        .into_iter()
        .filter_map(|d| Some((d.id, serde_json::from_value(d.body).ok()?)))
        .collect())
}

pub async fn reload(st: &AppState) -> anyhow::Result<()> {
    st.generated.load_logs(&stored(st, Source::Logs).await?);
    st.generated.load_spans(&stored(st, Source::Spans).await?);
    Ok(())
}

fn present(src: Source, id: &str, d: &MetricDef) -> Value {
    json!({"id": id, "type": src.typ(), "attributes": d})
}

async fn list(st: AppState, src: Source) -> Result<Json<Value>, ApiError> {
    let all = stored(&st, src).await?;
    Ok(Json(
        json!({"data": all.iter().map(|(id, d)| present(src, id, d)).collect::<Vec<_>>()}),
    ))
}

async fn get(st: AppState, src: Source, id: String) -> Result<Json<Value>, ApiError> {
    let d: MetricDef = st
        .store
        .pg
        .doc(src.kind(), &id)
        .await?
        .and_then(|d| serde_json::from_value(d.body).ok())
        .ok_or(ApiError::NotFound)?;
    Ok(Json(json!({"data": present(src, &id, &d)})))
}

async fn save(st: &AppState, src: Source, id: &str, d: &MetricDef) -> Result<(), ApiError> {
    generated::validate(id, d).map_err(ApiError::BadQuery)?;
    st.store
        .pg
        .put_doc(
            src.kind(),
            id,
            &serde_json::to_value(d).map_err(anyhow::Error::from)?,
        )
        .await?;
    reload(st).await?;
    Ok(())
}

async fn create(st: AppState, src: Source, body: Value) -> Result<Json<Value>, ApiError> {
    let data = body.get("data").cloned().unwrap_or(body);
    let id = data["id"].as_str().unwrap_or("").trim().to_string();
    let d: MetricDef = serde_json::from_value(data["attributes"].clone())
        .map_err(|e| ApiError::BadQuery(format!("attributes: {e}")))?;
    if st.store.pg.doc(src.kind(), &id).await?.is_some() {
        return Err(ApiError::BadQuery(format!(
            "the metric {id} already exists"
        )));
    }
    save(&st, src, &id, &d).await?;
    Ok(Json(json!({"data": present(src, &id, &d)})))
}

/// Filter, group by and percentiles can change; the computation can't (as in Datadog).
async fn update(
    st: AppState,
    src: Source,
    id: String,
    body: Value,
) -> Result<Json<Value>, ApiError> {
    let mut d: MetricDef = st
        .store
        .pg
        .doc(src.kind(), &id)
        .await?
        .and_then(|d| serde_json::from_value(d.body).ok())
        .ok_or(ApiError::NotFound)?;
    let data = body.get("data").cloned().unwrap_or(body);
    let a = &data["attributes"];
    if let Some(f) = a.get("filter") {
        d.filter = serde_json::from_value(f.clone())
            .map_err(|e| ApiError::BadQuery(format!("filter: {e}")))?;
    }
    if let Some(g) = a.get("group_by") {
        d.group_by = serde_json::from_value(g.clone())
            .map_err(|e| ApiError::BadQuery(format!("group_by: {e}")))?;
    }
    if let Some(p) = a
        .pointer("/compute/include_percentiles")
        .and_then(Value::as_bool)
    {
        d.compute.include_percentiles = Some(p);
    }
    save(&st, src, &id, &d).await?;
    Ok(Json(json!({"data": present(src, &id, &d)})))
}

async fn delete(st: AppState, src: Source, id: String) -> Result<Json<Value>, ApiError> {
    if !st.store.pg.delete_doc(src.kind(), &id).await? {
        return Err(ApiError::NotFound);
    }
    reload(&st).await?;
    Ok(Json(json!({})))
}

pub async fn logs_list(State(st): State<AppState>) -> Result<Json<Value>, ApiError> {
    list(st, Source::Logs).await
}
pub async fn logs_get(
    State(st): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    get(st, Source::Logs, id).await
}
pub async fn logs_create(
    State(st): State<AppState>,
    Json(b): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    create(st, Source::Logs, b).await
}
pub async fn logs_update(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Json(b): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    update(st, Source::Logs, id, b).await
}
pub async fn logs_delete(
    State(st): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    delete(st, Source::Logs, id).await
}
pub async fn spans_list(State(st): State<AppState>) -> Result<Json<Value>, ApiError> {
    list(st, Source::Spans).await
}
pub async fn spans_get(
    State(st): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    get(st, Source::Spans, id).await
}
pub async fn spans_create(
    State(st): State<AppState>,
    Json(b): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    create(st, Source::Spans, b).await
}
pub async fn spans_update(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Json(b): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    update(st, Source::Spans, id, b).await
}
pub async fn spans_delete(
    State(st): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    delete(st, Source::Spans, id).await
}
