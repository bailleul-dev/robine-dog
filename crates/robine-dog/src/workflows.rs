//! Workflow Automation and App Builder: workflows of generic steps (HTTP,
//! Kubernetes, robine-dog actions, approvals, pauses) started by a monitor
//! (`@workflow-<name>`), a security signal, a schedule (cron), the UI or the
//! API (`POST /api/v2/workflows/{id}/instances`). Apps are forms whose
//! buttons start workflows with what was typed. No vendor-specific action:
//! an HTTP step reaches any webhook (Slack, Discord, ntfy…).

use crate::AppState;
use crate::api::{ApiError, ApiResult};
use crate::store::pg::{Doc, new_id};
use axum::Json;
use axum::extract::{Path, Query, State};
use chrono::{Datelike, Timelike};
use regex::Regex;
use serde::Deserialize;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use std::time::Duration;

const KIND: &str = "workflow";
const APP: &str = "app";
pub const ACTIONS: &[&str] = &[
    "http",
    "k8s.restart",
    "k8s.scale",
    "k8s.delete_pod",
    "case.create",
    "downtime.create",
    "monitor.mute",
    "oncall.page",
    "approval",
    "sleep",
];

static HANDLE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"@workflow-([A-Za-z0-9_\-.]+)").unwrap());
static VAR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\{\{\s*([A-Za-z0-9_.\-\[\]]+)\s*(\|\s*json\s*)?\}\}").unwrap());
static CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .user_agent("robine-dog-workflows")
        .build()
        .expect("http client")
});

// ---------- templating and conditions ----------

