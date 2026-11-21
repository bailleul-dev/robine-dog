//! The trace-agent API on port 8126: what `ddtrace` libraries send to a local
//! Datadog Agent. robine-dog stands in for the agent, so an application
//! configured with `DD_AGENT_HOST`/`DD_TRACE_AGENT_URL` reports here directly.

use crate::AppState;
use crate::store::greptime::{lit, opt_lit};
use crate::store::writer::{Insertable, next_id};
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get, post};
use axum::{Json, Router};
use serde_json::{Map, Value, json};
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct SpanRow {
    pub trace_id: String,
    pub span_id: String,
    pub parent_id: String,
    pub service: String,
    pub name: String,
    pub resource: String,
    pub typ: Option<String>,
    pub env: Option<String>,
    pub version: Option<String>,
    pub start: i64,
    pub duration: i64,
    pub error: bool,
    pub top_level: bool,
    pub meta: Map<String, Value>,
    pub metrics: Map<String, Value>,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/v0.3/traces", post(v04).put(v04))
        .route("/v0.4/traces", post(v04).put(v04))
        .route("/v0.5/traces", post(v05).put(v05))
        .route("/info", get(info))
        .route("/v0.6/stats", any(discard))
        .route(
            "/profiling/v1/input",
            axum::routing::post(crate::profiling::intake),
        )
        .route(
            "/v0.1/pipeline_stats",
            axum::routing::post(crate::dsm::intake),
        )
        .route(
            "/telemetry/proxy/{*rest}",
            axum::routing::post(crate::appsec::telemetry),
        )
        .route("/debugger/v1/input", axum::routing::post(debugger))
        .route("/debugger/v2/input", axum::routing::post(debugger))
        .route("/debugger/{*rest}", any(discard))
        .route("/evp_proxy/{*rest}", any(crate::llmobs::evp_proxy))
        .route("/tracer_flare/v1", any(discard))
}

