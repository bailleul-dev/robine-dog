//! Dashboard lists (`/api/v1/dashboard/lists/manual`, and their dashboards
//! under `/api/v2/dashboard/lists/manual/{id}/dashboards`), and each user's
//! favorite dashboards.

use super::{ApiError, ApiResult, dashboards};
use crate::AppState;
use crate::auth::Identity;
use crate::store::pg::Doc;
use axum::extract::{Path, State};
use axum::{Extension, Json};
use serde::Deserialize;
use serde_json::{Value, json};

pub const KIND: &str = "dashboard_list";
const FAVORITES: &str = "dashboard_favorites";

fn iso(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|d| d.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
        .unwrap_or_default()
}

fn ids(body: &Value) -> Vec<String> {
    body["dashboards"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

fn present(d: &Doc) -> Value {
    json!({
        "id": d.id.parse::<i64>().unwrap_or(0),
        "name": d.body["name"],
        "type": "manual_dashboard_list",
        "dashboard_count": ids(&d.body).len(),
        "is_favorite": false,
        "author": d.body["author"],
        "created": iso(d.created_at),
        "modified": iso(d.updated_at),
    })
}

async fn load(st: &AppState, id: i64) -> Result<Doc, ApiError> {
    st.store
        .pg
        .doc(KIND, &id.to_string())
        .await?
        .ok_or(ApiError::NotFound)
}

pub async fn list(State(st): State<AppState>) -> ApiResult<Value> {
    let mut docs = st.store.pg.docs(KIND).await?;
    docs.sort_by_key(|d| d.id.parse::<i64>().unwrap_or(0));
    Ok(Json(
        json!({"dashboard_lists": docs.iter().map(present).collect::<Vec<_>>()}),
    ))
}

pub async fn get(State(st): State<AppState>, Path(id): Path<i64>) -> ApiResult<Value> {
    Ok(Json(present(&load(&st, id).await?)))
}

#[derive(Deserialize)]
pub struct ListBody {
    name: String,
}

fn name(b: &ListBody) -> Result<String, ApiError> {
    let n = b.name.trim();
    if n.is_empty() {
        return Err(ApiError::BadQuery("name is required".into()));
    }
    Ok(n.to_string())
}

pub async fn create(
    State(st): State<AppState>,
    id: Option<Extension<Identity>>,
    Json(b): Json<ListBody>,
) -> ApiResult<Value> {
    let name = name(&b)?;
    let next = st
        .store
        .pg
        .docs(KIND)
        .await?
        .iter()
        .filter_map(|d| d.id.parse::<i64>().ok())
        .max()
        .unwrap_or(0)
        + 1;
    let author = id.map(|Extension(i)| {
        json!({"handle": i.email.clone().unwrap_or(i.sub.clone()), "name": i.name, "email": i.email})
    });
    let d = st
        .store
        .pg
        .put_doc(
            KIND,
            &next.to_string(),
            &json!({"name": name, "dashboards": [], "author": author.unwrap_or(json!({"handle": "robine-dog", "name": "robine-dog"}))}),
        )
        .await?;
    Ok(Json(present(&d)))
}

pub async fn update(
    State(st): State<AppState>,
    Path(id): Path<i64>,
    Json(b): Json<ListBody>,
) -> ApiResult<Value> {
    let mut d = load(&st, id).await?;
    d.body["name"] = json!(name(&b)?);
    let d = st.store.pg.put_doc(KIND, &d.id, &d.body).await?;
    Ok(Json(present(&d)))
}

pub async fn delete(State(st): State<AppState>, Path(id): Path<i64>) -> ApiResult<Value> {
    if !st.store.pg.delete_doc(KIND, &id.to_string()).await? {
        return Err(ApiError::NotFound);
    }
    Ok(Json(json!({"deleted_dashboard_list_id": id})))
}

/// Datadog's dashboard types: ordered layouts were timeboards, free ones
/// screenboards.
fn dash_type(d: &Doc) -> &'static str {
    if d.body["layout_type"] == "free" {
        "custom_screenboard"
    } else {
        "custom_timeboard"
    }
}

fn dash_item(d: &Doc) -> Value {
    let s = dashboards::summary(d);
    json!({
        "id": d.id,
        "type": dash_type(d),
        "title": s["title"],
        "url": s["url"],
        "created": s["created_at"],
        "modified": s["modified_at"],
        "is_read_only": false,
        "is_shared": false,
        "popularity": 0,
        "author": {"handle": "robine-dog", "name": "robine-dog"},
    })
}

pub async fn dashboards(State(st): State<AppState>, Path(id): Path<i64>) -> ApiResult<Value> {
    let l = load(&st, id).await?;
    let all = st.store.pg.docs(dashboards::KIND).await?;
    let items: Vec<Value> = ids(&l.body)
        .iter()
        .filter_map(|i| all.iter().find(|d| &d.id == i).map(dash_item))
        .collect();
    Ok(Json(json!({"total": items.len(), "dashboards": items})))
}

#[derive(Deserialize)]
pub struct Items {
    dashboards: Vec<Item>,
}

#[derive(Deserialize, serde::Serialize, Clone)]
pub struct Item {
    id: String,
    #[serde(rename = "type", default)]
    kind: Option<String>,
}

/// The items that are dashboards of robine-dog, with their types.
async fn known(st: &AppState, items: &[Item]) -> Result<Vec<(String, &'static str)>, ApiError> {
    let all = st.store.pg.docs(dashboards::KIND).await?;
    items
        .iter()
        .map(|i| {
            all.iter()
                .find(|d| d.id == i.id)
                .map(|d| (d.id.clone(), dash_type(d)))
                .ok_or_else(|| ApiError::BadQuery(format!("no dashboard {}", i.id)))
        })
        .collect()
}

fn listed(items: &[(String, &str)]) -> Vec<Value> {
    items
        .iter()
        .map(|(id, t)| json!({"id": id, "type": t}))
        .collect()
}

pub async fn add(
    State(st): State<AppState>,
    Path(id): Path<i64>,
    Json(b): Json<Items>,
) -> ApiResult<Value> {
    let mut l = load(&st, id).await?;
    let items = known(&st, &b.dashboards).await?;
    let mut current = ids(&l.body);
    for (i, _) in &items {
        if !current.contains(i) {
            current.push(i.clone());
        }
    }
    l.body["dashboards"] = json!(current);
    st.store.pg.put_doc(KIND, &l.id, &l.body).await?;
    Ok(Json(json!({"added_dashboards_to_list": listed(&items)})))
}

pub async fn remove(
    State(st): State<AppState>,
    Path(id): Path<i64>,
    Json(b): Json<Items>,
) -> ApiResult<Value> {
    let mut l = load(&st, id).await?;
    let gone: Vec<&String> = b.dashboards.iter().map(|i| &i.id).collect();
    let before = ids(&l.body);
    let removed: Vec<Value> = b
        .dashboards
        .iter()
        .filter(|i| before.contains(&i.id))
        .map(|i| json!({"id": i.id, "type": i.kind}))
        .collect();
    l.body["dashboards"] = json!(
        before
            .into_iter()
            .filter(|i| !gone.contains(&i))
            .collect::<Vec<_>>()
    );
    st.store.pg.put_doc(KIND, &l.id, &l.body).await?;
    Ok(Json(json!({"deleted_dashboards_from_list": removed})))
}

pub async fn replace(
    State(st): State<AppState>,
    Path(id): Path<i64>,
    Json(b): Json<Items>,
) -> ApiResult<Value> {
    let mut l = load(&st, id).await?;
    let items = known(&st, &b.dashboards).await?;
    let mut ordered: Vec<String> = Vec::new();
    for (i, _) in &items {
        if !ordered.contains(i) {
            ordered.push(i.clone());
        }
    }
    l.body["dashboards"] = json!(ordered);
    st.store.pg.put_doc(KIND, &l.id, &l.body).await?;
    Ok(Json(json!({"dashboards": listed(&items)})))
}

/// The lists, with their dashboard ids, for the UI.
pub async fn ui_lists(State(st): State<AppState>) -> ApiResult<Value> {
    let mut docs = st.store.pg.docs(KIND).await?;
    docs.sort_by_key(|d| d.id.parse::<i64>().unwrap_or(0));
    Ok(Json(json!(
        docs.iter()
            .map(|d| {
                let mut p = present(d);
                p["dashboard_ids"] = json!(ids(&d.body));
                p
            })
            .collect::<Vec<_>>()
    )))
}

// --- favorites -------------------------------------------------------------------------

async fn favorites_of(st: &AppState, sub: &str) -> Result<Vec<String>, ApiError> {
    Ok(st
        .store
        .pg
        .doc(FAVORITES, sub)
        .await?
        .map(|d| ids(&d.body))
        .unwrap_or_default())
}

pub async fn favorites(
    State(st): State<AppState>,
    Extension(id): Extension<Identity>,
) -> ApiResult<Value> {
    Ok(Json(json!(favorites_of(&st, &id.sub).await?)))
}

async fn set_favorite(st: &AppState, sub: &str, dash: &str, on: bool) -> ApiResult<Value> {
    let mut favs = favorites_of(st, sub).await?;
    favs.retain(|f| f != dash);
    if on {
        st.store
            .pg
            .doc(dashboards::KIND, dash)
            .await?
            .ok_or(ApiError::NotFound)?;
        favs.push(dash.to_string());
    }
    st.store
        .pg
        .put_doc(FAVORITES, sub, &json!({"dashboards": favs}))
        .await?;
    Ok(Json(json!(favs)))
}

pub async fn star(
    State(st): State<AppState>,
    Extension(id): Extension<Identity>,
    Path(dash): Path<String>,
) -> ApiResult<Value> {
    set_favorite(&st, &id.sub, &dash, true).await
}

pub async fn unstar(
    State(st): State<AppState>,
    Extension(id): Extension<Identity>,
    Path(dash): Path<String>,
) -> ApiResult<Value> {
    set_favorite(&st, &id.sub, &dash, false).await
}