/// The value at `a.b.0.c` in the context.
pub fn lookup<'a>(ctx: &'a Value, path: &str) -> Option<&'a Value> {
    let mut v = ctx;
    for part in path.split(['.', '[', ']']).filter(|p| !p.is_empty()) {
        v = match v {
            Value::Object(m) => m.get(part)?,
            Value::Array(a) => a.get(part.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(v)
}

/// `{{ trigger.status }}`, `{{ steps.call.body.id }}`, `{{ x | json }}`.
pub fn render(t: &str, ctx: &Value) -> String {
    VAR.replace_all(t, |c: &regex::Captures| {
        let v = lookup(ctx, &c[1]).cloned().unwrap_or(Value::Null);
        if c.get(2).is_some() {
            return v.to_string();
        }
        match v {
            Value::String(s) => s,
            Value::Null => String::new(),
            v => v.to_string(),
        }
    })
    .into_owned()
}

/// Renders every string of a JSON value.
pub fn render_value(v: &Value, ctx: &Value) -> Value {
    match v {
        Value::String(s) => Value::String(render(s, ctx)),
        Value::Array(a) => Value::Array(a.iter().map(|x| render_value(x, ctx)).collect()),
        Value::Object(m) => Value::Object(
            m.iter()
                .map(|(k, x)| (k.clone(), render_value(x, ctx)))
                .collect(),
        ),
        v => v.clone(),
    }
}

/// A step's `if`: rendered, then `a == b`, `a != b`, `a contains b`, or
/// truthy (not empty, `false`, `0` or `null`).
pub fn condition(expr: &str, ctx: &Value) -> bool {
    let r = render(expr, ctx);
    for (op, f) in [
        (
            " != ",
            (|a: &str, b: &str| a != b) as fn(&str, &str) -> bool,
        ),
        (" == ", |a, b| a == b),
        (" contains ", |a, b| a.contains(b)),
    ] {
        if let Some((a, b)) = r.split_once(op) {
            return f(a.trim(), b.trim());
        }
    }
    let t = r.trim();
    !(t.is_empty() || t == "false" || t == "0" || t == "null")
}

// ---------- cron ----------

fn field_matches(spec: &str, v: u32, min: u32, max: u32) -> bool {
    spec.split(',').any(|part| {
        let (range, step) = match part.split_once('/') {
            Some((r, s)) => (r, s.parse::<u32>().unwrap_or(1).max(1)),
            None => (part, 1),
        };
        let (lo, hi) = if range == "*" {
            (min, max)
        } else if let Some((a, b)) = range.split_once('-') {
            (a.parse().unwrap_or(u32::MAX), b.parse().unwrap_or(0))
        } else {
            let a = range.parse().unwrap_or(u32::MAX);
            (a, if step > 1 { max } else { a })
        };
        v >= lo && v <= hi && (v - lo).is_multiple_of(step)
    })
}

/// A 5-field cron expression (UTC): minute hour day-of-month month day-of-week.
pub fn cron_matches(expr: &str, t: chrono::DateTime<chrono::Utc>) -> bool {
    let f: Vec<&str> = expr.split_whitespace().collect();
    if f.len() != 5 {
        return false;
    }
    let dow = t.weekday().num_days_from_sunday();
    field_matches(f[0], t.minute(), 0, 59)
        && field_matches(f[1], t.hour(), 0, 23)
        && field_matches(f[2], t.day(), 1, 31)
        && field_matches(f[3], t.month(), 1, 12)
        && (field_matches(f[4], dow, 0, 7) || (dow == 0 && field_matches(f[4], 7, 0, 7)))
}

// ---------- validation ----------

fn validate(b: &Value) -> Result<(), ApiError> {
    let bad = |m: String| Err(ApiError::BadQuery(m));
    let name = b["name"].as_str().unwrap_or("");
    if !crate::monitors::webhooks::valid_name(name) {
        return bad(
            "name: letters, digits, `_`, `-` or `.` (it is the @workflow-<name> handle)".into(),
        );
    }
    let Some(steps) = b["steps"].as_array() else {
        return bad("steps: a list".into());
    };
    let mut names = std::collections::HashSet::new();
    for (i, s) in steps.iter().enumerate() {
        let a = s["action"].as_str().unwrap_or("");
        if !ACTIONS.contains(&a) {
            return bad(format!(
                "step {}: action is one of {}",
                i + 1,
                ACTIONS.join(", ")
            ));
        }
        let n = s["name"].as_str().unwrap_or("");
        if n.is_empty() || !names.insert(n.to_string()) {
            return bad(format!("step {}: a unique name is required", i + 1));
        }
    }
    if let Some(c) = b["triggers"]["schedule"]
        .as_str()
        .filter(|c| !c.trim().is_empty())
        && c.split_whitespace().count() != 5
    {
        return bad("schedule: a 5-field cron expression (UTC), e.g. `*/15 * * * *`".into());
    }
    Ok(())
}

// ---------- actions ----------

fn kube() -> Result<(String, String, reqwest::Client), String> {
    let host = std::env::var("KUBERNETES_SERVICE_HOST")
        .map_err(|_| "Kubernetes actions need robine-dog to run in a cluster".to_string())?;
    let port = std::env::var("KUBERNETES_SERVICE_PORT").unwrap_or_else(|_| "443".into());
    let dir = "/var/run/secrets/kubernetes.io/serviceaccount";
    let token =
        std::fs::read_to_string(format!("{dir}/token")).map_err(|e| format!("token: {e}"))?;
    let ca = std::fs::read(format!("{dir}/ca.crt")).map_err(|e| format!("ca.crt: {e}"))?;
    let client = reqwest::Client::builder()
        .add_root_certificate(reqwest::Certificate::from_pem(&ca).map_err(|e| e.to_string())?)
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())?;
    let host = if host.contains(':') {
        format!("[{host}]")
    } else {
        host
    };
    Ok((
        format!("https://{host}:{port}"),
        token.trim().to_string(),
        client,
    ))
}

async fn kube_call(
    method: reqwest::Method,
    path: &str,
    ctype: &str,
    body: Option<Value>,
) -> Result<Value, String> {
    let (base, token, client) = kube()?;
    let mut r = client
        .request(method, format!("{base}{path}"))
        .bearer_auth(token);
    if let Some(b) = body {
        r = r.header("content-type", ctype).body(b.to_string());
    }
    let res = r.send().await.map_err(|e| e.to_string())?;
    let status = res.status();
    let v: Value = res.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        return Err(format!(
            "Kubernetes {status}: {}",
            v["message"].as_str().unwrap_or("")
        ));
    }
    Ok(json!({"status": status.as_u16(), "kind": v["kind"], "name": v["metadata"]["name"]}))
}

