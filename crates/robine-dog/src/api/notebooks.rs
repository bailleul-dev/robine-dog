//! Notebooks: a title and a list of cells (text, metric graphs, log
//! graphs), as in Datadog. The cells are the UI's; they are stored as given.

use super::{ApiError, ApiResult};
use crate::AppState;
use crate::store::pg::new_id;
use axum::Json;
use axum::extract::{Path, State};
use serde_json::{Value, json};

const KIND: &str = "notebook";

fn checked(mut v: Value) -> Result<Value, ApiError> {
    let name = v
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or("");
    if name.is_empty() {
        return Err(ApiError::BadQuery("a notebook needs a name".into()));
    }
    if !v.get("cells").is_some_and(Value::is_array) {
        v["cells"] = json!([]);
    }
    Ok(v)
}

pub async fn list(State(st): State<AppState>) -> ApiResult<Value> {
    let mut docs = st.store.pg.docs(KIND).await?;
    docs.sort_by_key(|d| std::cmp::Reverse(d.updated_at));
    let data: Vec<Value> = docs
        .into_iter()
        .map(|d| {
            json!({
                "id": d.id,
                "name": d.body.get("name"),
                "cells": d.body.get("cells").and_then(Value::as_array).map_or(0, Vec::len),
                "modified_at": d.updated_at,
            })
        })
        .collect();
    Ok(Json(json!({"data": data})))
}

pub async fn get(State(st): State<AppState>, Path(id): Path<String>) -> ApiResult<Value> {
    let d = st
        .store
        .pg
        .doc(KIND, &id)
        .await?
        .ok_or(ApiError::NotFound)?;
    let mut body = d.body;
    body["id"] = json!(d.id);
    Ok(Json(body))
}

pub async fn create(State(st): State<AppState>, Json(v): Json<Value>) -> ApiResult<Value> {
    let mut v = checked(v)?;
    let id = new_id();
    v["id"] = json!(id);
    st.store.pg.put_doc(KIND, &id, &v).await?;
    Ok(Json(v))
}

pub async fn update(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Json(v): Json<Value>,
) -> ApiResult<Value> {
    st.store
        .pg
        .doc(KIND, &id)
        .await?
        .ok_or(ApiError::NotFound)?;
    let mut v = checked(v)?;
    v["id"] = json!(id);
    st.store.pg.put_doc(KIND, &id, &v).await?;
    Ok(Json(v))
}

pub async fn delete(State(st): State<AppState>, Path(id): Path<String>) -> ApiResult<Value> {
    if !st.store.pg.delete_doc(KIND, &id).await? {
        return Err(ApiError::NotFound);
    }
    Ok(Json(json!({})))
}

/// What the UI needs to report its own sessions here (RUM on itself):
/// nothing without a client token.
pub async fn rum_config(State(st): State<AppState>) -> ApiResult<Value> {
    Ok(Json(json!({
        "client_token": st.cfg.client_token,
        "version": env!("CARGO_PKG_VERSION"),
        "env": std::env::var("DD_ENV").ok(),
    })))
}
