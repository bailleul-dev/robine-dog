//! Monitors and webhooks: the UI's pages and Datadog's API
//! (`/api/v1/monitor`, `/api/v1/integration/webhooks/configuration/webhooks`).

use super::{ApiError, ApiResult};
use crate::AppState;
use crate::monitors::webhooks::{self, WebhookDef};
use crate::monitors::{self, KIND, MonitorDef};
use crate::store::pg::{Doc, GroupState, MonitorEvent};
use axum::Json;
use axum::extract::{Path, Query, State};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use std::collections::HashMap;

fn iso(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|d| d.format("%Y-%m-%dT%H:%M:%S%.6f+00:00").to_string())
        .unwrap_or_default()
}

fn parse_id(id: &str) -> Result<i64, ApiError> {
    id.parse().map_err(|_| ApiError::NotFound)
}

fn from_doc(d: &Doc) -> Option<MonitorDef> {
    let mut m: MonitorDef = serde_json::from_value(d.body.clone()).ok()?;
    m.id = d.id.parse().ok();
    Some(m)
}

/// The monitor as Datadog's API returns it, with its groups' state.
fn present(def: &MonitorDef, doc: &Doc, groups: &[GroupState]) -> Value {
    let mut v = serde_json::to_value(def).unwrap_or_default();
    let multi = monitors::parse(&def.query)
        .map(|p| match &p.source {
            monitors::Source::Metric { query, .. } => !query.by.is_empty(),
            monitors::Source::Events { by, .. } => !by.is_empty(),
        })
        .unwrap_or(false);
    let secs = |ms: Option<i64>| ms.map(|m| m / 1000);
    let state: Map<String, Value> = groups
        .iter()
        .map(|g| {
            (
                g.group.clone(),
                json!({
                    "name": g.group,
                    "status": g.status,
                    "value": g.value,
                    "last_triggered_ts": secs(g.last_triggered),
                    "last_notified_ts": secs(g.last_notified),
                    "last_resolved_ts": secs(g.last_resolved),
                    "last_data_ts": secs(g.last_data),
                    "since_ts": g.since / 1000,
                }),
            )
        })
        .collect();
    let o = v.as_object_mut().expect("monitor object");
    o.insert("id".into(), json!(def.id));
    o.insert(
        "overall_state".into(),
        json!(monitors::overall(groups.iter().map(|g| g.status.clone()))),
    );
    o.insert(
        "overall_state_modified".into(),
        json!(groups.iter().map(|g| g.since).max().map(iso)),
    );
    o.insert("created".into(), json!(iso(doc.created_at)));
    o.insert("modified".into(), json!(iso(doc.updated_at)));
    o.insert("multi".into(), json!(multi));
    o.insert("deleted".into(), Value::Null);
    o.insert("org_id".into(), json!(1));
    o.insert("restricted_roles".into(), Value::Null);
    o.insert(
        "creator".into(),
        json!({"name": "robine-dog", "handle": "robine-dog", "email": null}),
    );
    o.insert("state".into(), json!({"groups": state}));
    o.insert(
        "last_triggered_ts".into(),
        json!(
            groups
                .iter()
                .filter_map(|g| g.last_triggered)
                .max()
                .map(|m| m / 1000)
        ),
    );
    v
}

async fn load(st: &AppState, id: i64) -> Result<(MonitorDef, Doc), ApiError> {
    let doc = st
        .store
        .pg
        .doc(KIND, &id.to_string())
        .await?
        .ok_or(ApiError::NotFound)?;
    let def = from_doc(&doc).ok_or(ApiError::NotFound)?;
    Ok((def, doc))
}

async fn presented(st: &AppState, id: i64) -> Result<Value, ApiError> {
    let (def, doc) = load(st, id).await?;
    let groups = st.store.pg.monitor_groups(id).await?;
    Ok(present(&def, &doc, &groups))
}

#[derive(Deserialize, Default)]
pub struct ListParams {
    name: Option<String>,
    monitor_tags: Option<String>,
}