fn workload(p: &Value) -> Result<(String, String, String), String> {
    let ns = p["namespace"].as_str().unwrap_or("default").to_string();
    let kind = p["kind"].as_str().unwrap_or("deployment").to_lowercase();
    if !matches!(kind.as_str(), "deployment" | "statefulset" | "daemonset") {
        return Err("kind: deployment, statefulset or daemonset".into());
    }
    let name = p["name"]
        .as_str()
        .filter(|n| !n.is_empty())
        .ok_or("name is required")?;
    Ok((ns, kind, name.to_string()))
}

fn api_err(e: ApiError) -> String {
    match e {
        ApiError::BadQuery(m) => m,
        e => format!("{e:?}"),
    }
}

/// Runs one step; its output, or why it failed.
async fn act(st: &AppState, action: &str, p: &Value) -> Result<Value, String> {
    let now = crate::intake::logs::now_ms();
    let s = |k: &str| p[k].as_str().unwrap_or("").to_string();
    match action {
        "http" => {
            let method = reqwest::Method::from_bytes(s("method").to_uppercase().as_bytes())
                .unwrap_or(reqwest::Method::POST);
            let url = s("url");
            if !url.starts_with("http://") && !url.starts_with("https://") {
                return Err("url: http:// or https://".into());
            }
            let mut r = CLIENT.request(method, &url);
            for (k, v) in p["headers"].as_object().into_iter().flatten() {
                r = r.header(
                    k,
                    v.as_str()
                        .map(String::from)
                        .unwrap_or_else(|| v.to_string()),
                );
            }
            match &p["body"] {
                Value::Null => {}
                Value::String(b) => {
                    if !p["headers"]
                        .as_object()
                        .is_some_and(|h| h.keys().any(|k| k.eq_ignore_ascii_case("content-type")))
                    {
                        let ct = if serde_json::from_str::<Value>(b).is_ok() {
                            "application/json"
                        } else {
                            "text/plain"
                        };
                        r = r.header("content-type", ct);
                    }
                    r = r.body(b.clone());
                }
                b => r = r.json(b),
            }
            let res = r.send().await.map_err(|e| e.to_string())?;
            let status = res.status().as_u16();
            let text = res.text().await.unwrap_or_default();
            let body = serde_json::from_str::<Value>(&text)
                .unwrap_or_else(|_| Value::String(text.chars().take(10_000).collect()));
            let out = json!({"status": status, "body": body});
            if status >= 400 && !p["allow_errors"].as_bool().unwrap_or(false) {
                return Err(format!("HTTP {status}"));
            }
            Ok(out)
        }
        "k8s.restart" => {
            let (ns, kind, name) = workload(p)?;
            let patch = json!({"spec": {"template": {"metadata": {"annotations": {
                "kubectl.kubernetes.io/restartedAt": chrono::Utc::now().to_rfc3339()}}}}});
            kube_call(
                reqwest::Method::PATCH,
                &format!("/apis/apps/v1/namespaces/{ns}/{kind}s/{name}"),
                "application/strategic-merge-patch+json",
                Some(patch),
            )
            .await
        }
        "k8s.scale" => {
            let (ns, kind, name) = workload(p)?;
            let replicas = p["replicas"]
                .as_i64()
                .or_else(|| s("replicas").parse().ok())
                .ok_or("replicas is required")?;
            kube_call(
                reqwest::Method::PATCH,
                &format!("/apis/apps/v1/namespaces/{ns}/{kind}s/{name}/scale"),
                "application/merge-patch+json",
                Some(json!({"spec": {"replicas": replicas}})),
            )
            .await
        }
        "k8s.delete_pod" => {
            let ns = p["namespace"].as_str().unwrap_or("default");
            let name = p["name"]
                .as_str()
                .filter(|n| !n.is_empty())
                .ok_or("name is required")?;
            kube_call(
                reqwest::Method::DELETE,
                &format!("/api/v1/namespaces/{ns}/pods/{name}"),
                "",
                None,
            )
            .await
        }
        "case.create" => {
            let Json(v) = crate::cases::create(State(st.clone()), None, Json(p.clone()))
                .await
                .map_err(api_err)?;
            Ok(json!({"id": v["data"]["id"], "key": v["data"]["attributes"]["key"]}))
        }
        "downtime.create" => {
            let minutes = p["minutes"]
                .as_i64()
                .or_else(|| s("minutes").parse().ok())
                .unwrap_or(60);
            let mut d = p.clone();
            d["end"] = json!(now / 1000 + minutes * 60);
            if let Some(m) = s("monitor_id")
                .parse::<i64>()
                .ok()
                .or(p["monitor_id"].as_i64())
            {
                d["monitor_id"] = json!(m);
            }
            let d = serde_json::from_value(d).map_err(|e| e.to_string())?;
            let Json(v) = crate::api::downtimes::create(State(st.clone()), Json(d))
                .await
                .map_err(api_err)?;
            Ok(json!({"id": v.id, "end": v.end}))
        }
        "monitor.mute" => {
            let id = p["monitor_id"]
                .as_i64()
                .map(|i| i.to_string())
                .unwrap_or_else(|| s("monitor_id"));
            let mut body = json!({"scope": p.get("scope").and_then(|x| x.as_str()).filter(|x| !x.is_empty()).unwrap_or("*")});
            if let Some(m) = p["minutes"].as_i64().or_else(|| s("minutes").parse().ok()) {
                body["end"] = json!(now / 1000 + m * 60);
            }
            let m = serde_json::from_value(body).map_err(|e| e.to_string())?;
            let _ = crate::api::monitors::mute(State(st.clone()), Path(id.clone()), Some(Json(m)))
                .await
                .map_err(api_err)?;
            Ok(json!({"muted": id}))
        }
        "oncall.page" => {
            let Json(v) = crate::oncall::create_page(State(st.clone()), Json(p.clone()))
                .await
                .map_err(api_err)?;
            Ok(json!({"id": v["data"]["id"], "status": v["data"]["status"]}))
        }
        "sleep" => {
            let secs = p["seconds"]
                .as_u64()
                .or_else(|| s("seconds").parse().ok())
                .unwrap_or(1)
                .min(3600);
            tokio::time::sleep(Duration::from_secs(secs)).await;
            Ok(json!({"slept": secs}))
        }
        a => Err(format!("unknown action {a}")),
    }
}

