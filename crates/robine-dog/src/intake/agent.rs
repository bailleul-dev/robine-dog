//! What a real Datadog Agent forwards once its intake URLs point here
//! (`DD_DD_URL`, `DD_APM_DD_URL`, `DD_LOGS_CONFIG_LOGS_DD_URL`):
//!
//! - traces: `POST /api/v0.2/traces`, a protobuf `AgentPayload`;
//! - APM stats: `POST /api/v0.2/stats` (accepted, unused: trace metrics are
//!   derived from the spans themselves);
//! - metrics: `POST /api/v2/series` (protobuf `MetricPayload`, see
//!   [`parse_series`]) and `POST /api/beta/sketches` (protobuf
//!   `SketchPayload`: the distributions, as DDSketch bins);
//! - the rest (host metadata, check runs…) is acknowledged and logged once.
//!
//! The messages mirror `datadog-agent/pkg/proto/datadog/trace` and
//! `agent-payload/proto/metrics`, written by hand to need no `protoc`.

use super::logs::{api_key_ok, decompress};
use super::series::{Series, valid_name};
use crate::AppState;
use crate::metrics::{Kind, Point, normalize_tags};
use axum::Json;
use axum::body::Bytes;
use axum::extract::{OriginalUri, Query, State};
use axum::http::{HeaderMap, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use prost::Message;
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Mutex;

// --- traces --------------------------------------------------------------

#[derive(Clone, PartialEq, Message)]
pub struct AgentPayload {
    #[prost(string, tag = "1")]
    pub host_name: String,
    #[prost(string, tag = "2")]
    pub env: String,
    #[prost(message, repeated, tag = "5")]
    pub tracer_payloads: Vec<TracerPayload>,
    #[prost(map = "string, string", tag = "6")]
    pub tags: HashMap<String, String>,
    #[prost(string, tag = "7")]
    pub agent_version: String,
    /// The string-table encoding recent Agents (7.7x+) send instead.
    #[prost(message, repeated, tag = "11")]
    pub idx_tracer_payloads: Vec<idx::TracerPayload>,
}

#[derive(Clone, PartialEq, Message)]
pub struct TracerPayload {
    #[prost(string, tag = "1")]
    pub container_id: String,
    #[prost(string, tag = "2")]
    pub language_name: String,
    #[prost(string, tag = "3")]
    pub language_version: String,
    #[prost(string, tag = "4")]
    pub tracer_version: String,
    #[prost(string, tag = "5")]
    pub runtime_id: String,
    #[prost(message, repeated, tag = "6")]
    pub chunks: Vec<TraceChunk>,
    #[prost(map = "string, string", tag = "7")]
    pub tags: HashMap<String, String>,
    #[prost(string, tag = "8")]
    pub env: String,
    #[prost(string, tag = "9")]
    pub hostname: String,
    #[prost(string, tag = "10")]
    pub app_version: String,
}

#[derive(Clone, PartialEq, Message)]
pub struct TraceChunk {
    #[prost(int32, tag = "1")]
    pub priority: i32,
    #[prost(string, tag = "2")]
    pub origin: String,
    #[prost(message, repeated, tag = "3")]
    pub spans: Vec<Span>,
    #[prost(map = "string, string", tag = "4")]
    pub tags: HashMap<String, String>,
    #[prost(bool, tag = "5")]
    pub dropped_trace: bool,
}

#[derive(Clone, PartialEq, Message)]
pub struct Span {
    #[prost(string, tag = "1")]
    pub service: String,
    #[prost(string, tag = "2")]
    pub name: String,
    #[prost(string, tag = "3")]
    pub resource: String,
    #[prost(uint64, tag = "4")]
    pub trace_id: u64,
    #[prost(uint64, tag = "5")]
    pub span_id: u64,
    #[prost(uint64, tag = "6")]
    pub parent_id: u64,
    #[prost(int64, tag = "7")]
    pub start: i64,
    #[prost(int64, tag = "8")]
    pub duration: i64,
    #[prost(int32, tag = "9")]
    pub error: i32,
    #[prost(map = "string, string", tag = "10")]
    pub meta: HashMap<String, String>,
    #[prost(map = "string, double", tag = "11")]
    pub metrics: HashMap<String, f64>,
    #[prost(string, tag = "12")]
    pub r#type: String,
}

/// The spans of an `AgentPayload`, one chunk per trace, with what the Agent
/// moved up to the payload (env, version, hostname, sampling priority)
/// put back on the spans.
pub fn decode_traces(body: &[u8]) -> anyhow::Result<Vec<Vec<super::traces::SpanRow>>> {
    let p = AgentPayload::decode(body)?;
    let mut out = Vec::new();
    for tp in &p.idx_tracer_payloads {
        out.extend(idx::chunks(tp, &p.env, &p.host_name));
    }
    for tp in p.tracer_payloads {
        let env = first_non_empty(&[&tp.env, &p.env]);
        let host = first_non_empty(&[&tp.hostname, &p.host_name]);
        for chunk in tp.chunks {
            let rows = chunk
                .spans
                .into_iter()
                .map(|s| {
                    let mut meta: Map<String, Value> = s
                        .meta
                        .into_iter()
                        .map(|(k, v)| (k, Value::String(v)))
                        .collect();
                    let mut put = |k: &str, v: &str| {
                        if !v.is_empty() && !meta.contains_key(k) {
                            meta.insert(k.to_string(), Value::String(v.to_string()));
                        }
                    };
                    put("env", &env);
                    put("version", &tp.app_version);
                    put("_dd.hostname", &host);
                    put("_dd.origin", &chunk.origin);
                    put("language", &tp.language_name);
                    let mut metrics: Map<String, Value> = s
                        .metrics
                        .into_iter()
                        .filter(|(_, v)| v.is_finite())
                        .map(|(k, v)| (k, json!(v)))
                        .collect();
                    metrics
                        .entry("_sampling_priority_v1")
                        .or_insert(json!(chunk.priority));
                    super::traces::build(
                        s.trace_id.to_string(),
                        s.span_id.to_string(),
                        s.parent_id.to_string(),
                        s.service,
                        s.name,
                        s.resource,
                        Some(s.r#type),
                        s.start,
                        s.duration,
                        s.error != 0,
                        meta,
                        metrics,
                    )
                })
                .collect();
            out.push(rows);
        }
    }
    Ok(out)
}

fn first_non_empty(v: &[&str]) -> String {
    v.iter()
        .find(|s| !s.is_empty())
        .map(|s| s.to_string())
        .unwrap_or_default()
}

fn forbidden() -> Response {
    (
        StatusCode::FORBIDDEN,
        Json(json!({"errors": ["Forbidden"]})),
    )
        .into_response()
}

fn bad_request(e: impl std::fmt::Display) -> Response {
    tracing::warn!(error = %e, "undecodable Agent payload");
    (
        StatusCode::BAD_REQUEST,
        Json(json!({"errors": [e.to_string()]})),
    )
        .into_response()
}

pub async fn traces(
    State(st): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
    body: Bytes,
) -> Response {
    if !api_key_ok(&st, &headers, &q) {
        return forbidden();
    }
    let chunks = match decompress(&headers, body)
        .map_err(anyhow::Error::from)
        .and_then(|b| decode_traces(&b))
    {
        Ok(c) => c,
        Err(e) => return bad_request(e),
    };
    match super::traces::ingest(&st, chunks).await {
        Ok(n) => {
            tracing::debug!(spans = n, "Agent traces received");
            (StatusCode::ACCEPTED, Json(json!({}))).into_response()
        }
        Err(e) => {
            tracing::error!(error = %format!("{e:#}"), "storing Agent traces");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// The indexed encoding (`pkg/proto/datadog/trace/idx`): every string is a
/// reference into the payload's string table, attributes are typed values.
pub mod idx {
    use super::super::traces::{SpanRow, build};
    use prost::{Message, Oneof};
    use serde_json::{Map, Value, json};
    use std::collections::HashMap;

    #[derive(Clone, PartialEq, Message)]
    pub struct TracerPayload {
        #[prost(string, repeated, tag = "1")]
        pub strings: Vec<String>,
        #[prost(uint32, tag = "2")]
        pub container_id_ref: u32,
        #[prost(uint32, tag = "3")]
        pub language_name_ref: u32,
        #[prost(uint32, tag = "4")]
        pub language_version_ref: u32,
        #[prost(uint32, tag = "5")]
        pub tracer_version_ref: u32,
        #[prost(uint32, tag = "6")]
        pub runtime_id_ref: u32,
        #[prost(uint32, tag = "7")]
        pub env_ref: u32,
        #[prost(uint32, tag = "8")]
        pub hostname_ref: u32,
        #[prost(uint32, tag = "9")]
        pub app_version_ref: u32,
        #[prost(map = "uint32, message", tag = "10")]
        pub attributes: HashMap<u32, AnyValue>,
        #[prost(message, repeated, tag = "11")]
        pub chunks: Vec<TraceChunk>,
    }

    #[derive(Clone, PartialEq, Message)]
    pub struct TraceChunk {
        #[prost(int32, tag = "1")]
        pub priority: i32,
        #[prost(uint32, tag = "2")]
        pub origin_ref: u32,
        #[prost(map = "uint32, message", tag = "3")]
        pub attributes: HashMap<u32, AnyValue>,
        #[prost(message, repeated, tag = "4")]
        pub spans: Vec<Span>,
        #[prost(bool, tag = "5")]
        pub dropped_trace: bool,
        /// 16 bytes, big-endian: the high 64 bits, then the low ones.
        #[prost(bytes = "vec", tag = "6")]
        pub trace_id: Vec<u8>,
        #[prost(uint32, tag = "7")]
        pub sampling_mechanism: u32,
    }

    #[derive(Clone, PartialEq, Message)]
    pub struct Span {
        #[prost(uint32, tag = "1")]
        pub service_ref: u32,
        #[prost(uint32, tag = "2")]
        pub name_ref: u32,
        #[prost(uint32, tag = "3")]
        pub resource_ref: u32,
        #[prost(fixed64, tag = "4")]
        pub span_id: u64,
        #[prost(uint64, tag = "5")]
        pub parent_id: u64,
        #[prost(fixed64, tag = "6")]
        pub start: u64,
        #[prost(uint64, tag = "7")]
        pub duration: u64,
        #[prost(bool, tag = "8")]
        pub error: bool,
        #[prost(map = "uint32, message", tag = "9")]
        pub attributes: HashMap<u32, AnyValue>,
        #[prost(uint32, tag = "10")]
        pub type_ref: u32,
        #[prost(message, repeated, tag = "11")]
        pub links: Vec<SpanLink>,
        #[prost(message, repeated, tag = "12")]
        pub events: Vec<SpanEvent>,
        #[prost(uint32, tag = "13")]
        pub env_ref: u32,
        #[prost(uint32, tag = "14")]
        pub version_ref: u32,
        #[prost(uint32, tag = "15")]
        pub component_ref: u32,
        /// OpenTelemetry's SpanKind: 1 internal, 2 server, 3 client, 4 producer, 5 consumer.
        #[prost(int32, tag = "16")]
        pub kind: i32,
    }

    #[derive(Clone, PartialEq, Message)]
    pub struct SpanLink {
        #[prost(bytes = "vec", tag = "1")]
        pub trace_id: Vec<u8>,
        #[prost(fixed64, tag = "2")]
        pub span_id: u64,
        #[prost(map = "uint32, message", tag = "3")]
        pub attributes: HashMap<u32, AnyValue>,
        #[prost(uint32, tag = "4")]
        pub tracestate_ref: u32,
        #[prost(uint32, tag = "5")]
        pub flags: u32,
    }

    #[derive(Clone, PartialEq, Message)]
    pub struct SpanEvent {
        #[prost(fixed64, tag = "1")]
        pub time: u64,
        #[prost(uint32, tag = "2")]
        pub name_ref: u32,
        #[prost(map = "uint32, message", tag = "3")]
        pub attributes: HashMap<u32, AnyValue>,
    }

    #[derive(Clone, PartialEq, Message)]
    pub struct AnyValue {
        #[prost(oneof = "Any", tags = "1, 2, 3, 4, 5, 6, 7")]
        pub value: Option<Any>,
    }

    #[derive(Clone, PartialEq, Oneof)]
    pub enum Any {
        #[prost(uint32, tag = "1")]
        StringRef(u32),
        #[prost(bool, tag = "2")]
        Bool(bool),
        #[prost(double, tag = "3")]
        Double(f64),
        #[prost(int64, tag = "4")]
        Int(i64),
        #[prost(bytes, tag = "5")]
        Bytes(Vec<u8>),
        #[prost(message, tag = "6")]
        Array(ArrayValue),
        #[prost(message, tag = "7")]
        KeyValues(KeyValueList),
    }

    #[derive(Clone, PartialEq, Message)]
    pub struct ArrayValue {
        #[prost(message, repeated, tag = "1")]
        pub values: Vec<AnyValue>,
    }

    #[derive(Clone, PartialEq, Message)]
    pub struct KeyValueList {
        #[prost(message, repeated, tag = "1")]
        pub key_values: Vec<KeyValue>,
    }

    #[derive(Clone, PartialEq, Message)]
    pub struct KeyValue {
        #[prost(uint32, tag = "1")]
        pub key: u32,
        #[prost(message, optional, tag = "2")]
        pub value: Option<AnyValue>,
    }

    /// The low 64 bits as the decimal trace id, the high ones as `_dd.p.tid`
    /// (hex), as the v0.4 encoding carries them.
    pub fn split_trace_id(b: &[u8]) -> (u64, u64) {
        let word = |s: &[u8]| s.iter().fold(0u64, |acc, x| (acc << 8) | *x as u64);
        match b.len() {
            0 => (0, 0),
            n if n <= 8 => (0, word(b)),
            n => (word(&b[..n - 8]), word(&b[n - 8..])),
        }
    }

    struct Strings<'a>(&'a [String]);

    impl Strings<'_> {
        fn get(&self, r: u32) -> String {
            self.0.get(r as usize).cloned().unwrap_or_default()
        }

        fn json(&self, v: &AnyValue) -> Value {
            match &v.value {
                None => Value::Null,
                Some(Any::StringRef(r)) => Value::String(self.get(*r)),
                Some(Any::Bool(b)) => json!(b),
                Some(Any::Double(d)) => json!(d),
                Some(Any::Int(i)) => json!(i),
                Some(Any::Bytes(b)) => Value::String(String::from_utf8_lossy(b).into_owned()),
                Some(Any::Array(a)) => a.values.iter().map(|v| self.json(v)).collect(),
                Some(Any::KeyValues(kv)) => Value::Object(
                    kv.key_values
                        .iter()
                        .map(|k| {
                            let v = k.value.as_ref().map(|v| self.json(v)).unwrap_or(Value::Null);
                            (self.get(k.key), v)
                        })
                        .collect(),
                ),
            }
        }

        /// Numbers go to `metrics`, everything else to `meta` as text, as
        /// the v0.4 encoding splits them. Existing keys are kept.
        fn split_into(
            &self,
            attrs: &HashMap<u32, AnyValue>,
            meta: &mut Map<String, Value>,
            metrics: &mut Map<String, Value>,
        ) {
            for (k, v) in attrs {
                let key = self.get(*k);
                match &v.value {
                    Some(Any::Double(d)) if d.is_finite() => {
                        metrics.entry(key).or_insert(json!(d));
                    }
                    Some(Any::Int(i)) => {
                        metrics.entry(key).or_insert(json!(*i as f64));
                    }
                    _ => {
                        let text = match self.json(v) {
                            Value::String(s) => s,
                            other => other.to_string(),
                        };
                        meta.entry(key).or_insert(Value::String(text));
                    }
                }
            }
        }
    }

    const KINDS: [&str; 6] = ["", "internal", "server", "client", "producer", "consumer"];

    pub fn chunks(tp: &TracerPayload, agent_env: &str, agent_host: &str) -> Vec<Vec<SpanRow>> {
        let s = Strings(&tp.strings);
        let or = |a: String, b: &str| if a.is_empty() { b.to_string() } else { a };
        let payload_env = or(s.get(tp.env_ref), agent_env);
        let host = or(s.get(tp.hostname_ref), agent_host);
        let app_version = s.get(tp.app_version_ref);
        let language = s.get(tp.language_name_ref);
        let mut out = Vec::new();
        for chunk in &tp.chunks {
            let (high, low) = split_trace_id(&chunk.trace_id);
            let origin = s.get(chunk.origin_ref);
            let rows = chunk
                .spans
                .iter()
                .map(|span| {
                    let mut meta = Map::new();
                    let mut metrics = Map::new();
                    s.split_into(&span.attributes, &mut meta, &mut metrics);
                    s.split_into(&chunk.attributes, &mut meta, &mut metrics);
                    s.split_into(&tp.attributes, &mut meta, &mut metrics);
                    let mut put = |k: &str, v: String| {
                        if !v.is_empty() && !meta.contains_key(k) {
                            meta.insert(k.to_string(), Value::String(v));
                        }
                    };
                    put("env", or(s.get(span.env_ref), &payload_env));
                    put("version", or(s.get(span.version_ref), &app_version));
                    put("component", s.get(span.component_ref));
                    put("span.kind", KINDS.get(span.kind as usize).unwrap_or(&"").to_string());
                    put("_dd.hostname", host.clone());
                    put("_dd.origin", origin.clone());
                    put("language", language.clone());
                    if high != 0 {
                        put("_dd.p.tid", format!("{high:016x}"));
                    }
                    if !span.events.is_empty() {
                        let events: Vec<Value> = span
                            .events
                            .iter()
                            .map(|e| {
                                let attrs: Map<String, Value> = e
                                    .attributes
                                    .iter()
                                    .map(|(k, v)| (s.get(*k), s.json(v)))
                                    .collect();
                                json!({"name": s.get(e.name_ref), "time_unix_nano": e.time, "attributes": attrs})
                            })
                            .collect();
                        put("events", Value::Array(events).to_string());
                    }
                    if !span.links.is_empty() {
                        let links: Vec<Value> = span
                            .links
                            .iter()
                            .map(|l| {
                                let (hi, lo) = split_trace_id(&l.trace_id);
                                json!({"trace_id": format!("{hi:016x}{lo:016x}"), "span_id": l.span_id.to_string()})
                            })
                            .collect();
                        put("_dd.span_links", Value::Array(links).to_string());
                    }
                    metrics
                        .entry("_sampling_priority_v1")
                        .or_insert(json!(chunk.priority));
                    build(
                        low.to_string(),
                        span.span_id.to_string(),
                        span.parent_id.to_string(),
                        s.get(span.service_ref),
                        s.get(span.name_ref),
                        s.get(span.resource_ref),
                        Some(s.get(span.type_ref)),
                        span.start as i64,
                        span.duration as i64,
                        span.error,
                        meta,
                        metrics,
                    )
                })
                .collect();
            out.push(rows);
        }
        out
    }
}

// --- metrics -------------------------------------------------------------

#[derive(Clone, PartialEq, Message)]
pub struct MetricPayload {
    #[prost(message, repeated, tag = "1")]
    pub series: Vec<MetricSeries>,
}

#[derive(Clone, PartialEq, Message)]
pub struct MetricSeries {
    #[prost(message, repeated, tag = "1")]
    pub resources: Vec<Resource>,
    #[prost(string, tag = "2")]
    pub metric: String,
    #[prost(string, repeated, tag = "3")]
    pub tags: Vec<String>,
    #[prost(message, repeated, tag = "4")]
    pub points: Vec<MetricPoint>,
    /// 0 unspecified, 1 count, 2 rate, 3 gauge.
    #[prost(int32, tag = "5")]
    pub r#type: i32,
    #[prost(string, tag = "6")]
    pub unit: String,
    #[prost(string, tag = "7")]
    pub source_type_name: String,
    #[prost(int64, tag = "8")]
    pub interval: i64,
}

#[derive(Clone, PartialEq, Message)]
pub struct Resource {
    #[prost(string, tag = "1")]
    pub r#type: String,
    #[prost(string, tag = "2")]
    pub name: String,
}

#[derive(Clone, PartialEq, Message)]
pub struct MetricPoint {
    #[prost(double, tag = "1")]
    pub value: f64,
    #[prost(int64, tag = "2")]
    pub timestamp: i64,
}

/// A protobuf `MetricPayload`, the Agent's `/api/v2/series` body.
pub fn parse_series(body: &[u8]) -> anyhow::Result<Vec<Series>> {
    let p = MetricPayload::decode(body)?;
    Ok(p.series
        .into_iter()
        .map(|s| Series {
            kind: match s.r#type {
                1 => Kind::Count,
                2 => Kind::Rate,
                _ => Kind::Gauge,
            },
            host: s
                .resources
                .iter()
                .find(|r| r.r#type == "host" && !r.name.is_empty())
                .map(|r| r.name.clone()),
            tags: normalize_tags(s.tags),
            points: s
                .points
                .into_iter()
                .map(|p| (p.timestamp, p.value))
                .collect(),
            metric: s.metric,
        })
        .collect())
}

#[derive(Clone, PartialEq, Message)]
pub struct SketchPayload {
    #[prost(message, repeated, tag = "1")]
    pub sketches: Vec<Sketch>,
}

#[derive(Clone, PartialEq, Message)]
pub struct Sketch {
    #[prost(string, tag = "1")]
    pub metric: String,
    #[prost(string, tag = "2")]
    pub host: String,
    #[prost(string, repeated, tag = "4")]
    pub tags: Vec<String>,
    #[prost(message, repeated, tag = "7")]
    pub dogsketches: Vec<Dogsketch>,
}

#[derive(Clone, PartialEq, Message)]
pub struct Dogsketch {
    /// Epoch seconds.
    #[prost(int64, tag = "1")]
    pub ts: i64,
    #[prost(int64, tag = "2")]
    pub cnt: i64,
    #[prost(double, tag = "3")]
    pub min: f64,
    #[prost(double, tag = "4")]
    pub max: f64,
    #[prost(double, tag = "5")]
    pub avg: f64,
    #[prost(double, tag = "6")]
    pub sum: f64,
    /// Bin keys, paired with `n`.
    #[prost(sint32, repeated, tag = "7")]
    pub k: Vec<i32>,
    #[prost(uint32, repeated, tag = "8")]
    pub n: Vec<u32>,
}

/// The value of a DDSketch bin key, with the Agent's default mapping
/// (`pkg/util/quantile/config.go`: relative accuracy 1/128, minimum 1e-9):
/// `γ^(k − bias)` with `γ = 1 + 2/128`.
pub fn sketch_value(k: i32) -> f64 {
    const MAX_KEY: i32 = (1 << 15) - 1;
    match k {
        0 => 0.0,
        k if k < 0 => -sketch_value(-k),
        k if k >= MAX_KEY => f64::INFINITY,
        k => {
            let ln_gamma = (2.0f64 / 128.0).ln_1p();
            let emin = (1e-9f64.ln() / ln_gamma).floor() as i32;
            let bias = -emin + 1;
            ((k - bias) as f64 * ln_gamma).exp()
        }
    }
}

/// One weighted point per bin, clamped to the sketch's exact min and max.
pub fn sketch_points(p: SketchPayload) -> (Vec<Point>, Vec<String>) {
    let mut points = Vec::new();
    let mut names = Vec::new();
    for s in p.sketches {
        if !valid_name(&s.metric) {
            continue;
        }
        let tags = normalize_tags(s.tags);
        let host = (!s.host.is_empty()).then(|| s.host.clone());
        for d in &s.dogsketches {
            let mut bins: BTreeMap<i32, u64> = BTreeMap::new();
            for (k, n) in d.k.iter().zip(&d.n) {
                *bins.entry(*k).or_default() += *n as u64;
            }
            for (k, n) in bins {
                let v = sketch_value(k);
                if n == 0 || !v.is_finite() {
                    continue;
                }
                points.push(Point {
                    ts: d.ts * 1000,
                    metric: s.metric.clone(),
                    host: host.clone(),
                    tags: tags.clone(),
                    value: if d.min <= d.max { v.clamp(d.min, d.max) } else { v },
                    weight: n as f64,
                });
            }
        }
        names.push(s.metric);
    }
    (points, names)
}

pub async fn sketches(
    State(st): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
    body: Bytes,
) -> Response {
    if !api_key_ok(&st, &headers, &q) {
        return forbidden();
    }
    let payload = match decompress(&headers, body)
        .map_err(anyhow::Error::from)
        .and_then(|b| Ok(SketchPayload::decode(&b[..])?))
    {
        Ok(p) => p,
        Err(e) => return bad_request(e),
    };
    let (points, names) = sketch_points(payload);
    let kinds: Vec<(String, String)> = names
        .into_iter()
        .collect::<HashSet<_>>()
        .into_iter()
        .map(|n| (n, Kind::Distribution.as_str().to_string()))
        .collect();
    let stored = async {
        st.store.pg.touch_metrics(&kinds).await?;
        st.store.points.send(points).await
    };
    match stored.await {
        Ok(()) => (StatusCode::ACCEPTED, Json(json!({"errors": []}))).into_response(),
        Err(e) => {
            tracing::error!(error = %format!("{e:#}"), "storing sketches");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

// --- the rest --------------------------------------------------------------

static SEEN: Mutex<Option<HashSet<String>>> = Mutex::new(None);

/// Anything else an Agent posts (host metadata, check runs, APM stats…):
/// accepted, so the Agent stays healthy, and logged the first time.
pub async fn acknowledge(
    State(st): State<AppState>,
    method: Method,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    if !api_key_ok(&st, &headers, &q) {
        return forbidden();
    }
    let key = format!("{method} {}", uri.path());
    let first = SEEN
        .lock()
        .map(|mut g| g.get_or_insert_with(HashSet::new).insert(key.clone()))
        .unwrap_or(false);
    if first {
        tracing::info!(endpoint = %key, "Agent endpoint acknowledged but not used");
    }
    (StatusCode::ACCEPTED, Json(json!({"status": "ok"}))).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_an_agent_trace_payload() {
        let span = Span {
            service: "web".into(),
            name: "http.request".into(),
            resource: "GET /".into(),
            trace_id: 42,
            span_id: 7,
            start: 1_700_000_000_000_000_000,
            duration: 1_000_000,
            error: 1,
            metrics: HashMap::from([("_top_level".to_string(), 1.0)]),
            ..Default::default()
        };
        let payload = AgentPayload {
            host_name: "agent-host".into(),
            env: "prod".into(),
            tracer_payloads: vec![TracerPayload {
                app_version: "1.2.3".into(),
                language_name: "python".into(),
                chunks: vec![TraceChunk {
                    priority: 2,
                    spans: vec![span],
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        };
        let chunks = decode_traces(&payload.encode_to_vec()).unwrap();
        let s = &chunks[0][0];
        assert_eq!((s.trace_id.as_str(), s.span_id.as_str()), ("42", "7"));
        assert_eq!(s.env.as_deref(), Some("prod"));
        assert_eq!(s.version.as_deref(), Some("1.2.3"));
        assert!(s.error && s.top_level);
        assert_eq!(s.meta["_dd.hostname"], "agent-host");
        assert_eq!(s.metrics["_sampling_priority_v1"], 2);
    }

    #[test]
    fn decodes_the_indexed_trace_format() {
        use idx::{Any, AnyValue};
        let strings: Vec<String> = ["", "web", "http.request", "GET /", "prod", "h1", "http.status_code", "500", "_top_level", "1.4.0"]
            .map(String::from)
            .to_vec();
        let attr = |v| AnyValue { value: Some(v) };
        let span = idx::Span {
            service_ref: 1,
            name_ref: 2,
            resource_ref: 3,
            span_id: 7,
            start: 1_700_000_000_000_000_000,
            duration: 5_000,
            error: true,
            attributes: HashMap::from([(6, attr(Any::StringRef(7))), (8, attr(Any::Double(1.0)))]),
            version_ref: 9,
            kind: 2,
            ..Default::default()
        };
        let mut trace_id = vec![0u8; 16];
        trace_id[7] = 1; // high 64 bits = 1
        trace_id[15] = 42; // low 64 bits = 42
        let payload = AgentPayload {
            idx_tracer_payloads: vec![idx::TracerPayload {
                strings,
                env_ref: 4,
                hostname_ref: 5,
                chunks: vec![idx::TraceChunk {
                    priority: 1,
                    spans: vec![span],
                    trace_id,
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        };
        let chunks = decode_traces(&payload.encode_to_vec()).unwrap();
        let s = &chunks[0][0];
        assert_eq!((s.service.as_str(), s.resource.as_str()), ("web", "GET /"));
        assert_eq!(s.trace_id, "42");
        assert_eq!(s.meta["_dd.p.tid"], "0000000000000001");
        assert_eq!((s.env.as_deref(), s.version.as_deref()), (Some("prod"), Some("1.4.0")));
        assert_eq!(s.meta["http.status_code"], "500");
        assert_eq!(s.meta["span.kind"], "server");
        assert_eq!(s.meta["_dd.hostname"], "h1");
        assert!(s.error && s.top_level);
    }

    #[test]
    fn decodes_agent_series() {
        let payload = MetricPayload {
            series: vec![MetricSeries {
                metric: "system.load.1".into(),
                tags: vec!["env:prod".into()],
                points: vec![MetricPoint {
                    value: 0.5,
                    timestamp: 1_700_000_000,
                }],
                r#type: 3,
                resources: vec![Resource {
                    r#type: "host".into(),
                    name: "h1".into(),
                }],
                ..Default::default()
            }],
        };
        let s = parse_series(&payload.encode_to_vec()).unwrap();
        assert_eq!(s[0].kind, Kind::Gauge);
        assert_eq!(s[0].host.as_deref(), Some("h1"));
        assert_eq!(s[0].points, vec![(1_700_000_000, 0.5)]);
    }

    #[test]
    fn sketch_bins_map_back_to_values() {
        // The Agent's key for a value v is round(log_γ v) + bias.
        let ln_gamma = (2.0f64 / 128.0).ln_1p();
        let bias = -((1e-9f64.ln() / ln_gamma).floor() as i32) + 1;
        for v in [0.001f64, 0.25, 1.0, 42.0, 1234.5] {
            let k = (v.ln() / ln_gamma).round() as i32 + bias;
            let back = sketch_value(k);
            assert!((back - v).abs() / v < 0.01, "{v} → {k} → {back}");
            assert!((sketch_value(-k) + back).abs() < 1e-12);
        }
        let payload = SketchPayload {
            sketches: vec![Sketch {
                metric: "req.latency".into(),
                dogsketches: vec![Dogsketch {
                    ts: 10,
                    cnt: 5,
                    min: 1.0,
                    max: 2.0,
                    k: vec![1338, 1338, 1383],
                    n: vec![2, 1, 2],
                    ..Default::default()
                }],
                ..Default::default()
            }],
        };
        let (points, names) = sketch_points(payload);
        assert_eq!(names, vec!["req.latency"]);
        assert_eq!(points.len(), 2);
        assert_eq!((points[0].value, points[0].weight, points[0].ts), (1.0, 3.0, 10_000));
        assert_eq!(points[1].value, 2.0);
    }
}
