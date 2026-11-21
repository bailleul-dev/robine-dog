//! API keys and application keys: named, revocable keys
//! (`/api/v2/api_keys`, `/api/v2/application_keys`), stored hashed; the key
//! itself is shown once, at creation. API keys are accepted wherever
//! `ROBINE_DOG_API_KEY` is (Agents, SDKs, the public API), which stays valid.

use crate::AppState;
use crate::api::{ApiError, ApiResult};
use crate::store::pg::new_id;
use axum::Json;
use axum::extract::{Path, State};
use rand::Rng;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex, RwLock};

pub const API: &str = "api_key";
pub const APP: &str = "app_key";

/// Hash → (kind, id).
static KEYS: LazyLock<RwLock<HashMap<String, (&'static str, String)>>> =
    LazyLock::new(Default::default);
/// Id → last use (ms), written to the documents every minute.
static USED: LazyLock<Mutex<HashMap<String, i64>>> = LazyLock::new(Default::default);

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn hash(key: &str) -> String {
    hex(&Sha256::digest(key.as_bytes()))
}

/// A known API key (not the configured one, checked by the caller).
pub fn api_key_valid(key: &str) -> bool {
    lookup(key, API)
}

/// Records the use of an application key, if it is one.
pub fn touch_app_key(key: &str) {
    lookup(key, APP);
}

fn lookup(key: &str, kind: &str) -> bool {
    let keys = KEYS.read().unwrap_or_else(|e| e.into_inner());
    if keys.is_empty() {
        return false;
    }
    match keys.get(&hash(key)) {
        Some((k, id)) if *k == kind => {
            USED.lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(id.clone(), crate::intake::logs::now_ms());
            true
        }
        _ => false,
    }
}

pub async fn init(st: &AppState) -> anyhow::Result<()> {
    let mut m = HashMap::new();
    for kind in [API, APP] {
        for d in st.store.pg.docs(kind).await? {
            if let Some(h) = d.body["hash"].as_str() {
                m.insert(h.to_string(), (kind, d.id));
            }
        }
    }
    *KEYS.write().unwrap_or_else(|e| e.into_inner()) = m;
    let st = st.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            let used = std::mem::take(&mut *USED.lock().unwrap_or_else(|e| e.into_inner()));
            for (id, ts) in used {
                for kind in [API, APP] {
                    if let Ok(Some(d)) = st.store.pg.doc(kind, &id).await {
                        let mut b = d.body;
                        b["last_used_at"] = json!(ts);
                        let _ = st.store.pg.put_doc(kind, &id, &b).await;
                    }
                }
            }
        }
    });
    Ok(())
}

fn typ(kind: &str) -> &'static str {
    if kind == API {
        "api_keys"
    } else {
        "application_keys"
    }
}

fn view(kind: &str, id: &str, b: &Value, key: Option<&str>) -> Value {
    let iso = |v: &Value| v.as_i64().map(crate::logconfig::iso);
    let last_used = USED
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(id)
        .copied()
        .or(b["last_used_at"].as_i64());
    let mut a = json!({"name": b["name"], "last4": b["last4"], "created_at": iso(&b["created_at"]),
        "modified_at": iso(&b["modified_at"]), "last_used_at": last_used.map(crate::logconfig::iso),
        "scopes": b["scopes"]});
    if let Some(k) = key {
        a["key"] = json!(k);
    }
    json!({"type": typ(kind), "id": id, "attributes": a})
}

async fn list(st: &AppState, kind: &str) -> ApiResult<Value> {
    let docs = st.store.pg.docs(kind).await?;
    Ok(Json(
        json!({"data": docs.iter().map(|d| view(kind, &d.id, &d.body, None)).collect::<Vec<_>>(),
        "meta": {"page": {"total_filtered_count": docs.len()}}}),
    ))
}