// ---------- runs ----------

/// Starts a workflow; the run goes on in the background. Its id.
pub async fn start(
    st: &AppState,
    wf: &Doc,
    trigger_type: &str,
    trigger: Value,
) -> anyhow::Result<i64> {
    let ctx = json!({"trigger": trigger, "inputs": trigger, "steps": {},
        "workflow": {"id": wf.id, "name": wf.body["name"]}});
    let c = st.store.pg.conn().await?;
    let row = c
        .query_one(
            "INSERT INTO workflow_runs (workflow_id, workflow_name, trigger_type, status, context, steps, next_step, started_at)
             VALUES ($1, $2, $3, 'running', $4, '[]'::jsonb, 0, $5) RETURNING id",
            &[&wf.id, &wf.body["name"].as_str().unwrap_or(""), &trigger_type, &ctx, &crate::intake::logs::now_ms()],
        )
        .await?;
    let id: i64 = row.get(0);
    let (st, wf) = (st.clone(), wf.clone());
    tokio::spawn(async move { proceed(&st, &wf, id).await });
    Ok(id)
}

async fn save(
    st: &AppState,
    id: i64,
    status: &str,
    ctx: &Value,
    steps: &Value,
    next: i32,
    error: Option<&str>,
) {
    let done =
        matches!(status, "succeeded" | "failed" | "rejected").then(crate::intake::logs::now_ms);
    let r = async {
        st.store.pg.conn().await?
            .execute(
                "UPDATE workflow_runs SET status = $2, context = $3, steps = $4, next_step = $5, error = $6,
                    finished_at = coalesce($7, finished_at) WHERE id = $1",
                &[&id, &status, ctx, steps, &next, &error, &done],
            )
            .await?;
        anyhow::Ok(())
    }
    .await;
    if let Err(e) = r {
        tracing::warn!(error = %e, run = id, "saving a workflow run");
    }
}