/// Exception Replay and Dynamic Instrumentation snapshots, sent by tracers to
/// the Agent's debugger endpoint: stored as logs (`source:dd_debugger`), as
/// the Agent forwards them to Datadog.
async fn debugger(
    axum::extract::State(st): axum::extract::State<crate::AppState>,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> axum::response::Response {
    let mut q = std::collections::HashMap::new();
    q.insert("ddsource".to_string(), "dd_debugger".to_string());
    if let Some(k) = &st.cfg.api_key {
        q.insert("dd-api-key".to_string(), k.clone());
    }
    super::logs::intake(
        axum::extract::State(st),
        axum::extract::Query(q),
        headers,
        body,
    )
    .await
}

async fn info() -> Json<Value> {
    Json(json!({
        "version": "7.60.0",
        "git_commit": "robine-dog",
        "endpoints": ["/v0.3/traces", "/v0.4/traces", "/v0.5/traces", "/telemetry/proxy/", "/info",
            "/profiling/v1/input", "/evp_proxy/v2/", "/evp_proxy/v4/", "/v0.1/pipeline_stats"],
        "client_drop_p0s": false,
        "span_meta_structs": false,
        "long_running_spans": false,
        "config": { "receiver_port": 8126 }
    }))
}

async fn discard() -> StatusCode {
    StatusCode::ACCEPTED
}

fn rate_response() -> Response {
    Json(json!({"rate_by_service": {"service:,env:": 1.0}})).into_response()
}

pub fn mp_to_json(v: rmpv::Value) -> Value {
    use rmpv::Value as M;
    match v {
        M::Nil => Value::Null,
        M::Boolean(b) => Value::Bool(b),
        M::Integer(i) => {
            if let Some(u) = i.as_u64() {
                json!(u)
            } else if let Some(s) = i.as_i64() {
                json!(s)
            } else {
                Value::Null
            }
        }
        M::F32(f) => json!(f as f64),
        M::F64(f) => json!(f),
        M::String(s) => Value::String(s.into_str().unwrap_or_default()),
        M::Binary(b) => Value::String(String::from_utf8_lossy(&b).into_owned()),
        M::Array(a) => Value::Array(a.into_iter().map(mp_to_json).collect()),
        M::Map(m) => Value::Object(
            m.into_iter()
                .map(|(k, v)| {
                    let key = match mp_to_json(k) {
                        Value::String(s) => s,
                        other => other.to_string(),
                    };
                    (key, mp_to_json(v))
                })
                .collect(),
        ),
        M::Ext(_, _) => Value::Null,
    }
}

fn id_string(v: Option<&Value>) -> String {
    match v {
        Some(Value::Number(n)) => n
            .as_u64()
            .map(|u| u.to_string())
            .unwrap_or_else(|| n.to_string()),
        Some(Value::String(s)) if !s.is_empty() => s.clone(),
        _ => "0".to_string(),
    }
}

fn int(v: Option<&Value>) -> i64 {
    match v {
        Some(Value::Number(n)) => n
            .as_i64()
            .or_else(|| n.as_f64().map(|f| f as i64))
            .unwrap_or(0),
        Some(Value::String(s)) => s.parse().unwrap_or(0),
        _ => 0,
    }
}

fn text(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

fn string_map(v: Option<&Value>) -> Map<String, Value> {
    match v {
        Some(Value::Object(m)) => m
            .iter()
            .map(|(k, v)| (k.clone(), Value::String(text(Some(v)))))
            .collect(),
        _ => Map::new(),
    }
}

fn number_map(v: Option<&Value>) -> Map<String, Value> {
    match v {
        Some(Value::Object(m)) => m
            .iter()
            .filter_map(|(k, v)| v.as_f64().map(|f| (k.clone(), json!(f))))
            .collect(),
        _ => Map::new(),
    }
}

/// A span in the v0.3/v0.4 map form (msgpack or JSON).
pub fn span_from_map(m: &Map<String, Value>) -> SpanRow {
    let meta = string_map(m.get("meta"));
    let metrics = number_map(m.get("metrics"));
    build(
        id_string(m.get("trace_id")),
        id_string(m.get("span_id")),
        id_string(m.get("parent_id")),
        text(m.get("service")),
        text(m.get("name")),
        text(m.get("resource")),
        Some(text(m.get("type"))),
        int(m.get("start")),
        int(m.get("duration")),
        int(m.get("error")) != 0,
        meta,
        metrics,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn build(
    trace_id: String,
    span_id: String,
    parent_id: String,
    service: String,
    name: String,
    resource: String,
    typ: Option<String>,
    start: i64,
    duration: i64,
    error: bool,
    meta: Map<String, Value>,
    metrics: Map<String, Value>,
) -> SpanRow {
    let service = if service.is_empty() {
        "unnamed-service".to_string()
    } else {
        service
    };
    let resource = if resource.is_empty() {
        name.clone()
    } else {
        resource
    };
    let env = meta.get("env").and_then(|v| v.as_str()).map(String::from);
    let version = meta
        .get("version")
        .and_then(|v| v.as_str())
        .map(String::from);
    let top_level = metrics.get("_top_level").and_then(|v| v.as_f64()) == Some(1.0)
        || metrics.get("_dd.top_level").and_then(|v| v.as_f64()) == Some(1.0);
    SpanRow {
        trace_id,
        span_id,
        parent_id,
        service,
        name,
        resource,
        typ: typ.filter(|t| !t.is_empty()),
        env,
        version,
        start,
        duration: duration.max(0),
        error,
        top_level,
        meta,
        metrics,
    }
}

/// A span is a service entry span when it is a root, when its parent is not
/// in the chunk, or when its parent belongs to another service — as the agent decides.
pub fn mark_top_level(chunk: &mut [SpanRow]) {
    let services: HashMap<String, String> = chunk
        .iter()
        .map(|s| (s.span_id.clone(), s.service.clone()))
        .collect();
    for s in chunk.iter_mut() {
        if s.top_level {
            continue;
        }
        s.top_level = match services.get(&s.parent_id) {
            None => true,
            Some(parent_service) => parent_service != &s.service,
        };
    }
}

pub fn decode_v04(headers: &HeaderMap, body: &[u8]) -> anyhow::Result<Vec<Vec<SpanRow>>> {
    let ct = headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let root: Value = if ct.contains("json") {
        serde_json::from_slice(body)?
    } else {
        mp_to_json(rmpv::decode::read_value(&mut &body[..])?)
    };
    let traces = root.as_array().cloned().unwrap_or_default();
    Ok(traces
        .iter()
        .map(|t| {
            t.as_array()
                .map(|spans| {
                    spans
                        .iter()
                        .filter_map(|s| s.as_object().map(span_from_map))
                        .collect()
                })
                .unwrap_or_default()
        })
        .collect())
}

/// v0.5: `[strings, traces]`, each span a 12-element array of string-table indexes and values.
pub fn decode_v05(body: &[u8]) -> anyhow::Result<Vec<Vec<SpanRow>>> {
    let root = rmpv::decode::read_value(&mut &body[..])?;
    let parts = root
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("v0.5 payload is not an array"))?;
    if parts.len() != 2 {
        anyhow::bail!("v0.5 payload must have 2 elements");
    }
    let strings: Vec<String> = parts[0]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("v0.5 string table"))?
        .iter()
        .map(|s| s.as_str().unwrap_or_default().to_string())
        .collect();
    let s = |v: &rmpv::Value| -> String {
        v.as_u64()
            .and_then(|i| strings.get(i as usize))
            .cloned()
            .unwrap_or_default()
    };
    let mut out = Vec::new();
    for trace in parts[1]
        .as_array()
        .map(|a| a.as_slice())
        .unwrap_or_default()
    {
        let mut chunk = Vec::new();
        for span in trace.as_array().map(|a| a.as_slice()).unwrap_or_default() {
            let Some(f) = span.as_array() else { continue };
            if f.len() < 12 {
                continue;
            }
            let mut meta = Map::new();
            if let Some(m) = f[9].as_map() {
                for (k, v) in m {
                    meta.insert(s(k), Value::String(s(v)));
                }
            }
            let mut metrics = Map::new();
            if let Some(m) = f[10].as_map() {
                for (k, v) in m {
                    if let Some(x) = v.as_f64().or_else(|| v.as_i64().map(|i| i as f64)) {
                        metrics.insert(s(k), json!(x));
                    }
                }
            }
            let id = |v: &rmpv::Value| {
                v.as_u64()
                    .map(|u| u.to_string())
                    .unwrap_or_else(|| "0".into())
            };
            chunk.push(build(
                id(&f[3]),
                id(&f[4]),
                id(&f[5]),
                s(&f[0]),
                s(&f[1]),
                s(&f[2]),
                Some(s(&f[11])),
                f[6].as_i64().unwrap_or(0),
                f[7].as_i64().unwrap_or(0),
                f[8].as_i64().unwrap_or(0) != 0,
                meta,
                metrics,
            ));
        }
        out.push(chunk);
    }
    Ok(out)
}

impl Insertable for SpanRow {
    const TABLE: &'static str = "spans";
    const COLUMNS: &'static str = r#"("ts", "id", "trace_id", "span_id", "parent_id", "service", "name", "resource", "type", "env", "version", "duration", "error", "top_level", "meta", "metrics")"#;

    fn values(&self, out: &mut String) {
        use std::fmt::Write;
        let meta = serde_json::to_string(&self.meta).unwrap_or_else(|_| "{}".into());
        let metrics = serde_json::to_string(&self.metrics).unwrap_or_else(|_| "{}".into());
        let _ = write!(
            out,
            "({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})",
            self.start,
            next_id(),
            lit(&self.trace_id),
            lit(&self.span_id),
            lit(&self.parent_id),
            lit(&self.service),
            lit(&self.name),
            lit(&self.resource),
            opt_lit(self.typ.as_deref()),
            opt_lit(self.env.as_deref()),
            opt_lit(self.version.as_deref()),
            self.duration,
            self.error,
            self.top_level,
            lit(&meta),
            lit(&metrics),
        );
    }
}

/// Stores trace chunks: service entry spans marked, trace metrics derived.
/// Every span feeds the trace metrics and span-based metrics; retention
/// filters then decide which traces are stored.
pub async fn ingest(st: &AppState, mut chunks: Vec<Vec<SpanRow>>) -> anyhow::Result<usize> {
    let mut attacks = Vec::new();
    for chunk in &mut chunks {
        mark_top_level(chunk);
        for s in chunk.iter() {
            crate::metrics::trace_metrics(&st.aggregator, s);
            st.generated.on_span(&st.aggregator, s);
            attacks.extend(crate::appsec::attacks(s));
        }
    }
    // Attacks are kept whatever the retention filters decide for the trace.
    if !attacks.is_empty() {
        st.store.logs.send(attacks).await?;
    }
    let now = crate::intake::logs::now_ms();
    let spans: Vec<SpanRow> = crate::catalog::retention::filter(st, chunks, now)
        .into_iter()
        .flatten()
        .collect();
    let n = spans.len();
    st.store.spans.send(spans).await?;
    Ok(n)
}

async fn store(st: AppState, decoded: anyhow::Result<Vec<Vec<SpanRow>>>) -> Response {
    let chunks = match decoded {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, "undecodable trace payload");
            return (StatusCode::BAD_REQUEST, e.to_string()).into_response();
        }
    };
    match ingest(&st, chunks).await {
        Ok(n) => {
            tracing::debug!(spans = n, "traces received");
            rate_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "storing spans");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

fn authorized(st: &AppState, headers: &HeaderMap) -> bool {
    // The trace port is meant for a trusted network, like a local agent; an
    // API key is only demanded when `ROBINE_DOG_TRACE_REQUIRE_KEY` is set.
    if !st.cfg.trace_require_key {
        return true;
    }
    crate::intake::logs::api_key_ok(st, headers, &HashMap::new())
}

async fn v04(State(st): State<AppState>, headers: HeaderMap, body: Bytes) -> Response {
    if !authorized(&st, &headers) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let decoded = decode_v04(&headers, &body);
    store(st, decoded).await
}

async fn v05(State(st): State<AppState>, headers: HeaderMap, body: Bytes) -> Response {
    if !authorized(&st, &headers) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let decoded = decode_v05(&body);
    store(st, decoded).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span(id: &str, parent: &str, service: &str) -> SpanRow {
        build(
            "1".into(),
            id.into(),
            parent.into(),
            service.into(),
            "op".into(),
            "".into(),
            None,
            0,
            10,
            false,
            Map::new(),
            Map::new(),
        )
    }

    #[test]
    fn top_level_detection() {
        let mut chunk = vec![
            span("1", "0", "web"),
            span("2", "1", "web"),
            span("3", "2", "db"),
        ];
        mark_top_level(&mut chunk);
        assert_eq!(
            chunk.iter().map(|s| s.top_level).collect::<Vec<_>>(),
            vec![true, false, true]
        );
        assert_eq!(chunk[0].resource, "op");
    }

    #[test]
    fn decodes_v04_msgpack() {
        use rmpv::Value as M;
        let span = M::Map(vec![
            (M::from("trace_id"), M::from(u64::MAX)),
            (M::from("span_id"), M::from(7u64)),
            (M::from("parent_id"), M::from(0u64)),
            (M::from("service"), M::from("web")),
            (M::from("name"), M::from("http.request")),
            (M::from("resource"), M::from("GET /")),
            (M::from("start"), M::from(1_700_000_000_000_000_000i64)),
            (M::from("duration"), M::from(5_000_000i64)),
            (M::from("error"), M::from(1)),
            (
                M::from("meta"),
                M::Map(vec![(M::from("env"), M::from("prod"))]),
            ),
            (
                M::from("metrics"),
                M::Map(vec![(M::from("_sampling_priority_v1"), M::from(1.0))]),
            ),
        ]);
        let mut buf = Vec::new();
        rmpv::encode::write_value(&mut buf, &M::Array(vec![M::Array(vec![span])])).unwrap();
        let chunks = decode_v04(&HeaderMap::new(), &buf).unwrap();
        let s = &chunks[0][0];
        assert_eq!(s.trace_id, u64::MAX.to_string());
        assert_eq!(s.env.as_deref(), Some("prod"));
        assert!(s.error);
        assert_eq!(s.duration, 5_000_000);
    }

    #[test]
    fn decodes_v05_msgpack() {
        use rmpv::Value as M;
        let strings = M::Array(
            ["", "web", "http.request", "GET /", "env", "prod", "web"]
                .iter()
                .map(|s| M::from(*s))
                .collect(),
        );
        let span = M::Array(vec![
            M::from(1u32),
            M::from(2u32),
            M::from(3u32),
            M::from(42u64),
            M::from(43u64),
            M::from(0u64),
            M::from(100i64),
            M::from(50i64),
            M::from(0),
            M::Map(vec![(M::from(4u32), M::from(5u32))]),
            M::Map(vec![]),
            M::from(6u32),
        ]);
        let mut buf = Vec::new();
        rmpv::encode::write_value(
            &mut buf,
            &M::Array(vec![strings, M::Array(vec![M::Array(vec![span])])]),
        )
        .unwrap();
        let chunks = decode_v05(&buf).unwrap();
        let s = &chunks[0][0];
        assert_eq!(
            (s.service.as_str(), s.resource.as_str(), s.trace_id.as_str()),
            ("web", "GET /", "42")
        );
        assert_eq!(s.env.as_deref(), Some("prod"));
        assert_eq!(s.typ.as_deref(), Some("web"));
    }
}
