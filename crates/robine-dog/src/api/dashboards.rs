//! Dashboards, stored as Datadog's dashboard JSON (`/api/v1/dashboard`):
//! `{title, description, layout_type, widgets: [{id, definition, layout}],
//! template_variables}`. Widgets are opaque to the server: the UI renders them
//! and runs their requests through the v2 query API.

use super::ApiError;
use crate::AppState;
use crate::store::pg::Doc;
use axum::Json;
use axum::extract::{Path, State};
use rand::Rng;
use serde_json::{Value, json};

pub const KIND: &str = "dashboard";

type ApiResult<T> = Result<Json<T>, ApiError>;

fn iso(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|d| d.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
        .unwrap_or_default()
}

/// Datadog's `abc-def-ghi` ids.
fn new_dashboard_id() -> String {
    let mut rng = rand::rng();
    let part = |rng: &mut rand::rngs::ThreadRng| -> String {
        (0..3)
            .map(|_| (b'a' + rng.random_range(0..26u8)) as char)
            .collect()
    };
    format!("{}-{}-{}", part(&mut rng), part(&mut rng), part(&mut rng))
}

fn slug(title: &str) -> String {
    let s: String = title
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    s.split('-')
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

fn present(d: &Doc) -> Value {
    let mut body = d.body.clone();
    let title = body["title"].as_str().unwrap_or("").to_string();
    if let Some(o) = body.as_object_mut() {
        o.insert("id".into(), json!(d.id));
        o.insert("created_at".into(), json!(iso(d.created_at)));
        o.insert("modified_at".into(), json!(iso(d.updated_at)));
        o.insert(
            "url".into(),
            json!(format!("/dashboard/{}/{}", d.id, slug(&title))),
        );
    }
    body
}

fn summary(d: &Doc) -> Value {
    let full = present(d);
    json!({
        "id": full["id"],
        "title": full["title"],
        "description": full["description"],
        "layout_type": full["layout_type"],
        "url": full["url"],
        "created_at": full["created_at"],
        "modified_at": full["modified_at"],
        "is_read_only": false,
        "widget_count": full["widgets"].as_array().map(Vec::len).unwrap_or(0),
    })
}

/// Checks the shape and gives every widget an id.
fn normalize(mut v: Value) -> Result<Value, ApiError> {
    let o = v
        .as_object_mut()
        .ok_or_else(|| ApiError::BadQuery("a dashboard is a JSON object".into()))?;
    let title = o.get("title").and_then(Value::as_str).unwrap_or("").trim();
    if title.is_empty() {
        return Err(ApiError::BadQuery("title is required".into()));
    }
    for k in ["id", "created_at", "modified_at", "url"] {
        o.remove(k);
    }
    let layout = o
        .get("layout_type")
        .and_then(Value::as_str)
        .unwrap_or("ordered")
        .to_string();
    if layout != "ordered" && layout != "free" {
        return Err(ApiError::BadQuery("layout_type is ordered or free".into()));
    }
    o.insert("layout_type".into(), json!(layout));
    o.entry("description").or_insert(Value::Null);
    o.entry("template_variables").or_insert_with(|| json!([]));
    let widgets = o.entry("widgets").or_insert_with(|| json!([]));
    let Some(list) = widgets.as_array_mut() else {
        return Err(ApiError::BadQuery("widgets is an array".into()));
    };
    let mut rng = rand::rng();
    fn fill(list: &mut [Value], rng: &mut rand::rngs::ThreadRng) -> Result<(), ApiError> {
        for w in list {
            let wo = w
                .as_object_mut()
                .ok_or_else(|| ApiError::BadQuery("a widget is an object".into()))?;
            if !wo.get("definition").is_some_and(Value::is_object) {
                return Err(ApiError::BadQuery("every widget has a definition".into()));
            }
            if !wo.get("id").is_some_and(Value::is_number) {
                wo.insert("id".into(), json!(rng.random_range(1..(1i64 << 52))));
            }
            if let Some(children) = wo
                .get_mut("definition")
                .and_then(|d| d.get_mut("widgets"))
                .and_then(Value::as_array_mut)
            {
                fill(children, rng)?;
            }
        }
        Ok(())
    }
    fill(list, &mut rng)?;
    Ok(v)
}

pub async fn list(State(st): State<AppState>) -> ApiResult<Value> {
    let docs = st.store.pg.docs(KIND).await?;
    Ok(Json(
        json!({"dashboards": docs.iter().map(summary).collect::<Vec<_>>()}),
    ))
}

pub async fn get(State(st): State<AppState>, Path(id): Path<String>) -> ApiResult<Value> {
    let d = st
        .store
        .pg
        .doc(KIND, &id)
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(present(&d)))
}

pub async fn create(State(st): State<AppState>, Json(body): Json<Value>) -> ApiResult<Value> {
    let body = normalize(body)?;
    let d = st
        .store
        .pg
        .put_doc(KIND, &new_dashboard_id(), &body)
        .await?;
    Ok(Json(present(&d)))
}