/// Runs the steps from `next_step` until the end, a failure or an approval.
async fn proceed(st: &AppState, wf: &Doc, id: i64) {
    let Ok(c) = st.store.pg.conn().await else {
        return;
    };
    let Ok(row) = c
        .query_one(
            "SELECT context, steps, next_step FROM workflow_runs WHERE id = $1",
            &[&id],
        )
        .await
    else {
        return;
    };
    drop(c);
    let (mut ctx, mut log, next): (Value, Value, i32) = (row.get(0), row.get(1), row.get(2));
    let steps = wf.body["steps"].as_array().cloned().unwrap_or_default();
    for (i, step) in steps.iter().enumerate().skip(next as usize) {
        let name = step["name"].as_str().unwrap_or("step").to_string();
        let action = step["action"].as_str().unwrap_or("");
        let started = crate::intake::logs::now_ms();
        let mut entry = json!({"name": name, "action": action, "started_at": started});
        if let Some(cond) = step["if"].as_str().filter(|c| !c.trim().is_empty())
            && !condition(cond, &ctx)
        {
            entry["status"] = json!("skipped");
            log.as_array_mut().unwrap().push(entry);
            continue;
        }
        let params = render_value(&step["params"], &ctx);
        if action == "approval" {
            entry["status"] = json!("waiting");
            entry["message"] = params["message"].clone();
            log.as_array_mut().unwrap().push(entry);
            save(st, id, "waiting_approval", &ctx, &log, i as i32, None).await;
            return;
        }
        let r = act(st, action, &params).await;
        entry["duration_ms"] = json!(crate::intake::logs::now_ms() - started);
        entry["params"] = params;
        match r {
            Ok(out) => {
                entry["status"] = json!("succeeded");
                entry["output"] = out.clone();
                ctx["steps"][&name] = out;
                log.as_array_mut().unwrap().push(entry);
                save(st, id, "running", &ctx, &log, i as i32 + 1, None).await;
            }
            Err(e) => {
                entry["status"] = json!("failed");
                entry["error"] = json!(e);
                log.as_array_mut().unwrap().push(entry);
                if step["continue_on_error"].as_bool().unwrap_or(false) {
                    ctx["steps"][&name] = json!({"error": e});
                    continue;
                }
                save(
                    st,
                    id,
                    "failed",
                    &ctx,
                    &log,
                    i as i32 + 1,
                    Some(&format!("{name}: {e}")),
                )
                .await;
                return;
            }
        }
    }
    save(st, id, "succeeded", &ctx, &log, steps.len() as i32, None).await;
}

fn run_view(r: &tokio_postgres::Row) -> Value {
    json!({"id": r.get::<_, i64>(0).to_string(), "workflow_id": r.get::<_, String>(1),
        "workflow_name": r.get::<_, String>(2), "trigger_type": r.get::<_, String>(3),
        "status": r.get::<_, String>(4), "steps": r.get::<_, Value>(5), "started_at": r.get::<_, i64>(6),
        "finished_at": r.get::<_, Option<i64>>(7), "error": r.get::<_, Option<String>>(8),
        "trigger": r.get::<_, Value>(9)["trigger"]})
}
const RUN_COLS: &str = "id, workflow_id, workflow_name, trigger_type, status, steps, started_at, finished_at, error, context";

// ---------- triggers ----------

async fn by_name(st: &AppState, name: &str) -> Option<Doc> {
    st.store
        .pg
        .docs(KIND)
        .await
        .ok()?
        .into_iter()
        .find(|d| d.body["name"].as_str() == Some(name))
}

fn enabled(d: &Doc) -> bool {
    d.body["enabled"].as_bool().unwrap_or(true)
}

/// From a monitor's notification: `@workflow-<name>` handles.
pub async fn notify(st: &AppState, message: &str, trigger: Value) -> Vec<String> {
    let mut sent = Vec::new();
    for c in HANDLE.captures_iter(message) {
        let name = &c[1];
        match by_name(st, name).await {
            Some(wf) if enabled(&wf) => match start(st, &wf, "monitor", trigger.clone()).await {
                Ok(_) => sent.push(format!("workflow:{name}")),
                Err(e) => tracing::warn!(error = %e, workflow = name, "starting a workflow"),
            },
            _ => tracing::warn!(workflow = name, "no enabled workflow by this name"),
        }
    }
    sent
}

/// A new security signal: workflows triggered by signals (all or that rule),
/// and handles in the rule's message.
pub async fn on_signal(st: &AppState, rule_id: &str, message: &str, trigger: Value) {
    let Ok(all) = st.store.pg.docs(KIND).await else {
        return;
    };
    for wf in all.iter().filter(|w| enabled(w)) {
        let t = &wf.body["triggers"];
        let by_trigger = t["security_signal"].as_bool().unwrap_or(false)
            && t["security_rule"]
                .as_str()
                .is_none_or(|r| r.is_empty() || r == rule_id);
        let named = wf.body["name"]
            .as_str()
            .is_some_and(|n| message.contains(&format!("@workflow-{n}")));
        if by_trigger || named {
            let _ = start(st, wf, "security_signal", trigger.clone()).await;
        }
    }
}

