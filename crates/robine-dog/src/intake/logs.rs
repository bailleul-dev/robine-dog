//! Datadog-compatible logs intake: `POST /api/v2/logs` (and the legacy `/v1/input`).
//!
//! Accepts what the Datadog Agent (with `logs_config.use_http`), the
//! serverless forwarders and plain `curl` send: a JSON object or array of
//! objects, optionally gzip/deflate-compressed, authenticated by `DD-API-KEY`.

use crate::AppState;
use crate::store::greptime::{lit, opt_lit, tags_column};
use crate::store::writer::{Insertable, next_id};
use axum::Json;
use axum::body::Bytes;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct LogRow {
    pub ts: i64,
    pub status: String,
    pub service: Option<String>,
    pub host: Option<String>,
    pub source: Option<String>,
    pub message: String,
    pub tags: Vec<String>,
    pub attributes: Map<String, Value>,
    pub trace_id: Option<String>,
    pub span_id: Option<String>,
}

/// Maps any level spelling to Datadog's statuses, as its status remapper does.
pub fn normalize_status(s: &str) -> &'static str {
    let l = s.trim().to_ascii_lowercase();
    if let Ok(n) = l.parse::<i64>() {
        return match n {
            0 => "emergency",
            1 => "alert",
            2 => "critical",
            3 => "error",
            4 => "warn",
            5 => "notice",
            6 => "info",
            _ => "debug",
        };
    }
    if l.starts_with("emerg") || l.starts_with('f') || l.starts_with("panic") {
        "emergency"
    } else if l.starts_with('a') {
        "alert"
    } else if l.starts_with('c') {
        "critical"
    } else if l.starts_with('e') {
        "error"
    } else if l.starts_with('w') {
        "warn"
    } else if l.starts_with('n') {
        "notice"
    } else if l.starts_with('i') {
        "info"
    } else if l.starts_with('d') || l.starts_with('t') || l.starts_with('v') {
        "debug"
    } else if l.starts_with('o') || l.starts_with('s') {
        "ok"
    } else {
        "info"
    }
}

/// Datadog trace ids in logs are either decimal (64 bits) or 32 hex chars (128 bits).
/// Spans are stored by the decimal low 64 bits, so both forms correlate.
pub fn normalize_trace_id(v: &Value) -> Option<String> {
    let s = match v {
        Value::String(s) => s.trim().to_string(),
        Value::Number(n) => n.to_string(),
        _ => return None,
    };
    if s.is_empty() || s == "0" {
        return None;
    }
    if s.len() == 32 && s.chars().all(|c| c.is_ascii_hexdigit()) {
        return u64::from_str_radix(&s[16..], 16)
            .ok()
            .map(|n| n.to_string());
    }
    if s.len() == 16
        && s.chars().any(|c| c.is_ascii_alphabetic())
        && s.chars().all(|c| c.is_ascii_hexdigit())
    {
        return u64::from_str_radix(&s, 16).ok().map(|n| n.to_string());
    }
    Some(s)
}

/// `{"a.b": 1}` → `{"a": {"b": 1}}`, recursively, so `@a.b` always means one path.
pub fn unflatten(map: Map<String, Value>) -> Map<String, Value> {
    let mut out = Map::new();
    for (k, v) in map {
        let v = match v {
            Value::Object(o) => Value::Object(unflatten(o)),
            other => other,
        };
        let parts: Vec<&str> = k.split('.').filter(|p| !p.is_empty()).collect();
        if parts.len() <= 1 {
            merge(&mut out, k, v);
            continue;
        }
        let mut nested = v;
        for p in parts[1..].iter().rev() {
            let mut m = Map::new();
            m.insert((*p).to_string(), nested);
            nested = Value::Object(m);
        }
        merge(&mut out, parts[0].to_string(), nested);
    }
    out
}

fn merge(into: &mut Map<String, Value>, k: String, v: Value) {
    match (into.get_mut(&k), v) {
        (Some(Value::Object(existing)), Value::Object(new)) => {
            for (k2, v2) in new {
                merge(existing, k2, v2);
            }
        }
        (_, v) => {
            into.insert(k, v);
        }
    }
}