async fn create(st: &AppState, kind: &'static str, v: Value) -> ApiResult<Value> {
    let name = v["data"]["attributes"]["name"]
        .as_str()
        .unwrap_or("")
        .trim()
        .to_string();
    if name.is_empty() {
        return Err(ApiError::BadQuery(
            "data.attributes.name is required".into(),
        ));
    }
    let bytes = if kind == API { 16 } else { 20 };
    let raw: Vec<u8> = (0..bytes).map(|_| rand::rng().random()).collect();
    let key = hex(&raw);
    let now = crate::intake::logs::now_ms();
    let id = new_id();
    let b = json!({"name": name, "hash": hash(&key), "last4": &key[key.len() - 4..], "created_at": now,
        "modified_at": now, "scopes": v["data"]["attributes"]["scopes"]});
    st.store.pg.put_doc(kind, &id, &b).await?;
    KEYS.write()
        .unwrap_or_else(|e| e.into_inner())
        .insert(hash(&key), (kind, id.clone()));
    Ok(Json(json!({"data": view(kind, &id, &b, Some(&key))})))
}

async fn get(st: &AppState, kind: &str, id: &str) -> ApiResult<Value> {
    let d = st.store.pg.doc(kind, id).await?.ok_or(ApiError::NotFound)?;
    Ok(Json(json!({"data": view(kind, id, &d.body, None)})))
}

async fn rename(st: &AppState, kind: &str, id: &str, v: Value) -> ApiResult<Value> {
    let d = st.store.pg.doc(kind, id).await?.ok_or(ApiError::NotFound)?;
    let mut b = d.body;
    if let Some(n) = v["data"]["attributes"]["name"]
        .as_str()
        .filter(|n| !n.trim().is_empty())
    {
        b["name"] = json!(n.trim());
    }
    b["modified_at"] = json!(crate::intake::logs::now_ms());
    st.store.pg.put_doc(kind, id, &b).await?;
    Ok(Json(json!({"data": view(kind, id, &b, None)})))
}

async fn revoke(st: &AppState, kind: &str, id: &str) -> ApiResult<Value> {
    if let Some(d) = st.store.pg.doc(kind, id).await?
        && let Some(h) = d.body["hash"].as_str()
    {
        KEYS.write().unwrap_or_else(|e| e.into_inner()).remove(h);
    }
    st.store.pg.delete_doc(kind, id).await?;
    Ok(Json(json!({})))
}

pub async fn list_api(State(st): State<AppState>) -> ApiResult<Value> {
    list(&st, API).await
}
pub async fn create_api(State(st): State<AppState>, Json(v): Json<Value>) -> ApiResult<Value> {
    create(&st, API, v).await
}
pub async fn get_api(State(st): State<AppState>, Path(id): Path<String>) -> ApiResult<Value> {
    get(&st, API, &id).await
}
pub async fn update_api(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Json(v): Json<Value>,
) -> ApiResult<Value> {
    rename(&st, API, &id, v).await
}
pub async fn delete_api(State(st): State<AppState>, Path(id): Path<String>) -> ApiResult<Value> {
    revoke(&st, API, &id).await
}
pub async fn list_app(State(st): State<AppState>) -> ApiResult<Value> {
    list(&st, APP).await
}
pub async fn create_app(State(st): State<AppState>, Json(v): Json<Value>) -> ApiResult<Value> {
    create(&st, APP, v).await
}
pub async fn get_app(State(st): State<AppState>, Path(id): Path<String>) -> ApiResult<Value> {
    get(&st, APP, &id).await
}
pub async fn update_app(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Json(v): Json<Value>,
) -> ApiResult<Value> {
    rename(&st, APP, &id, v).await
}
pub async fn delete_app(State(st): State<AppState>, Path(id): Path<String>) -> ApiResult<Value> {
    revoke(&st, APP, &id).await
}

/// `GET /api/ui/help`: the README, and whether an API key is configured.
pub async fn help(State(st): State<AppState>) -> Json<Value> {
    Json(
        json!({"readme": include_str!("../../../README.md"), "api_key_configured": st.cfg.api_key.is_some(),
        "version": std::env::var("ROBINE_DOG_VERSION").unwrap_or_else(|_| env!("CARGO_PKG_VERSION").into())}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_checked_by_hash_and_kind() {
        KEYS.write().unwrap().insert(hash("abc"), (API, "1".into()));
        KEYS.write().unwrap().insert(hash("def"), (APP, "2".into()));
        assert!(api_key_valid("abc"));
        assert!(!api_key_valid("def"));
        assert!(!api_key_valid("nope"));
        assert!(USED.lock().unwrap().contains_key("1"));
    }
}