/// Scheduled workflows, checked at each minute.
pub fn spawn(st: AppState) {
    tokio::spawn(async move {
        let mut last = -1i64;
        loop {
            tokio::time::sleep(Duration::from_secs(5)).await;
            let now = chrono::Utc::now();
            let minute = now.timestamp() / 60;
            if minute == last {
                continue;
            }
            last = minute;
            let Ok(all) = st.store.pg.docs(KIND).await else {
                continue;
            };
            for wf in all.iter().filter(|w| enabled(w)) {
                if let Some(c) = wf.body["triggers"]["schedule"].as_str()
                    && cron_matches(c, now)
                {
                    let _ = start(
                        &st,
                        wf,
                        "schedule",
                        json!({"scheduled_at": now.timestamp_millis()}),
                    )
                    .await;
                }
            }
        }
    });
}

// ---------- API ----------

fn view(d: &Doc) -> Value {
    json!({"id": d.id, "type": "workflows", "attributes": d.body,
        "meta": {"created_at": d.created_at, "updated_at": d.updated_at}})
}

fn body_of(v: Value) -> Value {
    let a = v["data"]["attributes"].clone();
    if a.is_object() { a } else { v }
}

pub async fn list(State(st): State<AppState>) -> ApiResult<Value> {
    let docs = st.store.pg.docs(KIND).await?;
    Ok(Json(
        json!({"data": docs.iter().map(view).collect::<Vec<_>>(), "actions": ACTIONS}),
    ))
}

pub async fn get(State(st): State<AppState>, Path(id): Path<String>) -> ApiResult<Value> {
    let d = st
        .store
        .pg
        .doc(KIND, &id)
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(json!({"data": view(&d)})))
}

async fn put(st: &AppState, id: &str, v: Value) -> ApiResult<Value> {
    let mut b = body_of(v);
    if b["steps"].is_null() {
        b["steps"] = json!([]);
    }
    validate(&b)?;
    let others = st.store.pg.docs(KIND).await?;
    if others
        .iter()
        .any(|d| d.id != id && d.body["name"] == b["name"])
    {
        return Err(ApiError::BadQuery(
            "a workflow already has this name".into(),
        ));
    }
    let d = st.store.pg.put_doc(KIND, id, &b).await?;
    Ok(Json(json!({"data": view(&d)})))
}

pub async fn create(State(st): State<AppState>, Json(v): Json<Value>) -> ApiResult<Value> {
    put(&st, &new_id(), v).await
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
    put(&st, &id, v).await
}

pub async fn delete(State(st): State<AppState>, Path(id): Path<String>) -> ApiResult<Value> {
    st.store.pg.delete_doc(KIND, &id).await?;
    Ok(Json(json!({})))
}

/// `POST /api/v2/workflows/{id}/instances`: `{"meta": {"payload": {...}}}`.
pub async fn run(
    State(st): State<AppState>,
    Path(id): Path<String>,
    body: Option<Json<Value>>,
) -> ApiResult<Value> {
    let wf = st
        .store
        .pg
        .doc(KIND, &id)
        .await?
        .ok_or(ApiError::NotFound)?;
    let v = body.map(|b| b.0).unwrap_or(Value::Null);
    let payload = v["meta"]["payload"].clone();
    let payload = if payload.is_null() {
        v.get("payload").cloned().unwrap_or(json!({}))
    } else {
        payload
    };
    let rid = start(&st, &wf, "api", payload).await?;
    Ok(Json(json!({"data": {"id": rid.to_string()}})))
}

#[derive(Debug, Deserialize)]
pub struct RunFilter {
    pub workflow_id: Option<String>,
}

