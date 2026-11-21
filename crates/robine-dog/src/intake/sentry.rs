//! Sentry-compatible intake: an application keeps its Sentry SDK and points
//! its DSN here, as Datadog's Error Tracking accepts it. Each error event
//! becomes an error log with `error.kind`, `error.message` and `error.stack`,
//! which is what Error Tracking groups into issues.
//!
//! - `POST /api/{project}/envelope/` (current SDKs) and
//!   `POST /api/{project}/store/` (older ones), gzip or deflate;
//! - the same under `/sentry/{service}/`, so the DSN names the service:
//!   `https://<client token>@<host>/sentry/<service>/1`. Otherwise the
//!   service is the event's `service` tag, or `sentry-<project>`;
//! - the DSN's key is the client token (`ROBINE_DOG_CLIENT_TOKEN`, safe to
//!   ship in a browser or a mobile app) or the API key.
//!
//! Mapping: `environment` → `env`, `release` → `version`, `server_name` →
//! host, `platform` → source, `user` → `usr.*`, `request` → `http.*`, tags →
//! tags, `contexts.trace` → the log's trace and span ids. Transactions,
//! sessions, attachments and the other envelope items are accepted and
//! dropped.

use super::logs::{
    LogRow, decompress, normalize_status, normalize_trace_id, now_ms, parse_timestamp_ms,
};
use crate::AppState;
use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};
use std::collections::HashMap;

const MAX_BREADCRUMBS: usize = 50;

/// Browsers post from any origin.
pub fn cors(mut r: Response) -> Response {
    let h = r.headers_mut();
    h.insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_static("*"),
    );
    h.insert(
        header::ACCESS_CONTROL_ALLOW_HEADERS,
        HeaderValue::from_static(
            "content-type, content-encoding, x-sentry-auth, sentry-trace, baggage, dd-api-key",
        ),
    );
    h.insert(
        header::ACCESS_CONTROL_ALLOW_METHODS,
        HeaderValue::from_static("POST, OPTIONS"),
    );
    r
}

pub async fn preflight() -> Response {
    cors(StatusCode::NO_CONTENT.into_response())
}

/// The public key of a DSN, wherever the SDK put it.
fn sentry_key(
    headers: &HeaderMap,
    q: &HashMap<String, String>,
    envelope: Option<&Value>,
) -> Option<String> {
    if let Some(k) = q.get("sentry_key") {
        return Some(k.clone());
    }
    let auth = headers
        .get("x-sentry-auth")
        .or_else(|| headers.get(header::AUTHORIZATION))
        .and_then(|v| v.to_str().ok());
    if let Some(k) = auth.and_then(|a| {
        a.trim_start_matches("Sentry ")
            .split(',')
            .find_map(|p| p.trim().strip_prefix("sentry_key=").map(String::from))
    }) {
        return Some(k);
    }
    // `"dsn": "https://<key>@host/1"` in the envelope header.
    let dsn = envelope?.get("dsn")?.as_str()?;
    let user = dsn.split_once("://")?.1.split_once('@')?.0;
    Some(user.split(':').next().unwrap_or(user).to_string())
}

pub fn authorized(st: &AppState, key: Option<&str>) -> bool {
    let known = [&st.cfg.client_token, &st.cfg.api_key];
    if known.iter().all(|k| k.is_none()) {
        return true;
    }
    key.is_some_and(|k| known.iter().any(|x| x.as_deref() == Some(k)))
}

/// An envelope: a JSON header line, then items, each a JSON header line and
/// a payload (`length` bytes, or up to the end of the line).
fn envelope_items(raw: &[u8]) -> (Option<Value>, Vec<(String, Vec<u8>)>) {
    let line_end = |from: usize| {
        raw[from..]
            .iter()
            .position(|b| *b == b'\n')
            .map_or(raw.len(), |i| from + i)
    };
    let end = line_end(0);
    let header = serde_json::from_slice::<Value>(&raw[..end]).ok();
    let mut items = Vec::new();
    let mut pos = (end + 1).min(raw.len());
    while pos < raw.len() {
        let end = line_end(pos);
        let Ok(item) = serde_json::from_slice::<Value>(&raw[pos..end]) else {
            pos = end + 1;
            continue;
        };
        pos = (end + 1).min(raw.len());
        let stop = match item.get("length").and_then(Value::as_u64) {
            Some(n) => (pos + n as usize).min(raw.len()),
            None => line_end(pos),
        };
        let typ = item
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        items.push((typ, raw[pos..stop].to_vec()));
        pos = stop + 1;
    }
    (header, items)
}