pub async fn update(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> ApiResult<Value> {
    st.store
        .pg
        .doc(KIND, &id)
        .await?
        .ok_or(ApiError::NotFound)?;
    let body = normalize(body)?;
    let d = st.store.pg.put_doc(KIND, &id, &body).await?;
    Ok(Json(present(&d)))
}

pub async fn delete(State(st): State<AppState>, Path(id): Path<String>) -> ApiResult<Value> {
    if !st.store.pg.delete_doc(KIND, &id).await? {
        return Err(ApiError::NotFound);
    }
    Ok(Json(json!({"deleted_dashboard_id": id})))
}

pub async fn clone(State(st): State<AppState>, Path(id): Path<String>) -> ApiResult<Value> {
    let d = st
        .store
        .pg
        .doc(KIND, &id)
        .await?
        .ok_or(ApiError::NotFound)?;
    let mut body = d.body;
    if let Some(o) = body.as_object_mut() {
        let title = o
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        o.insert("title".into(), json!(format!("{title} (cloned)")));
    }
    let d = st
        .store
        .pg
        .put_doc(KIND, &new_dashboard_id(), &body)
        .await?;
    Ok(Json(present(&d)))
}

/// A starter dashboard, installed once when there is none.
pub async fn ensure_default(st: &AppState) -> anyhow::Result<()> {
    if !st.store.pg.docs(KIND).await?.is_empty() {
        return Ok(());
    }
    let ts = |title: &str, queries: Value, formulas: Value, display: &str, x: i64, y: i64| {
        json!({"definition": {
            "type": "timeseries", "title": title, "show_legend": true,
            "requests": [{"response_format": "timeseries", "queries": queries, "formulas": formulas, "display_type": display}]
        }, "layout": {"x": x, "y": y, "width": 6, "height": 3}})
    };
    let body = json!({
        "title": "Overview",
        "description": "Requests, errors and latency across services.",
        "layout_type": "ordered",
        "template_variables": [{"name": "service", "prefix": "service", "default": "*", "available_values": []}],
        "widgets": [
            {"definition": {"type": "query_value", "title": "Requests (past window)", "precision": 0, "requests": [{
                "response_format": "scalar",
                "queries": [{"data_source": "spans", "name": "a", "search": {"query": "$service"}, "compute": {"aggregation": "count"}}],
                "formulas": [{"formula": "a"}]}]},
             "layout": {"x": 0, "y": 0, "width": 3, "height": 2}},
            {"definition": {"type": "query_value", "title": "Error rate", "precision": 2, "custom_unit": "%", "requests": [{
                "response_format": "scalar",
                "queries": [
                    {"data_source": "spans", "name": "a", "search": {"query": "$service status:error"}, "compute": {"aggregation": "count"}},
                    {"data_source": "spans", "name": "b", "search": {"query": "$service"}, "compute": {"aggregation": "count"}}],
                "formulas": [{"formula": "a / b * 100"}]}],
                "conditional_formats": [{"comparator": ">", "value": 5, "palette": "white_on_red"}, {"comparator": "<=", "value": 5, "palette": "white_on_green"}]},
             "layout": {"x": 3, "y": 0, "width": 3, "height": 2}},
            {"definition": {"type": "query_value", "title": "Error logs", "precision": 0, "requests": [{
                "response_format": "scalar",
                "queries": [{"data_source": "logs", "name": "a", "search": {"query": "$service status:error"}, "compute": {"aggregation": "count"}}],
                "formulas": [{"formula": "a"}]}]},
             "layout": {"x": 6, "y": 0, "width": 3, "height": 2}},
            {"definition": {"type": "note", "content": "**Overview**\n\nSpans and logs from every service. Pick a service above to narrow every widget.", "background_color": "yellow"},
             "layout": {"x": 9, "y": 0, "width": 3, "height": 2}},
            ts("Hits by service", json!([{"data_source": "spans", "name": "a", "search": {"query": "$service"}, "compute": {"aggregation": "count"}, "group_by": [{"facet": "service", "limit": 10}]}]), json!([{"formula": "a"}]), "bars", 0, 2),
            ts("p95 latency by service (ms)", json!([{"data_source": "spans", "name": "a", "search": {"query": "$service"}, "compute": {"aggregation": "pc95", "metric": "@duration"}, "group_by": [{"facet": "service", "limit": 10}]}]), json!([{"formula": "a / 1000000", "alias": "p95 (ms)"}]), "line", 6, 2),
            ts("Logs by status", json!([{"data_source": "logs", "name": "a", "search": {"query": "$service"}, "compute": {"aggregation": "count"}, "group_by": [{"facet": "status", "limit": 10}]}]), json!([{"formula": "a"}]), "bars", 0, 5),
            {"definition": {"type": "toplist", "title": "Top services by errors", "requests": [{
                "response_format": "scalar",
                "queries": [{"data_source": "spans", "name": "a", "search": {"query": "status:error"}, "compute": {"aggregation": "count"}, "group_by": [{"facet": "service", "limit": 10}]}],
                "formulas": [{"formula": "a", "limit": {"count": 10, "order": "desc"}}]}]},
             "layout": {"x": 6, "y": 5, "width": 6, "height": 3}},
        ]
    });
    st.store
        .pg
        .put_doc(
            KIND,
            &new_dashboard_id(),
            &normalize(body).map_err(|e| anyhow::anyhow!("{e:?}"))?,
        )
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes() {
        assert!(normalize(json!({"title": ""})).is_err());
        assert!(normalize(json!({"title": "x", "layout_type": "grid"})).is_err());
        let v = normalize(json!({"title": "x", "id": "zzz", "widgets": [
            {"definition": {"type": "group", "widgets": [{"definition": {"type": "note"}}]}}
        ]}))
        .unwrap();
        assert!(v.get("id").is_none());
        assert!(v["widgets"][0]["id"].is_number());
        assert!(v["widgets"][0]["definition"]["widgets"][0]["id"].is_number());
        assert_eq!(v["layout_type"], "ordered");
        assert_eq!(slug("My  Dash / prod"), "my-dash-prod");
        let id = new_dashboard_id();
        assert_eq!(id.len(), 11);
    }
}