pub async fn runs(State(st): State<AppState>, Query(f): Query<RunFilter>) -> ApiResult<Value> {
    let c = st.store.pg.conn().await?;
    let rows = c
        .query(
            &format!("SELECT {RUN_COLS} FROM workflow_runs WHERE ($1::TEXT IS NULL OR workflow_id = $1) ORDER BY id DESC LIMIT 200"),
            &[&f.workflow_id],
        )
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    Ok(Json(
        json!({"data": rows.iter().map(run_view).collect::<Vec<_>>()}),
    ))
}

/// `GET /api/v2/workflows/{id}/instances` (Datadog's path).
pub async fn instances(st: State<AppState>, Path(id): Path<String>) -> ApiResult<Value> {
    runs(
        st,
        Query(RunFilter {
            workflow_id: Some(id),
        }),
    )
    .await
}

async fn one_run(st: &AppState, rid: i64) -> Result<Value, ApiError> {
    let c = st.store.pg.conn().await?;
    let r = c
        .query_opt(
            &format!("SELECT {RUN_COLS} FROM workflow_runs WHERE id = $1"),
            &[&rid],
        )
        .await
        .map_err(|e| ApiError::Internal(e.into()))?
        .ok_or(ApiError::NotFound)?;
    Ok(run_view(&r))
}

pub async fn instance(
    State(st): State<AppState>,
    Path((_, rid)): Path<(String, String)>,
) -> ApiResult<Value> {
    let rid = rid.parse::<i64>().map_err(|_| ApiError::NotFound)?;
    Ok(Json(json!({"data": one_run(&st, rid).await?})))
}

pub async fn run_one(State(st): State<AppState>, Path(rid): Path<String>) -> ApiResult<Value> {
    let rid = rid.parse::<i64>().map_err(|_| ApiError::NotFound)?;
    Ok(Json(json!({"data": one_run(&st, rid).await?})))
}

/// `POST /workflows/runs/{id}/{approve|reject}`: answers a waiting approval.
pub async fn decide(
    State(st): State<AppState>,
    Path((rid, what)): Path<(String, String)>,
    who: Option<axum::Extension<crate::auth::Identity>>,
) -> ApiResult<Value> {
    let rid = rid.parse::<i64>().map_err(|_| ApiError::NotFound)?;
    let run = one_run(&st, rid).await?;
    if run["status"] != "waiting_approval" {
        return Err(ApiError::BadQuery(
            "this run is not waiting for an approval".into(),
        ));
    }
    let author = who
        .map(|axum::Extension(i)| i.email.unwrap_or(i.sub))
        .unwrap_or_else(|| "api".into());
    let c = st.store.pg.conn().await?;
    let row = c
        .query_one(
            "SELECT context, steps, next_step, workflow_id FROM workflow_runs WHERE id = $1",
            &[&rid],
        )
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    let (ctx, mut log, next, wid): (Value, Value, i32, String) =
        (row.get(0), row.get(1), row.get(2), row.get(3));
    let approved = match what.as_str() {
        "approve" => true,
        "reject" => false,
        _ => return Err(ApiError::NotFound),
    };
    if let Some(last) = log.as_array_mut().and_then(|a| a.last_mut()) {
        last["status"] = json!(if approved { "approved" } else { "rejected" });
        last["by"] = json!(author);
    }
    if !approved {
        save(&st, rid, "rejected", &ctx, &log, next + 1, Some("rejected")).await;
    } else {
        save(&st, rid, "running", &ctx, &log, next + 1, None).await;
        let wf = st
            .store
            .pg
            .doc(KIND, &wid)
            .await?
            .ok_or(ApiError::NotFound)?;
        let st2 = st.clone();
        tokio::spawn(async move { proceed(&st2, &wf, rid).await });
    }
    Ok(Json(json!({"data": one_run(&st, rid).await?})))
}

// ---------- App Builder ----------

fn app_view(d: &Doc) -> Value {
    json!({"id": d.id, "type": "appDefinitions", "attributes": d.body})
}

pub async fn apps(State(st): State<AppState>) -> ApiResult<Value> {
    let docs = st.store.pg.docs(APP).await?;
    Ok(Json(
        json!({"data": docs.iter().map(app_view).collect::<Vec<_>>()}),
    ))
}

