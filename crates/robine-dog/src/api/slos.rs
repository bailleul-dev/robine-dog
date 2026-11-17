//! Service level objectives, metric-based as in Datadog: a numerator (good
//! events) and a denominator (all events), both metric queries, and targets
//! per time frame. The definitions are stored here; the UI computes their
//! status with the metrics queries.

use super::{ApiError, ApiResult};
use crate::AppState;
use crate::store::pg::new_id;
use axum::Json;
use axum::extract::{Path, State};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const KIND: &str = "slo";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SloQuery {
    /// Good events: a metric query, or `a - b` (all minus bad).
    pub numerator: String,
    pub denominator: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Threshold {
    /// `7d`, `30d` or `90d`.
    pub timeframe: String,
    /// Percent.
    pub target: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warning: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Slo {
    #[serde(default)]
    pub id: Option<String>,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(rename = "type", default)]
    pub typ: String,
    pub query: SloQuery,
    pub thresholds: Vec<Threshold>,
    #[serde(default)]
    pub tags: Vec<String>,
}

fn check(mut s: Slo) -> Result<Slo, ApiError> {
    s.name = s.name.trim().to_string();
    if s.name.is_empty() {
        return Err(ApiError::BadQuery("an SLO needs a name".into()));
    }
    for q in s
        .query
        .numerator
        .split(" - ")
        .chain([s.query.denominator.as_str()])
    {
        crate::metrics::query::parse(q.trim())
            .map_err(|e| ApiError::BadQuery(format!("{}: {e}", q.trim())))?;
    }
    if s.thresholds.is_empty() {
        return Err(ApiError::BadQuery("an SLO needs a target".into()));
    }
    for t in &s.thresholds {
        if !matches!(t.timeframe.as_str(), "7d" | "30d" | "90d") {
            return Err(ApiError::BadQuery("time frames are 7d, 30d or 90d".into()));
        }
        if !(t.target > 0.0 && t.target < 100.0) {
            return Err(ApiError::BadQuery("a target is between 0 and 100 %".into()));
        }
    }
    s.typ = "metric".into();
    Ok(s)
}

async fn save(st: &AppState, s: &Slo) -> Result<(), ApiError> {
    let id = s.id.clone().unwrap_or_default();
    st.store
        .pg
        .put_doc(
            KIND,
            &id,
            &serde_json::to_value(s).map_err(anyhow::Error::from)?,
        )
        .await?;
    Ok(())
}

pub async fn list(State(st): State<AppState>) -> ApiResult<Value> {
    let mut all: Vec<Slo> = st
        .store
        .pg
        .docs(KIND)
        .await?
        .into_iter()
        .filter_map(|d| serde_json::from_value(d.body).ok())
        .collect();
    all.sort_by_key(|s| s.name.to_lowercase());
    Ok(Json(json!({"data": all})))
}

pub async fn create(State(st): State<AppState>, Json(s): Json<Slo>) -> ApiResult<Value> {
    let mut s = check(s)?;
    s.id = Some(new_id());
    save(&st, &s).await?;
    Ok(Json(json!({"data": [s]})))
}

pub async fn update(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Json(s): Json<Slo>,
) -> ApiResult<Value> {
    st.store
        .pg
        .doc(KIND, &id)
        .await?
        .ok_or(ApiError::NotFound)?;
    let mut s = check(s)?;
    s.id = Some(id);
    save(&st, &s).await?;
    Ok(Json(json!({"data": [s]})))
}

pub async fn delete(State(st): State<AppState>, Path(id): Path<String>) -> ApiResult<Value> {
    if !st.store.pg.delete_doc(KIND, &id).await? {
        return Err(ApiError::NotFound);
    }
    Ok(Json(json!({"data": [id]})))
}