pub async fn list(
    State(st): State<AppState>,
    Query(q): Query<ListParams>,
) -> ApiResult<Vec<Value>> {
    let docs = st.store.pg.docs(KIND).await?;
    let wanted: Vec<&str> = q
        .monitor_tags
        .as_deref()
        .map(|t| {
            t.split(',')
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .collect()
        })
        .unwrap_or_default();
    let mut out = Vec::new();
    for doc in &docs {
        let Some(def) = from_doc(doc) else { continue };
        if let Some(n) = &q.name
            && !def.name.to_lowercase().contains(&n.to_lowercase())
        {
            continue;
        }
        if !wanted.iter().all(|t| def.tags.iter().any(|x| x == t)) {
            continue;
        }
        let groups = st
            .store
            .pg
            .monitor_groups(def.id.unwrap_or_default())
            .await?;
        out.push(present(&def, doc, &groups));
    }
    out.sort_by_key(|m| std::cmp::Reverse(m["id"].as_i64().unwrap_or(0)));
    Ok(Json(out))
}

pub async fn get(State(st): State<AppState>, Path(id): Path<String>) -> ApiResult<Value> {
    Ok(Json(presented(&st, parse_id(&id)?).await?))
}

/// Checks a definition and normalizes it (type, critical threshold).
fn check(mut def: MonitorDef) -> Result<MonitorDef, ApiError> {
    let p = monitors::parse(&def.query).map_err(ApiError::BadQuery)?;
    let inferred = monitors::type_of(&p);
    if !(inferred == "metric alert" && def.typ == "query alert") {
        def.typ = inferred.into();
    }
    def.options.thresholds.critical = Some(p.threshold);
    if def.name.trim().is_empty() {
        return Err(ApiError::BadQuery("a monitor needs a name".into()));
    }
    if let Some(p) = def.priority
        && !(1..=5).contains(&p)
    {
        return Err(ApiError::BadQuery("priority is 1 to 5".into()));
    }
    Ok(def)
}

async fn save(st: &AppState, id: i64, mut def: MonitorDef) -> Result<Value, ApiError> {
    def.id = None;
    let body = serde_json::to_value(&def).map_err(anyhow::Error::from)?;
    st.store.pg.put_doc(KIND, &id.to_string(), &body).await?;
    def.id = Some(id);
    monitors::evaluate_soon(st, def);
    presented(st, id).await
}

pub async fn create(State(st): State<AppState>, Json(def): Json<MonitorDef>) -> ApiResult<Value> {
    let def = check(def)?;
    let id = st.store.pg.next_monitor_id().await?;
    Ok(Json(save(&st, id, def).await?))
}

/// Datadog's update is partial: only the fields given change (options merge).
pub async fn update(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Json(patch): Json<Value>,
) -> ApiResult<Value> {
    let id = parse_id(&id)?;
    let (current, _) = load(&st, id).await?;
    let mut merged = serde_json::to_value(&current).map_err(anyhow::Error::from)?;
    if let (Some(m), Value::Object(p)) = (merged.as_object_mut(), patch) {
        for (k, v) in p {
            match (k.as_str(), m.get_mut("options"), v) {
                ("options", Some(Value::Object(cur)), Value::Object(new)) => {
                    for (ok, ov) in new {
                        cur.insert(ok, ov);
                    }
                }
                ("id", _, _) => {}
                (_, _, v) => {
                    m.insert(k, v);
                }
            }
        }
    }
    let def: MonitorDef =
        serde_json::from_value(merged).map_err(|e| ApiError::BadQuery(e.to_string()))?;
    Ok(Json(save(&st, id, check(def)?).await?))
}

pub async fn delete(State(st): State<AppState>, Path(id): Path<String>) -> ApiResult<Value> {
    let id = parse_id(&id)?;
    if !st.store.pg.delete_doc(KIND, &id.to_string()).await? {
        return Err(ApiError::NotFound);
    }
    st.store.pg.forget_monitor(id).await?;
    Ok(Json(json!({"deleted_monitor_id": id})))
}

pub async fn validate(Json(def): Json<MonitorDef>) -> ApiResult<Value> {
    check(def)?;
    Ok(Json(json!({})))
}

#[derive(Deserialize, Default)]
pub struct Mute {
    scope: Option<String>,
    /// Epoch seconds.
    end: Option<i64>,
    #[serde(default)]
    all_scopes: bool,
}