fn text(v: Option<&Value>) -> Option<String> {
    match v? {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// `exception: {values: [...]}` or, from old SDKs, `exception: [...]`.
fn exceptions(event: &Value) -> Vec<&Value> {
    let e = event.get("exception");
    e.and_then(|e| e.get("values"))
        .or(e)
        .and_then(Value::as_array)
        .map(|a| a.iter().collect())
        .unwrap_or_default()
}

/// Frames as text, the crashing one first (Sentry lists it last).
fn frames(stacktrace: Option<&Value>, out: &mut String) {
    let Some(frames) = stacktrace
        .and_then(|s| s.get("frames"))
        .and_then(Value::as_array)
    else {
        return;
    };
    for f in frames.iter().rev() {
        let function = text(f.get("function")).unwrap_or_else(|| "<unknown>".into());
        let file = text(f.get("filename"))
            .or_else(|| text(f.get("abs_path")))
            .or_else(|| text(f.get("module")))
            .or_else(|| text(f.get("package")));
        out.push_str("    at ");
        out.push_str(&function);
        if let Some(file) = file {
            out.push_str(" (");
            out.push_str(&file);
            for k in ["lineno", "colno"] {
                if let Some(n) = f.get(k).and_then(Value::as_i64) {
                    out.push(':');
                    out.push_str(&n.to_string());
                }
            }
            out.push(')');
        }
        out.push('\n');
    }
}

fn headline(typ: Option<&str>, value: Option<&str>) -> String {
    match (typ, value) {
        (Some(t), Some(v)) => format!("{t}: {v}"),
        (Some(t), None) => t.to_string(),
        (None, Some(v)) => v.to_string(),
        (None, None) => "Error".into(),
    }
}

/// Tags are an object, or a list of `[key, value]` pairs.
fn tag_pairs(v: Option<&Value>) -> Vec<(String, String)> {
    match v {
        Some(Value::Object(m)) => m
            .iter()
            .filter_map(|(k, v)| Some((k.clone(), text(Some(v))?)))
            .collect(),
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(|p| Some((text(p.get(0))?, text(p.get(1))?)))
            .collect(),
        _ => vec![],
    }
}

/// One Sentry event as a log. `service` is the one the DSN's path names.
pub fn event_to_log(event: &Value, project: &str, service: Option<&str>, now: i64) -> LogRow {
    let mut attrs = Map::new();
    let mut tags: Vec<String> = Vec::new();
    let pairs = tag_pairs(event.get("tags"));

    let level = text(event.get("level")).unwrap_or_else(|| "error".into());
    let status = normalize_status(&level).to_string();
    let is_error = matches!(
        status.as_str(),
        "error" | "critical" | "alert" | "emergency"
    );

    // The exception thrown is the last of the chain; its causes come before.
    let chain = exceptions(event);
    let mut error = Map::new();
    let message = if let Some(top) = chain.last() {
        let (typ, value) = (text(top.get("type")), text(top.get("value")));
        let mut stack = String::new();
        for (i, e) in chain.iter().rev().enumerate() {
            let line = headline(
                text(e.get("type")).as_deref(),
                text(e.get("value")).as_deref(),
            );
            if i > 0 {
                stack.push_str("Caused by: ");
            }
            stack.push_str(&line);
            stack.push('\n');
            frames(e.get("stacktrace"), &mut stack);
        }
        error.insert(
            "kind".into(),
            json!(typ.clone().unwrap_or_else(|| "Error".into())),
        );
        if let Some(v) = &value {
            error.insert("message".into(), json!(v));
        }
        if stack.contains("\n    at ") {
            error.insert("stack".into(), json!(stack.trim_end()));
        }
        if let Some(h) = top.pointer("/mechanism/handled").and_then(Value::as_bool) {
            error.insert("handled".into(), json!(h));
        }
        headline(typ.as_deref(), value.as_deref())
    } else {
        // `capture_message`: a string, or `{formatted, message, params}`.
        let m = ["message", "logentry"]
            .iter()
            .filter_map(|k| event.get(*k))
            .find_map(|m| {
                text(Some(m))
                    .or_else(|| text(m.get("formatted")))
                    .or_else(|| text(m.get("message")))
            })
            .unwrap_or_else(|| "Sentry event".into());
        if is_error {
            // An issue of its own in Error Tracking, as in Sentry.
            error.insert("kind".into(), json!("Message"));
            error.insert("message".into(), json!(m));
            let mut stack = format!("{m}\n");
            let thread = event
                .pointer("/threads/values")
                .and_then(Value::as_array)
                .and_then(|t| {
                    t.iter()
                        .find(|t| t.get("stacktrace").is_some_and(|s| !s.is_null()))
                });
            frames(
                event
                    .get("stacktrace")
                    .or_else(|| thread.and_then(|t| t.get("stacktrace"))),
                &mut stack,
            );
            if stack.contains("\n    at ") {
                error.insert("stack".into(), json!(stack.trim_end()));
            }
        }
        m
    };
    if !error.is_empty() {
        attrs.insert("error".into(), Value::Object(error));
    }

    let mut sentry = Map::new();
    sentry.insert("project".into(), json!(project));
    for (k, path) in [
        ("event_id", "/event_id"),
        ("platform", "/platform"),
        ("transaction", "/transaction"),
        ("logger", "/logger"),
        ("sdk_name", "/sdk/name"),
        ("sdk_version", "/sdk/version"),
        ("dist", "/dist"),
    ] {
        if let Some(v) = text(event.pointer(path)) {
            sentry.insert(k.into(), json!(v));
        }
    }
    if let Some(b) = event
        .pointer("/breadcrumbs/values")
        .or_else(|| event.get("breadcrumbs"))
        .and_then(Value::as_array)
        .filter(|b| !b.is_empty())
    {
        let from = b.len().saturating_sub(MAX_BREADCRUMBS);
        sentry.insert("breadcrumbs".into(), json!(b[from..]));
    }
    attrs.insert("sentry".into(), Value::Object(sentry));

    if let Some(u) = event.get("user").and_then(Value::as_object) {
        let mut usr = Map::new();
        for (from, to) in [("id", "id"), ("email", "email"), ("username", "name")] {
            if let Some(v) = text(u.get(from)) {
                usr.insert(to.into(), json!(v));
            }
        }
        if !usr.is_empty() {
            attrs.insert("usr".into(), Value::Object(usr));
        }
        if let Some(ip) = text(u.get("ip_address")).filter(|ip| ip != "{{auto}}") {
            attrs.insert("network".into(), json!({"client": {"ip": ip}}));
        }
    }
    if let Some(r) = event.get("request").and_then(Value::as_object) {
        let mut http = Map::new();
        for (from, to) in [("method", "method"), ("url", "url")] {
            if let Some(v) = text(r.get(from)) {
                http.insert(to.into(), json!(v));
            }
        }
        let agent = r.get("headers").and_then(|h| {
            tag_pairs(Some(h))
                .into_iter()
                .find(|(k, _)| k.eq_ignore_ascii_case("user-agent"))
        });
        if let Some((_, ua)) = agent {
            http.insert("useragent".into(), json!(ua));
        }
        if !http.is_empty() {
            attrs.insert("http".into(), Value::Object(http));
        }
    }
    for k in ["contexts", "extra"] {
        if let Some(v) = event
            .get(k)
            .filter(|v| v.as_object().is_some_and(|o| !o.is_empty()))
        {
            attrs.insert(k.into(), v.clone());
        }
    }

    let mut service = service.map(String::from);
    for (k, v) in pairs {
        if k == "service" {
            service.get_or_insert(v);
        } else if !matches!(k.as_str(), "env" | "version") {
            tags.push(format!("{k}:{v}"));
        }
    }
    if let Some(v) = text(event.get("environment")) {
        tags.push(format!("env:{v}"));
    }
    if let Some(v) = text(event.get("release")) {
        tags.push(format!("version:{v}"));
    }
    tags.sort();
    tags.dedup();

    let trace = event.pointer("/contexts/trace");
    LogRow {
        ts: event
            .get("timestamp")
            .and_then(parse_timestamp_ms)
            .unwrap_or(now),
        status,
        service: Some(service.unwrap_or_else(|| format!("sentry-{project}"))),
        host: text(event.get("server_name")),
        source: Some(
            text(event.get("platform"))
                .filter(|p| p != "other")
                .unwrap_or_else(|| "sentry".into()),
        ),
        message,
        tags,
        attributes: attrs,
        trace_id: trace
            .and_then(|t| t.get("trace_id"))
            .and_then(normalize_trace_id),
        span_id: trace
            .and_then(|t| t.get("span_id"))
            .and_then(normalize_trace_id),
    }
}

pub async fn ingest(
    st: AppState,
    project: String,
    service: Option<String>,
    q: HashMap<String, String>,
    headers: HeaderMap,
    body: Bytes,
    envelope: bool,
) -> Response {
    let Ok(raw) = decompress(&headers, body) else {
        return cors((StatusCode::BAD_REQUEST, "bad compression").into_response());
    };
    let (header, events): (Option<Value>, Vec<Value>) = if envelope {
        let (header, items) = envelope_items(&raw);
        let events = items
            .into_iter()
            .filter(|(typ, _)| typ == "event")
            .filter_map(|(_, payload)| serde_json::from_slice(&payload).ok())
            .collect();
        (header, events)
    } else {
        (None, serde_json::from_slice(&raw).into_iter().collect())
    };
    if !authorized(&st, sentry_key(&headers, &q, header.as_ref()).as_deref()) {
        return cors(
            (
                StatusCode::FORBIDDEN,
                Json(json!({"detail": "invalid sentry_key"})),
            )
                .into_response(),
        );
    }
    let now = now_ms();
    let mut id = header
        .as_ref()
        .and_then(|h| text(h.get("event_id")))
        .unwrap_or_default();
    let rows: Vec<LogRow> = events
        .iter()
        .map(|e| {
            if let Some(event_id) = text(e.get("event_id")) {
                id = event_id;
            }
            let mut row = event_to_log(e, &project, service.as_deref(), now);
            st.hosts.apply(&mut row.host, &mut row.tags);
            st.pipelines.apply(&mut row);
            row
        })
        .collect();
    tracing::debug!(count = rows.len(), project = %project, "sentry events received");
    if let Err(e) = crate::logconfig::ingest(&st, rows).await {
        tracing::error!(error = %e, "storing sentry events");
        return cors(StatusCode::INTERNAL_SERVER_ERROR.into_response());
    }
    cors(Json(json!({"id": id})).into_response())
}

pub async fn service_envelope(
    State(st): State<AppState>,
    Path((service, project)): Path<(String, String)>,
    Query(q): Query<HashMap<String, String>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    ingest(st, project, Some(service), q, headers, body, true).await
}

pub async fn service_store(
    State(st): State<AppState>,
    Path((service, project)): Path<(String, String)>,
    Query(q): Query<HashMap<String, String>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    ingest(st, project, Some(service), q, headers, body, false).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_an_envelope() {
        let raw = concat!(
            r#"{"event_id":"9ec79c33ec9942ab8353589fcb2e04dc","dsn":"https://pubkey@dog.example/sentry/shop/1"}"#,
            "\n",
            r#"{"type":"session"}"#,
            "\n",
            r#"{"sid":"x"}"#,
            "\n",
            r#"{"type":"event","length":14}"#,
            "\n",
            "{\"level\":\n\"e\"}",
            "\n",
            r#"{"type":"attachment","length":3}"#,
            "\nab\n\n"
        );
        let (header, items) = envelope_items(raw.as_bytes());
        let types: Vec<&str> = items.iter().map(|(t, _)| t.as_str()).collect();
        assert_eq!(types, ["session", "event", "attachment"]);
        // `length` wins over newlines inside the payload.
        assert_eq!(items[1].1, b"{\"level\":\n\"e\"}");
        assert_eq!(items[2].1, b"ab\n");
        let key = sentry_key(&HeaderMap::new(), &HashMap::new(), header.as_ref());
        assert_eq!(key.as_deref(), Some("pubkey"));

        let mut h = HeaderMap::new();
        h.insert(
            "x-sentry-auth",
            HeaderValue::from_static("Sentry sentry_version=7, sentry_key=abc, sentry_client=x/1"),
        );
        assert_eq!(
            sentry_key(&h, &HashMap::new(), None).as_deref(),
            Some("abc")
        );
    }

    #[test]
    fn an_exception_becomes_an_error_log() {
        let event = json!({
            "event_id": "9ec79c33ec9942ab8353589fcb2e04dc",
            "timestamp": 1794910470.5,
            "platform": "python",
            "level": "error",
            "server_name": "api-1",
            "environment": "prod",
            "release": "1.4.2",
            "transaction": "/orders",
            "tags": {"team": "shop", "service": "ignored-when-the-dsn-names-one"},
            "user": {"id": "42", "email": "a@b.c", "ip_address": "203.0.113.7"},
            "request": {"method": "POST", "url": "https://shop/orders", "headers": {"User-Agent": "curl/8"}},
            "contexts": {"trace": {"trace_id": "e205b806eba1583b23aa4a49b3cf6933", "span_id": "840db9e96fbf9bdd"}},
            "exception": {"values": [
                {"type": "KeyError", "value": "'sku'", "stacktrace": {"frames": [
                    {"function": "lookup", "filename": "shop/catalog.py", "lineno": 12}
                ]}},
                {"type": "OrderError", "value": "order 1234 failed", "mechanism": {"handled": false},
                 "stacktrace": {"frames": [
                    {"function": "main", "filename": "app.py", "lineno": 3},
                    {"function": "create", "filename": "shop/orders.py", "lineno": 88, "colno": 9}
                ]}}
            ]}
        });
        let l = event_to_log(&event, "1", Some("shop-api"), 0);
        assert_eq!(l.ts, 1794910470500);
        assert_eq!(l.status, "error");
        assert_eq!(l.message, "OrderError: order 1234 failed");
        assert_eq!(l.service.as_deref(), Some("shop-api"));
        assert_eq!(l.source.as_deref(), Some("python"));
        assert_eq!(l.host.as_deref(), Some("api-1"));
        assert_eq!(l.tags, ["env:prod", "team:shop", "version:1.4.2"]);
        assert_eq!(l.trace_id.as_deref(), Some("2569948217778071859"));
        let a = &l.attributes;
        assert_eq!(a["error"]["kind"], "OrderError");
        assert_eq!(a["error"]["message"], "order 1234 failed");
        assert_eq!(a["error"]["handled"], false);
        assert_eq!(
            a["error"]["stack"],
            "OrderError: order 1234 failed\n    at create (shop/orders.py:88:9)\n    at main (app.py:3)\nCaused by: KeyError: 'sku'\n    at lookup (shop/catalog.py:12)"
        );
        assert_eq!(a["usr"]["id"], "42");
        assert_eq!(a["http"]["useragent"], "curl/8");
        assert_eq!(a["network"]["client"]["ip"], "203.0.113.7");
        assert_eq!(a["sentry"]["transaction"], "/orders");
    }

    #[test]
    fn messages_and_fallbacks() {
        let l = event_to_log(
            &json!({"message": {"formatted": "disk almost full"}, "level": "warning", "tags": [["service", "worker"]]}),
            "7",
            None,
            5,
        );
        assert_eq!(
            (l.status.as_str(), l.message.as_str(), l.ts),
            ("warn", "disk almost full", 5)
        );
        assert_eq!(l.service.as_deref(), Some("worker"));
        assert!(l.attributes.get("error").is_none());

        // An error-level message is an issue; no level means error.
        let l = event_to_log(&json!({"logentry": {"message": "boom"}}), "7", None, 5);
        assert_eq!(l.service.as_deref(), Some("sentry-7"));
        assert_eq!(l.source.as_deref(), Some("sentry"));
        assert_eq!(l.attributes["error"]["kind"], "Message");
    }
}