async fn put_app(st: &AppState, id: &str, v: Value) -> ApiResult<Value> {
    let b = body_of(v);
    if b["name"].as_str().unwrap_or("").trim().is_empty() {
        return Err(ApiError::BadQuery("an app needs a name".into()));
    }
    for btn in b["buttons"].as_array().into_iter().flatten() {
        let wid = btn["workflow_id"].as_str().unwrap_or("");
        if st.store.pg.doc(KIND, wid).await?.is_none() {
            return Err(ApiError::BadQuery(format!(
                "button {}: no workflow {wid}",
                btn["label"]
            )));
        }
    }
    let d = st.store.pg.put_doc(APP, id, &b).await?;
    Ok(Json(json!({"data": app_view(&d)})))
}

pub async fn create_app(State(st): State<AppState>, Json(v): Json<Value>) -> ApiResult<Value> {
    put_app(&st, &new_id(), v).await
}

pub async fn update_app(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Json(v): Json<Value>,
) -> ApiResult<Value> {
    st.store.pg.doc(APP, &id).await?.ok_or(ApiError::NotFound)?;
    put_app(&st, &id, v).await
}

pub async fn delete_app(State(st): State<AppState>, Path(id): Path<String>) -> ApiResult<Value> {
    st.store.pg.delete_doc(APP, &id).await?;
    Ok(Json(json!({})))
}

/// `POST /app-builder/apps/{id}/run`: `{"button": 0, "values": {...}}`.
pub async fn run_app(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Json(v): Json<Value>,
) -> ApiResult<Value> {
    let app = st.store.pg.doc(APP, &id).await?.ok_or(ApiError::NotFound)?;
    let i = v["button"].as_u64().unwrap_or(0) as usize;
    let wid = app.body["buttons"][i]["workflow_id"]
        .as_str()
        .ok_or(ApiError::NotFound)?;
    let wf = st
        .store
        .pg
        .doc(KIND, wid)
        .await?
        .ok_or(ApiError::NotFound)?;
    let values = v["values"].as_object().cloned().unwrap_or_else(Map::new);
    let rid = start(
        &st,
        &wf,
        "app",
        json!({"app": app.body["name"], "values": values}),
    )
    .await?;
    Ok(Json(json!({"data": {"id": rid.to_string()}})))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn templates_and_conditions() {
        let ctx = json!({"trigger": {"status": "Alert", "host": "web-1"}, "steps": {"call": {"body": {"items": [{"id": 7}]}}}});
        assert_eq!(
            render(
                "{{ trigger.host }} / {{steps.call.body.items[0].id}} / {{ nope }}",
                &ctx
            ),
            "web-1 / 7 / "
        );
        assert_eq!(
            render("{\"h\": {{ trigger.host | json }}}", &ctx),
            "{\"h\": \"web-1\"}"
        );
        assert!(condition("{{ trigger.status }} == Alert", &ctx));
        assert!(!condition("{{ trigger.status }} != Alert", &ctx));
        assert!(condition("{{ trigger.host }} contains web", &ctx));
        assert!(!condition("{{ trigger.none }}", &ctx));
        assert_eq!(
            render_value(&json!({"a": ["{{trigger.host}}", 1]}), &ctx),
            json!({"a": ["web-1", 1]})
        );
    }

    #[test]
    fn cron() {
        let t = chrono::Utc
            .with_ymd_and_hms(2026, 11, 22, 9, 30, 0)
            .unwrap(); // a Sunday
        assert!(cron_matches("*/15 * * * *", t));
        assert!(cron_matches("30 9 * * 0", t));
        assert!(cron_matches("30 9 * * 7", t));
        assert!(cron_matches("0-45/10 8-10 22 11 *", t));
        assert!(!cron_matches("30 9 * * 1-5", t));
        assert!(!cron_matches("31 * * * *", t));
        assert!(!cron_matches("* * *", t));
    }

    #[test]
    fn validation() {
        assert!(
            validate(&json!({"name": "restart-api", "steps": [{"name": "a", "action": "http"}]}))
                .is_ok()
        );
        assert!(validate(&json!({"name": "bad name", "steps": []})).is_err());
        assert!(
            validate(&json!({"name": "x", "steps": [{"name": "a", "action": "slack"}]})).is_err()
        );
        assert!(validate(&json!({"name": "x", "steps": [{"name": "a", "action": "http"}, {"name": "a", "action": "sleep"}]})).is_err());
        assert!(
            validate(&json!({"name": "x", "steps": [], "triggers": {"schedule": "* *"}})).is_err()
        );
    }
}