async fn set_silenced(
    st: &AppState,
    id: i64,
    f: impl FnOnce(&mut Map<String, Value>),
) -> Result<Value, ApiError> {
    let (mut def, _) = load(st, id).await?;
    f(&mut def.options.silenced);
    def.id = None;
    st.store
        .pg
        .put_doc(
            KIND,
            &id.to_string(),
            &serde_json::to_value(&def).map_err(anyhow::Error::from)?,
        )
        .await?;
    presented(st, id).await
}

pub async fn mute(
    State(st): State<AppState>,
    Path(id): Path<String>,
    body: Option<Json<Mute>>,
) -> ApiResult<Value> {
    let m = body.map(|b| b.0).unwrap_or_default();
    let scope = m.scope.unwrap_or_else(|| "*".into());
    Ok(Json(
        set_silenced(&st, parse_id(&id)?, |s| {
            s.insert(scope, m.end.map(Value::from).unwrap_or(Value::Null));
        })
        .await?,
    ))
}

pub async fn unmute(
    State(st): State<AppState>,
    Path(id): Path<String>,
    body: Option<Json<Mute>>,
) -> ApiResult<Value> {
    let m = body.map(|b| b.0).unwrap_or_default();
    Ok(Json(
        set_silenced(&st, parse_id(&id)?, |s| match m.scope {
            Some(scope) if !m.all_scopes => {
                s.remove(&scope);
            }
            _ => s.clear(),
        })
        .await?,
    ))
}

#[derive(Deserialize)]
pub struct Limit {
    limit: Option<i64>,
}

pub async fn events(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<Limit>,
) -> ApiResult<Vec<MonitorEvent>> {
    Ok(Json(
        st.store
            .pg
            .monitor_events(Some(parse_id(&id)?), q.limit.unwrap_or(100).clamp(1, 1000))
            .await?,
    ))
}

pub async fn all_events(
    State(st): State<AppState>,
    Query(q): Query<Limit>,
) -> ApiResult<Vec<MonitorEvent>> {
    Ok(Json(
        st.store
            .pg
            .monitor_events(None, q.limit.unwrap_or(100).clamp(1, 1000))
            .await?,
    ))
}

#[derive(Deserialize)]
pub struct Range {
    from: i64,
    to: i64,
}

/// The evaluated values per group, for the monitor page's graph.
pub async fn history(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Query(r): Query<Range>,
) -> ApiResult<Value> {
    let series = monitors::history(&st, parse_id(&id)?, r.from, r.to).await?;
    Ok(Json(json!(
        series
            .into_iter()
            .map(|(g, pts)| json!({"group": g, "points": pts.into_iter().map(|(t, v, s)| json!([t, v, s])).collect::<Vec<_>>()}))
            .collect::<Vec<_>>()
    )))
}

#[derive(Deserialize)]
pub struct Preview {
    query: String,
    #[serde(default)]
    thresholds: monitors::Thresholds,
}

/// What a query evaluates to right now, per group (the editor's check).
pub async fn preview(State(st): State<AppState>, Json(p): Json<Preview>) -> ApiResult<Value> {
    let parsed = monitors::parse(&p.query).map_err(ApiError::BadQuery)?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let samples = monitors::evaluate(&st, &parsed, now).await?;
    let th = p.thresholds;
    Ok(Json(json!({
        "type": monitors::type_of(&parsed),
        "window_ms": parsed.window_ms,
        "comparator": parsed.cmp.symbol(),
        "threshold": parsed.threshold,
        "groups": samples.iter().map(|s| json!({
            "group": s.group,
            "value": s.value,
            "status": s.value.map(|v| monitors::next_status(&parsed, &th, monitors::OK, v)).unwrap_or(monitors::NO_DATA),
        })).collect::<Vec<_>>(),
    })))
}

// --- webhooks --------------------------------------------------------------------------

pub async fn webhooks(State(st): State<AppState>) -> ApiResult<Vec<WebhookDef>> {
    Ok(Json(
        st.store
            .pg
            .docs(webhooks::KIND)
            .await?
            .into_iter()
            .filter_map(|d| serde_json::from_value(d.body).ok())
            .collect(),
    ))
}

