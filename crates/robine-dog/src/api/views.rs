//! Saved views of the explorers: a name, the page (`logs`, `traces`…) and the
//! page's URL state (query, facets, columns, visualization, time range).

use super::ApiError;
use crate::AppState;
use crate::store::pg::new_id;
use axum::Json;
use axum::extract::{Path, Query, State};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub const KIND: &str = "saved_view";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct View {
    #[serde(default, skip_deserializing)]
    pub id: String,
    pub name: String,
    pub page: String,
    /// The URL's query string, without the leading `?`.
    #[serde(default)]
    pub params: String,
}

#[derive(Deserialize)]
pub struct Filter {
    #[serde(default)]
    pub page: Option<String>,
}

fn check(v: &View) -> Result<(), ApiError> {
    if v.name.trim().is_empty() {
        return Err(ApiError::BadQuery("a view needs a name".into()));
    }
    if !matches!(v.page.as_str(), "logs" | "traces" | "metrics" | "errors") {
        return Err(ApiError::BadQuery(
            "page is logs, traces, metrics or errors".into(),
        ));
    }
    Ok(())
}

pub async fn list(
    State(st): State<AppState>,
    Query(f): Query<Filter>,
) -> Result<Json<Vec<View>>, ApiError> {
    let mut out: Vec<View> = st
        .store
        .pg
        .docs(KIND)
        .await?
        .into_iter()
        .filter_map(|d| {
            let mut v: View = serde_json::from_value(d.body).ok()?;
            v.id = d.id;
            Some(v)
        })
        .filter(|v| f.page.as_deref().is_none_or(|p| p == v.page))
        .collect();
    out.sort_by_key(|v| v.name.to_lowercase());
    Ok(Json(out))
}

pub async fn create(
    State(st): State<AppState>,
    Json(mut v): Json<View>,
) -> Result<Json<View>, ApiError> {
    check(&v)?;
    v.id = new_id();
    st.store
        .pg
        .put_doc(
            KIND,
            &v.id,
            &serde_json::to_value(&v).map_err(anyhow::Error::from)?,
        )
        .await?;
    Ok(Json(v))
}

pub async fn update(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Json(mut v): Json<View>,
) -> Result<Json<View>, ApiError> {
    st.store
        .pg
        .doc(KIND, &id)
        .await?
        .ok_or(ApiError::NotFound)?;
    check(&v)?;
    v.id = id.clone();
    st.store
        .pg
        .put_doc(
            KIND,
            &id,
            &serde_json::to_value(&v).map_err(anyhow::Error::from)?,
        )
        .await?;
    Ok(Json(v))
}

pub async fn delete(
    State(st): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    if !st.store.pg.delete_doc(KIND, &id).await? {
        return Err(ApiError::NotFound);
    }
    Ok(Json(json!({})))
}