fn get_path<'a>(m: &'a Map<String, Value>, path: &[&str]) -> Option<&'a Value> {
    let mut cur = m.get(path[0])?;
    for p in &path[1..] {
        cur = cur.as_object()?.get(*p)?;
    }
    Some(cur)
}

fn take_string(m: &mut Map<String, Value>, keys: &[&str]) -> Option<String> {
    for k in keys {
        if let Some(Value::String(s)) = m.get(*k) {
            let s = s.clone();
            m.remove(*k);
            return Some(s);
        }
    }
    None
}

fn as_text(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// Epoch seconds, milliseconds, microseconds or nanoseconds, or RFC 3339.
pub fn parse_timestamp_ms(v: &Value) -> Option<i64> {
    match v {
        Value::Number(n) => {
            let f = n.as_f64()?;
            Some(if f > 1e17 {
                (f / 1e6) as i64
            } else if f > 1e14 {
                (f / 1e3) as i64
            } else if f > 1e11 {
                f as i64
            } else {
                (f * 1e3) as i64
            })
        }
        Value::String(s) => {
            if let Ok(f) = s.parse::<f64>() {
                return parse_timestamp_ms(&json!(f));
            }
            let t = time::OffsetDateTime::parse(s, &time::format_description::well_known::Rfc3339)
                .ok()?;
            Some((t.unix_timestamp_nanos() / 1_000_000) as i64)
        }
        _ => None,
    }
}

pub fn now_ms() -> i64 {
    (time::OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000) as i64
}

#[derive(Default)]
pub struct Defaults {
    pub source: Option<String>,
    pub service: Option<String>,
    pub host: Option<String>,
    pub tags: Vec<String>,
}

pub fn split_tags(s: &str) -> Vec<String> {
    s.split(',')
        .map(|t| t.trim())
        .filter(|t| !t.is_empty())
        .map(|t| t.to_string())
        .collect()
}

pub fn process(obj: Map<String, Value>, defaults: &Defaults, now: i64) -> LogRow {
    let mut obj = obj;

    // A message that is itself a JSON object is parsed, as Datadog's JSON preprocessing does.
    let mut message = take_string(&mut obj, &["message"]);
    let inner = message
        .as_deref()
        .map(str::trim_start)
        .filter(|t| t.starts_with('{'))
        .and_then(|t| serde_json::from_str::<Value>(t).ok());
    if let Some(Value::Object(inner)) = inner {
        for (k, v) in inner {
            obj.insert(k, v);
        }
        message = Some(take_string(&mut obj, &["message", "msg", "log"]).unwrap_or_default());
    }
    if message.is_none() {
        message = take_string(&mut obj, &["msg", "log"]);
    }

    let mut tags = defaults.tags.clone();
    if let Some(t) = take_string(&mut obj, &["ddtags"]) {
        tags.extend(split_tags(&t));
    }
    let source = take_string(&mut obj, &["ddsource"]).or_else(|| defaults.source.clone());
    let host = take_string(&mut obj, &["hostname"])
        .or_else(|| take_string(&mut obj, &["host"]))
        .or_else(|| defaults.host.clone());
    let mut service = take_string(&mut obj, &["service"]).or_else(|| defaults.service.clone());

    let mut ts = None;
    for k in [
        "timestamp",
        "date",
        "@timestamp",
        "time",
        "_timestamp",
        "published_date",
    ] {
        if let Some(v) = obj.get(k)
            && let Some(t) = parse_timestamp_ms(v)
        {
            ts = Some(t);
            obj.remove(k);
            break;
        }
    }

    let status_raw = match obj.remove("status") {
        Some(v) => as_text(&v),
        None => ["level", "severity", "loglevel", "log_level", "levelname"]
            .iter()
            .find_map(|k| obj.get(*k).and_then(as_text)),
    };

    let attributes = unflatten(obj);

    let trace_id = get_path(&attributes, &["dd", "trace_id"])
        .or_else(|| attributes.get("trace_id"))
        .and_then(normalize_trace_id);
    let span_id = get_path(&attributes, &["dd", "span_id"])
        .or_else(|| attributes.get("span_id"))
        .and_then(normalize_trace_id);

    if service.is_none() {
        service = get_path(&attributes, &["dd", "service"]).and_then(as_text);
    }
    if service.is_none() {
        service = tags
            .iter()
            .find_map(|t| t.strip_prefix("service:").map(String::from));
    }
    for (attr, tag) in [("env", "env"), ("version", "version")] {
        if !tags.iter().any(|t| t.starts_with(&format!("{tag}:")))
            && let Some(v) = get_path(&attributes, &["dd", attr]).and_then(as_text)
        {
            tags.push(format!("{tag}:{v}"));
        }
    }
    if let Some(s) = &source
        && !tags.iter().any(|t| t.starts_with("source:"))
    {
        tags.push(format!("source:{s}"));
    }

    LogRow {
        ts: ts.unwrap_or(now),
        status: normalize_status(status_raw.as_deref().unwrap_or("info")).to_string(),
        service,
        host,
        source,
        message: message.unwrap_or_default(),
        tags,
        attributes,
        trace_id,
        span_id,
    }
}

impl Insertable for LogRow {
    const TABLE: &'static str = "logs";
    const COLUMNS: &'static str = r#"("ts", "id", "status", "service", "host", "source", "message", "tags", "attributes", "trace_id", "span_id")"#;

    fn values(&self, out: &mut String) {
        use std::fmt::Write;
        let attrs = serde_json::to_string(&self.attributes).unwrap_or_else(|_| "{}".into());
        let _ = write!(
            out,
            "({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})",
            self.ts,
            next_id(),
            lit(&self.status),
            opt_lit(self.service.as_deref()),
            opt_lit(self.host.as_deref()),
            opt_lit(self.source.as_deref()),
            lit(&self.message),
            lit(&tags_column(&self.tags)),
            lit(&attrs),
            opt_lit(self.trace_id.as_deref()),
            opt_lit(self.span_id.as_deref()),
        );
    }
}

pub fn decompress(headers: &HeaderMap, body: Bytes) -> std::io::Result<Vec<u8>> {
    use std::io::Read;
    let enc = headers
        .get(axum::http::header::CONTENT_ENCODING)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();
    let mut out = Vec::new();
    match enc.as_str() {
        "gzip" => {
            flate2::read::GzDecoder::new(&body[..]).read_to_end(&mut out)?;
        }
        "deflate" => {
            flate2::read::ZlibDecoder::new(&body[..]).read_to_end(&mut out)?;
        }
        _ => out = body.to_vec(),
    }
    Ok(out)
}

pub fn api_key_ok(st: &AppState, headers: &HeaderMap, q: &HashMap<String, String>) -> bool {
    let Some(expected) = &st.cfg.api_key else {
        return true;
    };
    let given = headers
        .get("dd-api-key")
        .and_then(|v| v.to_str().ok())
        .map(String::from)
        .or_else(|| q.get("dd-api-key").cloned())
        .or_else(|| q.get("api_key").cloned());
    given.as_deref() == Some(expected.as_str())
}

pub async fn intake(
    State(st): State<AppState>,
    Query(q): Query<HashMap<String, String>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if !api_key_ok(&st, &headers, &q) {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({"errors": ["Forbidden"]})),
        )
            .into_response();
    }
    let raw = match decompress(&headers, body) {
        Ok(b) => b,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"errors": ["Bad compression"]})),
            )
                .into_response();
        }
    };
    let ct = headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let parsed: Vec<Map<String, Value>> = match serde_json::from_slice::<Value>(&raw) {
        Ok(Value::Array(items)) => items
            .into_iter()
            .filter_map(|v| match v {
                Value::Object(o) => Some(o),
                Value::String(s) => {
                    Some(Map::from_iter([("message".to_string(), Value::String(s))]))
                }
                _ => None,
            })
            .collect(),
        Ok(Value::Object(o)) => vec![o],
        _ if !ct.contains("json") => String::from_utf8_lossy(&raw)
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| Map::from_iter([("message".to_string(), Value::String(l.to_string()))]))
            .collect(),
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"errors": ["Malformed JSON"]})),
            )
                .into_response();
        }
    };
    let defaults = Defaults {
        source: q.get("ddsource").cloned(),
        service: q.get("service").cloned(),
        host: q.get("hostname").or_else(|| q.get("host")).cloned(),
        tags: q.get("ddtags").map(|t| split_tags(t)).unwrap_or_default(),
    };
    let now = now_ms();
    let rows: Vec<LogRow> = parsed
        .into_iter()
        .map(|o| process(o, &defaults, now))
        .collect();
    let n = rows.len();
    if let Err(e) = st.store.logs.send(rows).await {
        tracing::error!(error = %e, "storing logs");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"errors": ["Storage error"]})),
        )
            .into_response();
    }
    tracing::debug!(count = n, "logs received");
    (StatusCode::ACCEPTED, Json(json!({}))).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obj(v: Value) -> Map<String, Value> {
        v.as_object().unwrap().clone()
    }

    #[test]
    fn statuses() {
        assert_eq!(normalize_status("WARNING"), "warn");
        assert_eq!(normalize_status("err"), "error");
        assert_eq!(normalize_status("emerg"), "emergency");
        assert_eq!(normalize_status("fatal"), "emergency");
        assert_eq!(normalize_status("3"), "error");
        assert_eq!(normalize_status("trace"), "debug");
        assert_eq!(normalize_status("whatever"), "warn");
        assert_eq!(normalize_status("OK"), "ok");
    }

    #[test]
    fn trace_ids() {
        assert_eq!(normalize_trace_id(&json!("1234")), Some("1234".into()));
        assert_eq!(normalize_trace_id(&json!(1234)), Some("1234".into()));
        assert_eq!(
            normalize_trace_id(&json!("6553f5d200000000000000000000000a")),
            Some("10".into())
        );
        assert_eq!(normalize_trace_id(&json!("0")), None);
    }

    #[test]
    fn unflattens_dotted_keys() {
        let m = unflatten(obj(
            json!({"dd.trace_id": "1", "dd": {"span_id": "2"}, "http.status_code": 500}),
        ));
        assert_eq!(
            Value::Object(m),
            json!({"dd": {"trace_id": "1", "span_id": "2"}, "http": {"status_code": 500}})
        );
    }

    #[test]
    fn remaps_reserved_attributes() {
        let r = process(
            obj(json!({
                "message": "{\"msg\":\"payment failed\",\"level\":\"ERROR\",\"dd.trace_id\":\"42\",\"dd.env\":\"prod\",\"amount\":12.5}",
                "ddsource": "python",
                "ddtags": "team:pay,env:staging",
                "hostname": "i-123",
                "service": "payments",
                "timestamp": 1700000000123i64
            })),
            &Defaults::default(),
            0,
        );
        assert_eq!(r.message, "payment failed");
        assert_eq!(r.status, "error");
        assert_eq!(r.trace_id.as_deref(), Some("42"));
        assert_eq!(r.ts, 1700000000123);
        assert_eq!(r.host.as_deref(), Some("i-123"));
        assert_eq!(r.service.as_deref(), Some("payments"));
        assert_eq!(r.source.as_deref(), Some("python"));
        assert!(r.tags.contains(&"env:staging".to_string()));
        assert!(!r.tags.contains(&"env:prod".to_string()));
        assert_eq!(r.attributes["amount"], json!(12.5));
    }

    #[test]
    fn timestamps() {
        assert_eq!(parse_timestamp_ms(&json!(1700000000)), Some(1700000000000));
        assert_eq!(
            parse_timestamp_ms(&json!(1700000000123i64)),
            Some(1700000000123)
        );
        assert_eq!(
            parse_timestamp_ms(&json!(1700000000123456789i64)),
            Some(1700000000123)
        );
        assert_eq!(
            parse_timestamp_ms(&json!("2023-11-14T22:13:20.123Z")),
            Some(1700000000123)
        );
    }
}