pub async fn webhook(
    State(st): State<AppState>,
    Path(name): Path<String>,
) -> ApiResult<WebhookDef> {
    webhooks::load(&st, &name)
        .await?
        .map(Json)
        .ok_or(ApiError::NotFound)
}

fn check_hook(h: &WebhookDef) -> Result<(), ApiError> {
    if !webhooks::valid_name(&h.name) {
        return Err(ApiError::BadQuery(
            "webhook names use letters, digits, _ - .".into(),
        ));
    }
    if !(h.url.starts_with("http://") || h.url.starts_with("https://")) {
        return Err(ApiError::BadQuery("the URL must be http(s)".into()));
    }
    if let Some(hd) = h.custom_headers.as_deref().filter(|s| !s.trim().is_empty())
        && serde_json::from_str::<Map<String, Value>>(hd).is_err()
    {
        return Err(ApiError::BadQuery(
            "custom_headers must be a JSON object".into(),
        ));
    }
    Ok(())
}

pub async fn create_webhook(
    State(st): State<AppState>,
    Json(h): Json<WebhookDef>,
) -> ApiResult<WebhookDef> {
    check_hook(&h)?;
    if webhooks::load(&st, &h.name).await?.is_some() {
        return Err(ApiError::BadQuery(format!(
            "a webhook named {} exists",
            h.name
        )));
    }
    st.store
        .pg
        .put_doc(
            webhooks::KIND,
            &h.name,
            &serde_json::to_value(&h).map_err(anyhow::Error::from)?,
        )
        .await?;
    Ok(Json(h))
}

pub async fn update_webhook(
    State(st): State<AppState>,
    Path(name): Path<String>,
    Json(patch): Json<Value>,
) -> ApiResult<WebhookDef> {
    let current = webhooks::load(&st, &name)
        .await?
        .ok_or(ApiError::NotFound)?;
    let mut merged = serde_json::to_value(&current).map_err(anyhow::Error::from)?;
    if let (Some(m), Value::Object(p)) = (merged.as_object_mut(), patch) {
        m.extend(p);
    }
    let h: WebhookDef =
        serde_json::from_value(merged).map_err(|e| ApiError::BadQuery(e.to_string()))?;
    check_hook(&h)?;
    if h.name != name {
        st.store.pg.delete_doc(webhooks::KIND, &name).await?;
    }
    st.store
        .pg
        .put_doc(
            webhooks::KIND,
            &h.name,
            &serde_json::to_value(&h).map_err(anyhow::Error::from)?,
        )
        .await?;
    Ok(Json(h))
}

pub async fn delete_webhook(
    State(st): State<AppState>,
    Path(name): Path<String>,
) -> ApiResult<Value> {
    if !st.store.pg.delete_doc(webhooks::KIND, &name).await? {
        return Err(ApiError::NotFound);
    }
    Ok(Json(json!({})))
}

/// Sends a sample notification to a webhook.
pub async fn test_webhook(
    State(st): State<AppState>,
    Path(name): Path<String>,
) -> ApiResult<Value> {
    let hook = webhooks::load(&st, &name)
        .await?
        .ok_or(ApiError::NotFound)?;
    let vars: HashMap<&str, String> = HashMap::from([
        (
            "EVENT_TITLE",
            "[Triggered] robine-dog test notification".to_string(),
        ),
        (
            "EVENT_MSG",
            "This is a test notification from robine-dog.".to_string(),
        ),
        (
            "TEXT_ONLY_MSG",
            "This is a test notification from robine-dog.".to_string(),
        ),
        ("ALERT_TRANSITION", "Triggered".to_string()),
        ("ALERT_TYPE", "error".to_string()),
        ("EVENT_TYPE", "test".to_string()),
        ("ORG_NAME", "robine-dog".to_string()),
        ("LINK", st.cfg.base_url.clone()),
    ]);
    let vars: Vec<(&str, String)> = vars.into_iter().collect();
    match webhooks::send(&hook, &vars).await {
        Ok(code) => Ok(Json(json!({"ok": true, "status": code}))),
        Err(e) => Ok(Json(json!({"ok": false, "error": e.to_string()}))),
    }
}
