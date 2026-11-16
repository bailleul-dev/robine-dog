//! OTLP/HTTP intake: `POST /v1/traces`, `/v1/logs` and `/v1/metrics`, in
//! protobuf or JSON (gzip or not), mapped the way Datadog's OTLP ingestion
//! maps OpenTelemetry data:
//! - resources: `service.name` → service, `deployment.environment(.name)` →
//!   env, `service.version` → version, `host.name` → host;
//! - spans: 128-bit trace ids keep their low 64 bits (the high ones go to
//!   `_dd.p.tid`), operation and resource names come from the semantic
//!   conventions (`http.server.request`, `GET /users/{id}`…), status ERROR
//!   marks the span as an error and an `exception` event fills `error.*`;
//! - logs: the severity becomes the status, the body the message;
//! - metrics: gauges, sums (cumulative ones turned into deltas, as the Agent
//!   does), histograms and summaries (`.count`, `.sum`, `.min`, `.max`,
//!   `.quantile`).

use super::logs::{Defaults, api_key_ok, decompress, now_ms, process};
use super::series::{self, Series};
use super::traces::{self, SpanRow};
use crate::AppState;
use crate::metrics::Kind;
use axum::body::Bytes;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use prost::Message;
use serde_json::{Map, Value, json};
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

// --- protobuf messages (opentelemetry-proto v1) ----------------------------------------

#[derive(Clone, PartialEq, Message)]
pub struct AnyValue {
    #[prost(oneof = "any_value::Value", tags = "1, 2, 3, 4, 5, 6, 7")]
    pub value: Option<any_value::Value>,
}

/// Variant names follow the OTLP protobuf definitions.
#[allow(clippy::enum_variant_names)]
pub mod any_value {
    #[derive(Clone, PartialEq, prost::Oneof)]
    pub enum Value {
        #[prost(string, tag = "1")]
        StringValue(String),
        #[prost(bool, tag = "2")]
        BoolValue(bool),
        #[prost(int64, tag = "3")]
        IntValue(i64),
        #[prost(double, tag = "4")]
        DoubleValue(f64),
        #[prost(message, tag = "5")]
        ArrayValue(super::ArrayValue),
        #[prost(message, tag = "6")]
        KvlistValue(super::KeyValueList),
        #[prost(bytes = "vec", tag = "7")]
        BytesValue(Vec<u8>),
    }
}

#[derive(Clone, PartialEq, Message)]
pub struct ArrayValue {
    #[prost(message, repeated, tag = "1")]
    pub values: Vec<AnyValue>,
}

#[derive(Clone, PartialEq, Message)]
pub struct KeyValueList {
    #[prost(message, repeated, tag = "1")]
    pub values: Vec<KeyValue>,
}

#[derive(Clone, PartialEq, Message)]
pub struct KeyValue {
    #[prost(string, tag = "1")]
    pub key: String,
    #[prost(message, optional, tag = "2")]
    pub value: Option<AnyValue>,
}

#[derive(Clone, PartialEq, Message)]
pub struct Resource {
    #[prost(message, repeated, tag = "1")]
    pub attributes: Vec<KeyValue>,
}

#[derive(Clone, PartialEq, Message)]
pub struct Scope {
    #[prost(string, tag = "1")]
    pub name: String,
    #[prost(string, tag = "2")]
    pub version: String,
}

#[derive(Clone, PartialEq, Message)]
pub struct TracesRequest {
    #[prost(message, repeated, tag = "1")]
    pub resource_spans: Vec<ResourceSpans>,
}

#[derive(Clone, PartialEq, Message)]
pub struct ResourceSpans {
    #[prost(message, optional, tag = "1")]
    pub resource: Option<Resource>,
    #[prost(message, repeated, tag = "2")]
    pub scope_spans: Vec<ScopeSpans>,
}

#[derive(Clone, PartialEq, Message)]
pub struct ScopeSpans {
    #[prost(message, optional, tag = "1")]
    pub scope: Option<Scope>,
    #[prost(message, repeated, tag = "2")]
    pub spans: Vec<Span>,
}

#[derive(Clone, PartialEq, Message)]
pub struct Span {
    #[prost(bytes = "vec", tag = "1")]
    pub trace_id: Vec<u8>,
    #[prost(bytes = "vec", tag = "2")]
    pub span_id: Vec<u8>,
    #[prost(bytes = "vec", tag = "4")]
    pub parent_span_id: Vec<u8>,
    #[prost(string, tag = "5")]
    pub name: String,
    #[prost(int32, tag = "6")]
    pub kind: i32,
    #[prost(fixed64, tag = "7")]
    pub start_time_unix_nano: u64,
    #[prost(fixed64, tag = "8")]
    pub end_time_unix_nano: u64,
    #[prost(message, repeated, tag = "9")]
    pub attributes: Vec<KeyValue>,
    #[prost(message, repeated, tag = "11")]
    pub events: Vec<SpanEvent>,
    #[prost(message, optional, tag = "15")]
    pub status: Option<SpanStatus>,
}

#[derive(Clone, PartialEq, Message)]
pub struct SpanEvent {
    #[prost(fixed64, tag = "1")]
    pub time_unix_nano: u64,
    #[prost(string, tag = "2")]
    pub name: String,
    #[prost(message, repeated, tag = "3")]
    pub attributes: Vec<KeyValue>,
}

#[derive(Clone, PartialEq, Message)]
pub struct SpanStatus {
    #[prost(string, tag = "2")]
    pub message: String,
    #[prost(int32, tag = "3")]
    pub code: i32,
}

#[derive(Clone, PartialEq, Message)]
pub struct LogsRequest {
    #[prost(message, repeated, tag = "1")]
    pub resource_logs: Vec<ResourceLogs>,
}

#[derive(Clone, PartialEq, Message)]
pub struct ResourceLogs {
    #[prost(message, optional, tag = "1")]
    pub resource: Option<Resource>,
    #[prost(message, repeated, tag = "2")]
    pub scope_logs: Vec<ScopeLogs>,
}

#[derive(Clone, PartialEq, Message)]
pub struct ScopeLogs {
    #[prost(message, optional, tag = "1")]
    pub scope: Option<Scope>,
    #[prost(message, repeated, tag = "2")]
    pub log_records: Vec<LogRecord>,
}

#[derive(Clone, PartialEq, Message)]
pub struct LogRecord {
    #[prost(fixed64, tag = "1")]
    pub time_unix_nano: u64,
    #[prost(int32, tag = "2")]
    pub severity_number: i32,
    #[prost(string, tag = "3")]
    pub severity_text: String,
    #[prost(message, optional, tag = "5")]
    pub body: Option<AnyValue>,
    #[prost(message, repeated, tag = "6")]
    pub attributes: Vec<KeyValue>,
    #[prost(bytes = "vec", tag = "9")]
    pub trace_id: Vec<u8>,
    #[prost(bytes = "vec", tag = "10")]
    pub span_id: Vec<u8>,
    #[prost(fixed64, tag = "11")]
    pub observed_time_unix_nano: u64,
}

#[derive(Clone, PartialEq, Message)]
pub struct MetricsRequest {
    #[prost(message, repeated, tag = "1")]
    pub resource_metrics: Vec<ResourceMetrics>,
}

#[derive(Clone, PartialEq, Message)]
pub struct ResourceMetrics {
    #[prost(message, optional, tag = "1")]
    pub resource: Option<Resource>,
    #[prost(message, repeated, tag = "2")]
    pub scope_metrics: Vec<ScopeMetrics>,
}

#[derive(Clone, PartialEq, Message)]
pub struct ScopeMetrics {
    #[prost(message, optional, tag = "1")]
    pub scope: Option<Scope>,
    #[prost(message, repeated, tag = "2")]
    pub metrics: Vec<Metric>,
}

#[derive(Clone, PartialEq, Message)]
pub struct Metric {
    #[prost(string, tag = "1")]
    pub name: String,
    #[prost(string, tag = "3")]
    pub unit: String,
    #[prost(oneof = "metric::Data", tags = "5, 7, 9, 10, 11")]
    pub data: Option<metric::Data>,
}

pub mod metric {
    #[derive(Clone, PartialEq, prost::Oneof)]
    pub enum Data {
        #[prost(message, tag = "5")]
        Gauge(super::Gauge),
        #[prost(message, tag = "7")]
        Sum(super::Sum),
        #[prost(message, tag = "9")]
        Histogram(super::Histogram),
        #[prost(message, tag = "10")]
        ExponentialHistogram(super::ExponentialHistogram),
        #[prost(message, tag = "11")]
        Summary(super::Summary),
    }
}

#[derive(Clone, PartialEq, Message)]
pub struct Gauge {
    #[prost(message, repeated, tag = "1")]
    pub data_points: Vec<NumberDataPoint>,
}

#[derive(Clone, PartialEq, Message)]
pub struct Sum {
    #[prost(message, repeated, tag = "1")]
    pub data_points: Vec<NumberDataPoint>,
    #[prost(int32, tag = "2")]
    pub aggregation_temporality: i32,
    #[prost(bool, tag = "3")]
    pub is_monotonic: bool,
}

#[derive(Clone, PartialEq, Message)]
pub struct Histogram {
    #[prost(message, repeated, tag = "1")]
    pub data_points: Vec<HistogramDataPoint>,
    #[prost(int32, tag = "2")]
    pub aggregation_temporality: i32,
}

#[derive(Clone, PartialEq, Message)]
pub struct ExponentialHistogram {
    #[prost(message, repeated, tag = "1")]
    pub data_points: Vec<ExpHistogramDataPoint>,
    #[prost(int32, tag = "2")]
    pub aggregation_temporality: i32,
}

#[derive(Clone, PartialEq, Message)]
pub struct Summary {
    #[prost(message, repeated, tag = "1")]
    pub data_points: Vec<SummaryDataPoint>,
}

#[derive(Clone, PartialEq, Message)]
pub struct NumberDataPoint {
    #[prost(message, repeated, tag = "7")]
    pub attributes: Vec<KeyValue>,
    #[prost(fixed64, tag = "2")]
    pub start_time_unix_nano: u64,
    #[prost(fixed64, tag = "3")]
    pub time_unix_nano: u64,
    #[prost(oneof = "number_data_point::Value", tags = "4, 6")]
    pub value: Option<number_data_point::Value>,
}

pub mod number_data_point {
    #[derive(Clone, PartialEq, prost::Oneof)]
    pub enum Value {
        #[prost(double, tag = "4")]
        AsDouble(f64),
        #[prost(sfixed64, tag = "6")]
        AsInt(i64),
    }
}

#[derive(Clone, PartialEq, Message)]
pub struct HistogramDataPoint {
    #[prost(message, repeated, tag = "9")]
    pub attributes: Vec<KeyValue>,
    #[prost(fixed64, tag = "2")]
    pub start_time_unix_nano: u64,
    #[prost(fixed64, tag = "3")]
    pub time_unix_nano: u64,
    #[prost(fixed64, tag = "4")]
    pub count: u64,
    #[prost(double, optional, tag = "5")]
    pub sum: Option<f64>,
    #[prost(double, optional, tag = "11")]
    pub min: Option<f64>,
    #[prost(double, optional, tag = "12")]
    pub max: Option<f64>,
}

#[derive(Clone, PartialEq, Message)]
pub struct ExpHistogramDataPoint {
    #[prost(message, repeated, tag = "1")]
    pub attributes: Vec<KeyValue>,
    #[prost(fixed64, tag = "2")]
    pub start_time_unix_nano: u64,
    #[prost(fixed64, tag = "3")]
    pub time_unix_nano: u64,
    #[prost(fixed64, tag = "4")]
    pub count: u64,
    #[prost(double, optional, tag = "5")]
    pub sum: Option<f64>,
    #[prost(double, optional, tag = "12")]
    pub min: Option<f64>,
    #[prost(double, optional, tag = "13")]
    pub max: Option<f64>,
}

#[derive(Clone, PartialEq, Message)]
pub struct SummaryDataPoint {
    #[prost(message, repeated, tag = "7")]
    pub attributes: Vec<KeyValue>,
    #[prost(fixed64, tag = "2")]
    pub start_time_unix_nano: u64,
    #[prost(fixed64, tag = "3")]
    pub time_unix_nano: u64,
    #[prost(fixed64, tag = "4")]
    pub count: u64,
    #[prost(double, tag = "5")]
    pub sum: f64,
    #[prost(message, repeated, tag = "6")]
    pub quantile_values: Vec<ValueAtQuantile>,
}

#[derive(Clone, PartialEq, Message)]
pub struct ValueAtQuantile {
    #[prost(double, tag = "1")]
    pub quantile: f64,
    #[prost(double, tag = "2")]
    pub value: f64,
}

// --- OTLP/JSON → the same messages ------------------------------------------------------
// (lowerCamelCase fields, hex ids, 64-bit integers as strings, enums as numbers)

mod js {
    use super::*;

    pub fn get<'a>(v: &'a Value, camel: &str) -> Option<&'a Value> {
        v.get(camel).or_else(|| {
            let snake: String = camel
                .chars()
                .flat_map(|c| {
                    if c.is_ascii_uppercase() {
                        vec!['_', c.to_ascii_lowercase()]
                    } else {
                        vec![c]
                    }
                })
                .collect();
            v.get(snake.as_str())
        })
    }

    pub fn arr<'a>(v: &'a Value, k: &str) -> &'a [Value] {
        get(v, k)
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    pub fn s(v: &Value, k: &str) -> String {
        get(v, k).and_then(Value::as_str).unwrap_or("").to_string()
    }

    pub fn u(v: &Value, k: &str) -> u64 {
        match get(v, k) {
            Some(Value::String(x)) => x.parse().unwrap_or(0),
            Some(Value::Number(n)) => n
                .as_u64()
                .or_else(|| n.as_f64().map(|f| f as u64))
                .unwrap_or(0),
            _ => 0,
        }
    }

    pub fn i(v: &Value, k: &str) -> i32 {
        match get(v, k) {
            Some(Value::Number(n)) => n.as_i64().unwrap_or(0) as i32,
            Some(Value::String(x)) => x.parse().unwrap_or(match x.as_str() {
                "SPAN_KIND_INTERNAL" | "STATUS_CODE_OK" | "AGGREGATION_TEMPORALITY_DELTA" => 1,
                "SPAN_KIND_SERVER" | "STATUS_CODE_ERROR" | "AGGREGATION_TEMPORALITY_CUMULATIVE" => {
                    2
                }
                "SPAN_KIND_CLIENT" => 3,
                "SPAN_KIND_PRODUCER" => 4,
                "SPAN_KIND_CONSUMER" => 5,
                _ => 0,
            }),
            _ => 0,
        }
    }

    pub fn f(v: &Value, k: &str) -> Option<f64> {
        match get(v, k) {
            Some(Value::Number(n)) => n.as_f64(),
            Some(Value::String(x)) => x.parse().ok(),
            _ => None,
        }
    }

    /// OTLP/JSON ids are hex; base64 is accepted too.
    pub fn id(v: &Value, k: &str) -> Vec<u8> {
        let t = s(v, k);
        if t.len().is_multiple_of(2) && t.chars().all(|c| c.is_ascii_hexdigit()) {
            (0..t.len())
                .step_by(2)
                .filter_map(|i| u8::from_str_radix(&t[i..i + 2], 16).ok())
                .collect()
        } else {
            use base64::Engine;
            base64::engine::general_purpose::STANDARD
                .decode(t)
                .unwrap_or_default()
        }
    }

    pub fn any(v: &Value) -> AnyValue {
        use any_value::Value as A;
        let value = if let Some(x) = get(v, "stringValue").and_then(Value::as_str) {
            Some(A::StringValue(x.to_string()))
        } else if let Some(x) = get(v, "boolValue").and_then(Value::as_bool) {
            Some(A::BoolValue(x))
        } else if get(v, "intValue").is_some() {
            Some(A::IntValue(match get(v, "intValue") {
                Some(Value::String(x)) => x.parse().unwrap_or(0),
                Some(Value::Number(n)) => n.as_i64().unwrap_or(0),
                _ => 0,
            }))
        } else if let Some(x) = f(v, "doubleValue") {
            Some(A::DoubleValue(x))
        } else if let Some(a) = get(v, "arrayValue") {
            Some(A::ArrayValue(ArrayValue {
                values: arr(a, "values").iter().map(any).collect(),
            }))
        } else if let Some(a) = get(v, "kvlistValue") {
            Some(A::KvlistValue(KeyValueList {
                values: kvs(a, "values"),
            }))
        } else {
            get(v, "bytesValue")
                .and_then(Value::as_str)
                .map(|b| A::StringValue(b.to_string()))
        };
        AnyValue { value }
    }

    pub fn kvs(v: &Value, k: &str) -> Vec<KeyValue> {
        arr(v, k)
            .iter()
            .map(|kv| KeyValue {
                key: s(kv, "key"),
                value: get(kv, "value").map(any),
            })
            .collect()
    }

    pub fn resource(v: &Value) -> Option<Resource> {
        get(v, "resource").map(|r| Resource {
            attributes: kvs(r, "attributes"),
        })
    }

    pub fn scope(v: &Value) -> Option<Scope> {
        get(v, "scope").map(|sc| Scope {
            name: s(sc, "name"),
            version: s(sc, "version"),
        })
    }

    pub fn traces(v: &Value) -> TracesRequest {
        TracesRequest {
            resource_spans: arr(v, "resourceSpans")
                .iter()
                .map(|rs| ResourceSpans {
                    resource: resource(rs),
                    scope_spans: arr(rs, "scopeSpans")
                        .iter()
                        .map(|ss| ScopeSpans {
                            scope: scope(ss),
                            spans: arr(ss, "spans")
                                .iter()
                                .map(|sp| Span {
                                    trace_id: id(sp, "traceId"),
                                    span_id: id(sp, "spanId"),
                                    parent_span_id: id(sp, "parentSpanId"),
                                    name: s(sp, "name"),
                                    kind: i(sp, "kind"),
                                    start_time_unix_nano: u(sp, "startTimeUnixNano"),
                                    end_time_unix_nano: u(sp, "endTimeUnixNano"),
                                    attributes: kvs(sp, "attributes"),
                                    events: arr(sp, "events")
                                        .iter()
                                        .map(|e| SpanEvent {
                                            time_unix_nano: u(e, "timeUnixNano"),
                                            name: s(e, "name"),
                                            attributes: kvs(e, "attributes"),
                                        })
                                        .collect(),
                                    status: get(sp, "status").map(|st| SpanStatus {
                                        message: s(st, "message"),
                                        code: i(st, "code"),
                                    }),
                                })
                                .collect(),
                        })
                        .collect(),
                })
                .collect(),
        }
    }

    pub fn logs(v: &Value) -> LogsRequest {
        LogsRequest {
            resource_logs: arr(v, "resourceLogs")
                .iter()
                .map(|rl| ResourceLogs {
                    resource: resource(rl),
                    scope_logs: arr(rl, "scopeLogs")
                        .iter()
                        .map(|sl| ScopeLogs {
                            scope: scope(sl),
                            log_records: arr(sl, "logRecords")
                                .iter()
                                .map(|lr| LogRecord {
                                    time_unix_nano: u(lr, "timeUnixNano"),
                                    severity_number: i(lr, "severityNumber"),
                                    severity_text: s(lr, "severityText"),
                                    body: get(lr, "body").map(any),
                                    attributes: kvs(lr, "attributes"),
                                    trace_id: id(lr, "traceId"),
                                    span_id: id(lr, "spanId"),
                                    observed_time_unix_nano: u(lr, "observedTimeUnixNano"),
                                })
                                .collect(),
                        })
                        .collect(),
                })
                .collect(),
        }
    }

    fn number_points(v: &Value) -> Vec<NumberDataPoint> {
        arr(v, "dataPoints")
            .iter()
            .map(|p| NumberDataPoint {
                attributes: kvs(p, "attributes"),
                start_time_unix_nano: u(p, "startTimeUnixNano"),
                time_unix_nano: u(p, "timeUnixNano"),
                value: if get(p, "asInt").is_some() {
                    Some(number_data_point::Value::AsInt(u(p, "asInt") as i64))
                } else {
                    f(p, "asDouble").map(number_data_point::Value::AsDouble)
                },
            })
            .collect()
    }

    pub fn metrics(v: &Value) -> MetricsRequest {
        MetricsRequest {
            resource_metrics: arr(v, "resourceMetrics")
                .iter()
                .map(|rm| ResourceMetrics {
                    resource: resource(rm),
                    scope_metrics: arr(rm, "scopeMetrics")
                        .iter()
                        .map(|sm| ScopeMetrics {
                            scope: scope(sm),
                            metrics: arr(sm, "metrics")
                                .iter()
                                .map(|m| Metric {
                                    name: s(m, "name"),
                                    unit: s(m, "unit"),
                                    data: if let Some(g) = get(m, "gauge") {
                                        Some(metric::Data::Gauge(Gauge {
                                            data_points: number_points(g),
                                        }))
                                    } else if let Some(x) = get(m, "sum") {
                                        Some(metric::Data::Sum(Sum {
                                            data_points: number_points(x),
                                            aggregation_temporality: i(x, "aggregationTemporality"),
                                            is_monotonic: get(x, "isMonotonic")
                                                .and_then(Value::as_bool)
                                                .unwrap_or(false),
                                        }))
                                    } else if let Some(h) = get(m, "histogram") {
                                        Some(metric::Data::Histogram(Histogram {
                                            aggregation_temporality: i(h, "aggregationTemporality"),
                                            data_points: arr(h, "dataPoints")
                                                .iter()
                                                .map(|p| HistogramDataPoint {
                                                    attributes: kvs(p, "attributes"),
                                                    start_time_unix_nano: u(p, "startTimeUnixNano"),
                                                    time_unix_nano: u(p, "timeUnixNano"),
                                                    count: u(p, "count"),
                                                    sum: f(p, "sum"),
                                                    min: f(p, "min"),
                                                    max: f(p, "max"),
                                                })
                                                .collect(),
                                        }))
                                    } else if let Some(h) = get(m, "exponentialHistogram") {
                                        Some(metric::Data::ExponentialHistogram(
                                            ExponentialHistogram {
                                                aggregation_temporality: i(
                                                    h,
                                                    "aggregationTemporality",
                                                ),
                                                data_points: arr(h, "dataPoints")
                                                    .iter()
                                                    .map(|p| ExpHistogramDataPoint {
                                                        attributes: kvs(p, "attributes"),
                                                        start_time_unix_nano: u(
                                                            p,
                                                            "startTimeUnixNano",
                                                        ),
                                                        time_unix_nano: u(p, "timeUnixNano"),
                                                        count: u(p, "count"),
                                                        sum: f(p, "sum"),
                                                        min: f(p, "min"),
                                                        max: f(p, "max"),
                                                    })
                                                    .collect(),
                                            },
                                        ))
                                    } else {
                                        get(m, "summary").map(|x| {
                                            metric::Data::Summary(Summary {
                                                data_points: arr(x, "dataPoints")
                                                    .iter()
                                                    .map(|p| SummaryDataPoint {
                                                        attributes: kvs(p, "attributes"),
                                                        start_time_unix_nano: u(
                                                            p,
                                                            "startTimeUnixNano",
                                                        ),
                                                        time_unix_nano: u(p, "timeUnixNano"),
                                                        count: u(p, "count"),
                                                        sum: f(p, "sum").unwrap_or(0.0),
                                                        quantile_values: arr(p, "quantileValues")
                                                            .iter()
                                                            .map(|q| ValueAtQuantile {
                                                                quantile: f(q, "quantile")
                                                                    .unwrap_or(0.0),
                                                                value: f(q, "value").unwrap_or(0.0),
                                                            })
                                                            .collect(),
                                                    })
                                                    .collect(),
                                            })
                                        })
                                    },
                                })
                                .collect(),
                        })
                        .collect(),
                })
                .collect(),
        }
    }
}

// --- mapping ---------------------------------------------------------------------------

fn any_to_json(v: &AnyValue) -> Value {
    use any_value::Value as A;
    match &v.value {
        Some(A::StringValue(s)) => json!(s),
        Some(A::BoolValue(b)) => json!(b),
        Some(A::IntValue(i)) => json!(i),
        Some(A::DoubleValue(d)) => json!(d),
        Some(A::ArrayValue(a)) => Value::Array(a.values.iter().map(any_to_json).collect()),
        Some(A::KvlistValue(l)) => Value::Object(attrs(&l.values)),
        Some(A::BytesValue(b)) => {
            use base64::Engine;
            json!(base64::engine::general_purpose::STANDARD.encode(b))
        }
        None => Value::Null,
    }
}

fn attrs(kvs: &[KeyValue]) -> Map<String, Value> {
    kvs.iter()
        .map(|kv| {
            (
                kv.key.clone(),
                kv.value.as_ref().map(any_to_json).unwrap_or(Value::Null),
            )
        })
        .collect()
}

fn text(v: &Value) -> Option<String> {
    match v {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// Service, env, version and host of a resource.
struct Res {
    attrs: Map<String, Value>,
    service: Option<String>,
    env: Option<String>,
    version: Option<String>,
    host: Option<String>,
}

fn res(r: &Option<Resource>) -> Res {
    let attrs = r.as_ref().map(|r| attrs(&r.attributes)).unwrap_or_default();
    let get = |keys: &[&str]| keys.iter().find_map(|k| attrs.get(*k).and_then(text));
    Res {
        service: get(&["service.name"]).filter(|s| !s.starts_with("unknown_service")),
        env: get(&["deployment.environment.name", "deployment.environment"]),
        version: get(&["service.version"]),
        host: get(&["host.name", "k8s.node.name"]),
        attrs: attrs.clone(),
    }
}

/// The low 64 bits of an id, as a decimal string ("0" when absent).
fn low64(id: &[u8]) -> String {
    if id.is_empty() {
        return "0".into();
    }
    let tail = &id[id.len().saturating_sub(8)..];
    tail.iter()
        .fold(0u64, |acc, b| (acc << 8) | *b as u64)
        .to_string()
}

fn hex(id: &[u8]) -> String {
    id.iter().map(|b| format!("{b:02x}")).collect()
}

const KINDS: [&str; 6] = [
    "unspecified",
    "internal",
    "server",
    "client",
    "producer",
    "consumer",
];

/// Operation name, resource and type, from the semantic conventions.
fn names(span: &Span, a: &Map<String, Value>, scope: &str) -> (String, String, String) {
    let kind = KINDS.get(span.kind as usize).copied().unwrap_or("internal");
    let get = |k: &str| a.get(k).and_then(text);
    let method = get("http.request.method").or_else(|| get("http.method"));
    let db = get("db.system.name").or_else(|| get("db.system"));
    let messaging = get("messaging.system");
    let rpc = get("rpc.system");
    let operation = if let Some(op) = get("operation.name") {
        op
    } else if method.is_some() && kind == "server" {
        "http.server.request".into()
    } else if method.is_some() && kind == "client" {
        "http.client.request".into()
    } else if let (Some(db), "client") = (&db, kind) {
        format!("{db}.query")
    } else if let Some(m) = &messaging {
        format!(
            "{m}.{}",
            get("messaging.operation.type")
                .or_else(|| get("messaging.operation"))
                .unwrap_or_else(|| kind.into())
        )
    } else if let Some(r) = &rpc {
        format!("{r}.{kind}.request")
    } else if kind == "server" || kind == "client" {
        format!(
            "{}.{kind}",
            if scope.is_empty() {
                "opentelemetry"
            } else {
                scope
            }
        )
    } else {
        span.name.clone()
    };
    let resource = if let Some(r) = get("resource.name") {
        r
    } else if let Some(m) = &method {
        match get("http.route").filter(|_| kind == "server") {
            Some(route) => format!("{m} {route}"),
            None => m.clone(),
        }
    } else if db.is_some() {
        get("db.query.text")
            .or_else(|| get("db.statement"))
            .unwrap_or_else(|| span.name.clone())
    } else {
        span.name.clone()
    };
    let typ = if method.is_some() && kind == "server" {
        "web"
    } else if method.is_some() {
        "http"
    } else if let Some(db) = &db {
        match db.as_str() {
            "redis" | "memcached" => "cache",
            _ => "sql",
        }
    } else if messaging.is_some() {
        "queue"
    } else if kind == "server" {
        "web"
    } else {
        "custom"
    };
    (operation, resource, typ.to_string())
}

pub fn spans_of(req: TracesRequest) -> Vec<Vec<SpanRow>> {
    let mut chunks: HashMap<String, Vec<SpanRow>> = HashMap::new();
    for rs in req.resource_spans {
        let r = res(&rs.resource);
        for ss in rs.scope_spans {
            let scope = ss.scope.unwrap_or_default();
            for span in ss.spans {
                let a = attrs(&span.attributes);
                let (name, resource, typ) = names(&span, &a, &scope.name);
                let mut meta = Map::new();
                let mut metrics = Map::new();
                for (k, v) in r.attrs.iter().chain(a.iter()) {
                    match v {
                        Value::Number(_) => {
                            metrics.insert(k.clone(), v.clone());
                        }
                        Value::Null => {}
                        Value::String(s) => {
                            meta.insert(k.clone(), json!(s));
                        }
                        other => {
                            meta.insert(k.clone(), json!(other.to_string()));
                        }
                    }
                }
                let kind = KINDS.get(span.kind as usize).copied().unwrap_or("internal");
                meta.insert("span.kind".into(), json!(kind));
                meta.insert("otel.trace_id".into(), json!(hex(&span.trace_id)));
                if span.trace_id.len() == 16 {
                    meta.insert("_dd.p.tid".into(), json!(hex(&span.trace_id[..8])));
                }
                if !scope.name.is_empty() {
                    meta.insert("otel.library.name".into(), json!(scope.name));
                }
                if !scope.version.is_empty() {
                    meta.insert("otel.library.version".into(), json!(scope.version));
                }
                if let Some(e) = &r.env {
                    meta.insert("env".into(), json!(e));
                }
                if let Some(v) = &r.version {
                    meta.insert("version".into(), json!(v));
                }
                if let Some(h) = &r.host {
                    meta.insert("_dd.hostname".into(), json!(h));
                }
                let status = span.status.clone().unwrap_or_default();
                let error = status.code == 2;
                meta.insert(
                    "otel.status_code".into(),
                    json!(
                        ["Unset", "Ok", "Error"]
                            .get(status.code as usize)
                            .copied()
                            .unwrap_or("Unset")
                    ),
                );
                if error {
                    if !status.message.is_empty() {
                        meta.insert("error.message".into(), json!(status.message));
                    }
                    if let Some(ex) = span.events.iter().find(|e| e.name == "exception") {
                        let ea = attrs(&ex.attributes);
                        for (from, to) in [
                            ("exception.type", "error.type"),
                            ("exception.message", "error.message"),
                            ("exception.stacktrace", "error.stack"),
                        ] {
                            if let Some(v) = ea.get(from).and_then(text) {
                                meta.insert(to.into(), json!(v));
                            }
                        }
                    }
                }
                let root = span.parent_span_id.is_empty();
                if root || kind == "server" || kind == "consumer" {
                    metrics.insert("_top_level".into(), json!(1));
                }
                let start = span.start_time_unix_nano as i64;
                let trace_id = low64(&span.trace_id);
                chunks
                    .entry(trace_id.clone())
                    .or_default()
                    .push(traces::build(
                        trace_id,
                        low64(&span.span_id),
                        low64(&span.parent_span_id),
                        r.service
                            .clone()
                            .unwrap_or_else(|| "otlpresourcenoservicename".into()),
                        name,
                        resource,
                        Some(typ),
                        start,
                        span.end_time_unix_nano as i64 - start,
                        error,
                        meta,
                        metrics,
                    ));
            }
        }
    }
    chunks.into_values().collect()
}

fn severity(n: i32) -> &'static str {
    match n {
        1..=4 => "trace",
        5..=8 => "debug",
        9..=12 => "info",
        13..=16 => "warn",
        17..=20 => "error",
        21..=24 => "fatal",
        _ => "info",
    }
}

/// OTLP log records as Datadog log objects, ready for the usual processing.
pub fn logs_of(req: LogsRequest) -> Vec<Map<String, Value>> {
    let mut out = Vec::new();
    for rl in req.resource_logs {
        let r = res(&rl.resource);
        for sl in rl.scope_logs {
            for lr in sl.log_records {
                let mut o = r.attrs.clone();
                o.extend(attrs(&lr.attributes));
                let message = match lr.body.as_ref().map(any_to_json) {
                    Some(Value::String(s)) => s,
                    Some(Value::Null) | None => String::new(),
                    Some(other) => other.to_string(),
                };
                o.insert("message".into(), json!(message));
                o.insert(
                    "status".into(),
                    json!(if lr.severity_text.is_empty() {
                        severity(lr.severity_number).to_string()
                    } else {
                        lr.severity_text.clone()
                    }),
                );
                let ns = if lr.time_unix_nano > 0 {
                    lr.time_unix_nano
                } else {
                    lr.observed_time_unix_nano
                };
                if ns > 0 {
                    o.insert("timestamp".into(), json!(ns / 1_000_000));
                }
                if let Some(s) = &r.service {
                    o.insert("service".into(), json!(s));
                }
                if let Some(h) = &r.host {
                    o.insert("hostname".into(), json!(h));
                }
                let mut tags = Vec::new();
                if let Some(e) = &r.env {
                    tags.push(format!("env:{e}"));
                }
                if let Some(v) = &r.version {
                    tags.push(format!("version:{v}"));
                }
                if !tags.is_empty() {
                    o.insert("ddtags".into(), json!(tags.join(",")));
                }
                if !lr.trace_id.is_empty() {
                    o.insert("otel.trace_id".into(), json!(hex(&lr.trace_id)));
                    o.insert("dd.trace_id".into(), json!(low64(&lr.trace_id)));
                }
                if !lr.span_id.is_empty() {
                    o.insert("dd.span_id".into(), json!(low64(&lr.span_id)));
                }
                o.insert("otel.severity_number".into(), json!(lr.severity_number));
                // Exceptions, as Datadog's standard error attributes.
                for (from, to) in [
                    ("exception.type", "error.kind"),
                    ("exception.message", "error.message"),
                    ("exception.stacktrace", "error.stack"),
                ] {
                    if let Some(v) = o.get(from).and_then(text) {
                        o.insert(to.into(), json!(v));
                    }
                }
                out.push(o);
            }
        }
    }
    out
}

/// Cumulative values last seen per series, to send deltas (as the Agent does).
static CUMULATIVE: LazyLock<Mutex<HashMap<String, (u64, f64)>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// When the server started (Unix nanoseconds).
static STARTED: LazyLock<u64> = LazyLock::new(|| {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
});

/// Records the server's start time; called at startup.
pub fn init() {
    LazyLock::force(&STARTED);
}

fn delta(key: String, start: u64, value: f64) -> Option<f64> {
    let mut m = CUMULATIVE.lock().ok()?;
    let prev = m.insert(key, (start, value));
    match prev {
        Some((s, v)) if s == start && value >= v => Some(value - v),
        // A reset (new start time or a lower value) starts over from that value.
        Some(_) => Some(value),
        // A first point counts in full when the counter started after us
        // (Datadog's `initial_cumulative_monotonic_value: auto`); otherwise it
        // only sets the baseline.
        None => (start > 0 && start >= *STARTED).then_some(value),
    }
}

fn metric_tags(r: &Res, kvs: &[KeyValue]) -> Vec<String> {
    let mut tags: Vec<String> = attrs(kvs)
        .iter()
        .filter_map(|(k, v)| text(v).map(|v| format!("{k}:{v}")))
        .collect();
    if let Some(s) = &r.service {
        tags.push(format!("service:{s}"));
    }
    if let Some(e) = &r.env {
        tags.push(format!("env:{e}"));
    }
    if let Some(v) = &r.version {
        tags.push(format!("version:{v}"));
    }
    tags.sort();
    tags.dedup();
    tags
}

pub fn series_of(req: MetricsRequest) -> Vec<Series> {
    let mut out = Vec::new();
    for rm in req.resource_metrics {
        let r = res(&rm.resource);
        for sm in rm.scope_metrics {
            for m in sm.metrics {
                let mut push = |metric: String, kind: Kind, tags: Vec<String>, t: u64, v: f64| {
                    if v.is_finite() {
                        out.push(Series {
                            metric,
                            kind,
                            host: r.host.clone(),
                            tags,
                            points: vec![((t / 1_000_000_000) as i64, v)],
                        });
                    }
                };
                // Cumulative counters become deltas (temporality 2 = cumulative).
                let counter = |name: &str,
                               tags: &[String],
                               cumulative: bool,
                               start: u64,
                               v: f64|
                 -> Option<f64> {
                    if cumulative {
                        delta(
                            format!(
                                "{}|{}|{name}|{}",
                                r.host.as_deref().unwrap_or(""),
                                r.service.as_deref().unwrap_or(""),
                                tags.join(",")
                            ),
                            start,
                            v,
                        )
                    } else {
                        Some(v)
                    }
                };
                let num = |p: &NumberDataPoint| match p.value {
                    Some(number_data_point::Value::AsDouble(d)) => d,
                    Some(number_data_point::Value::AsInt(i)) => i as f64,
                    None => f64::NAN,
                };
                match m.data {
                    Some(metric::Data::Gauge(g)) => {
                        for p in &g.data_points {
                            push(
                                m.name.clone(),
                                Kind::Gauge,
                                metric_tags(&r, &p.attributes),
                                p.time_unix_nano,
                                num(p),
                            );
                        }
                    }
                    Some(metric::Data::Sum(s)) => {
                        for p in &s.data_points {
                            let tags = metric_tags(&r, &p.attributes);
                            if s.is_monotonic {
                                if let Some(v) = counter(
                                    &m.name,
                                    &tags,
                                    s.aggregation_temporality == 2,
                                    p.start_time_unix_nano,
                                    num(p),
                                ) {
                                    push(m.name.clone(), Kind::Count, tags, p.time_unix_nano, v);
                                }
                            } else {
                                push(m.name.clone(), Kind::Gauge, tags, p.time_unix_nano, num(p));
                            }
                        }
                    }
                    Some(metric::Data::Histogram(h)) => {
                        let cumulative = h.aggregation_temporality == 2;
                        for p in &h.data_points {
                            let tags = metric_tags(&r, &p.attributes);
                            let count = format!("{}.count", m.name);
                            if let Some(v) = counter(
                                &count,
                                &tags,
                                cumulative,
                                p.start_time_unix_nano,
                                p.count as f64,
                            ) {
                                push(count, Kind::Count, tags.clone(), p.time_unix_nano, v);
                            }
                            if let Some(sum) = p.sum {
                                let name = format!("{}.sum", m.name);
                                if let Some(v) =
                                    counter(&name, &tags, cumulative, p.start_time_unix_nano, sum)
                                {
                                    push(name, Kind::Count, tags.clone(), p.time_unix_nano, v);
                                }
                            }
                            for (suffix, v) in [("min", p.min), ("max", p.max)] {
                                if let Some(v) = v {
                                    push(
                                        format!("{}.{suffix}", m.name),
                                        Kind::Gauge,
                                        tags.clone(),
                                        p.time_unix_nano,
                                        v,
                                    );
                                }
                            }
                        }
                    }
                    Some(metric::Data::ExponentialHistogram(h)) => {
                        let cumulative = h.aggregation_temporality == 2;
                        for p in &h.data_points {
                            let tags = metric_tags(&r, &p.attributes);
                            let count = format!("{}.count", m.name);
                            if let Some(v) = counter(
                                &count,
                                &tags,
                                cumulative,
                                p.start_time_unix_nano,
                                p.count as f64,
                            ) {
                                push(count, Kind::Count, tags.clone(), p.time_unix_nano, v);
                            }
                            if let Some(sum) = p.sum {
                                let name = format!("{}.sum", m.name);
                                if let Some(v) =
                                    counter(&name, &tags, cumulative, p.start_time_unix_nano, sum)
                                {
                                    push(name, Kind::Count, tags.clone(), p.time_unix_nano, v);
                                }
                            }
                            for (suffix, v) in [("min", p.min), ("max", p.max)] {
                                if let Some(v) = v {
                                    push(
                                        format!("{}.{suffix}", m.name),
                                        Kind::Gauge,
                                        tags.clone(),
                                        p.time_unix_nano,
                                        v,
                                    );
                                }
                            }
                        }
                    }
                    Some(metric::Data::Summary(s)) => {
                        for p in &s.data_points {
                            let tags = metric_tags(&r, &p.attributes);
                            for (suffix, v) in [("count", p.count as f64), ("sum", p.sum)] {
                                let name = format!("{}.{suffix}", m.name);
                                if let Some(v) =
                                    counter(&name, &tags, true, p.start_time_unix_nano, v)
                                {
                                    push(name, Kind::Count, tags.clone(), p.time_unix_nano, v);
                                }
                            }
                            for q in &p.quantile_values {
                                let mut t = tags.clone();
                                t.push(format!("quantile:{}", q.quantile));
                                push(
                                    format!("{}.quantile", m.name),
                                    Kind::Gauge,
                                    t,
                                    p.time_unix_nano,
                                    q.value,
                                );
                            }
                        }
                    }
                    None => {}
                }
            }
        }
    }
    out
}

// --- handlers --------------------------------------------------------------------------

enum Format {
    Proto,
    Json,
}

fn reply(format: Format, status: StatusCode, error: Option<String>) -> Response {
    match (format, error) {
        (Format::Json, None) => {
            (status, [(header::CONTENT_TYPE, "application/json")], "{}").into_response()
        }
        (Format::Json, Some(e)) => {
            (status, axum::Json(json!({"code": 3, "message": e}))).into_response()
        }
        (Format::Proto, None) => (
            status,
            [(header::CONTENT_TYPE, "application/x-protobuf")],
            Vec::new(),
        )
            .into_response(),
        (Format::Proto, Some(e)) => {
            (status, [(header::CONTENT_TYPE, "text/plain")], e).into_response()
        }
    }
}

/// Checks the key, decompresses and tells protobuf from JSON.
#[allow(clippy::result_large_err)]
fn prepare(
    st: &AppState,
    headers: &HeaderMap,
    q: &HashMap<String, String>,
    body: Bytes,
) -> Result<(Format, Vec<u8>), Response> {
    let json = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|c| c.contains("json"));
    let format = || if json { Format::Json } else { Format::Proto };
    if !api_key_ok(st, headers, q) {
        return Err(reply(
            format(),
            StatusCode::FORBIDDEN,
            Some("missing or invalid DD-API-KEY header".into()),
        ));
    }
    let raw = decompress(headers, body).map_err(|_| {
        reply(
            format(),
            StatusCode::BAD_REQUEST,
            Some("bad compression".into()),
        )
    })?;
    Ok((format(), raw))
}

pub async fn traces_handler(
    State(st): State<AppState>,
    Query(q): Query<HashMap<String, String>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let (format, raw) = match prepare(&st, &headers, &q, body) {
        Ok(x) => x,
        Err(r) => return r,
    };
    let req = match format {
        Format::Json => serde_json::from_slice::<Value>(&raw)
            .map(|v| js::traces(&v))
            .map_err(|e| e.to_string()),
        Format::Proto => TracesRequest::decode(raw.as_slice()).map_err(|e| e.to_string()),
    };
    match req {
        Ok(req) => match traces::ingest(&st, spans_of(req)).await {
            Ok(n) => {
                tracing::debug!(count = n, "otlp spans received");
                reply(format, StatusCode::OK, None)
            }
            Err(e) => reply(format, StatusCode::SERVICE_UNAVAILABLE, Some(e.to_string())),
        },
        Err(e) => reply(format, StatusCode::BAD_REQUEST, Some(e)),
    }
}

pub async fn logs_handler(
    State(st): State<AppState>,
    Query(q): Query<HashMap<String, String>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let (format, raw) = match prepare(&st, &headers, &q, body) {
        Ok(x) => x,
        Err(r) => return r,
    };
    let req = match format {
        Format::Json => serde_json::from_slice::<Value>(&raw)
            .map(|v| js::logs(&v))
            .map_err(|e| e.to_string()),
        Format::Proto => LogsRequest::decode(raw.as_slice()).map_err(|e| e.to_string()),
    };
    let req = match req {
        Ok(r) => r,
        Err(e) => return reply(format, StatusCode::BAD_REQUEST, Some(e)),
    };
    let defaults = Defaults {
        source: Some("otlp_log_ingestion".into()),
        service: None,
        host: None,
        tags: Vec::new(),
    };
    let now = now_ms();
    let rows: Vec<_> = logs_of(req)
        .into_iter()
        .map(|o| {
            let mut row = process(o, &defaults, now);
            st.pipelines.apply(&mut row);
            st.generated.on_log(&st.aggregator, &row);
            row
        })
        .collect();
    match st.store.logs.send(rows).await {
        Ok(_) => reply(format, StatusCode::OK, None),
        Err(e) => reply(format, StatusCode::SERVICE_UNAVAILABLE, Some(e.to_string())),
    }
}

pub async fn metrics_handler(
    State(st): State<AppState>,
    Query(q): Query<HashMap<String, String>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let (format, raw) = match prepare(&st, &headers, &q, body) {
        Ok(x) => x,
        Err(r) => return r,
    };
    let req = match format {
        Format::Json => serde_json::from_slice::<Value>(&raw)
            .map(|v| js::metrics(&v))
            .map_err(|e| e.to_string()),
        Format::Proto => MetricsRequest::decode(raw.as_slice()).map_err(|e| e.to_string()),
    };
    match req {
        Ok(req) => match series::store(&st, series_of(req)).await {
            Ok(_) => reply(format, StatusCode::OK, None),
            Err(e) => reply(format, StatusCode::SERVICE_UNAVAILABLE, Some(e.to_string())),
        },
        Err(e) => reply(format, StatusCode::BAD_REQUEST, Some(e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kv(k: &str, v: &str) -> KeyValue {
        KeyValue {
            key: k.into(),
            value: Some(AnyValue {
                value: Some(any_value::Value::StringValue(v.into())),
            }),
        }
    }

    #[test]
    fn spans_map_like_datadog() {
        let trace_id: Vec<u8> = (1..=16).collect();
        let req = TracesRequest {
            resource_spans: vec![ResourceSpans {
                resource: Some(Resource {
                    attributes: vec![
                        kv("service.name", "checkout"),
                        kv("deployment.environment.name", "prod"),
                        kv("host.name", "web-1"),
                    ],
                }),
                scope_spans: vec![ScopeSpans {
                    scope: Some(Scope {
                        name: "io.opentelemetry.http".into(),
                        version: "1.0".into(),
                    }),
                    spans: vec![Span {
                        trace_id: trace_id.clone(),
                        span_id: vec![0, 0, 0, 0, 0, 0, 1, 0],
                        parent_span_id: vec![],
                        name: "GET /users/:id".into(),
                        kind: 2,
                        start_time_unix_nano: 1_000_000_000,
                        end_time_unix_nano: 1_250_000_000,
                        attributes: vec![
                            kv("http.request.method", "GET"),
                            kv("http.route", "/users/{id}"),
                        ],
                        events: vec![SpanEvent {
                            time_unix_nano: 1_100_000_000,
                            name: "exception".into(),
                            attributes: vec![
                                kv("exception.type", "KeyError"),
                                kv("exception.message", "no user"),
                            ],
                        }],
                        status: Some(SpanStatus {
                            message: "boom".into(),
                            code: 2,
                        }),
                    }],
                }],
            }],
        };
        // Through protobuf and back.
        let req = TracesRequest::decode(req.encode_to_vec().as_slice()).unwrap();
        let chunks = spans_of(req);
        let s = &chunks[0][0];
        assert_eq!(s.service, "checkout");
        assert_eq!(s.name, "http.server.request");
        assert_eq!(s.resource, "GET /users/{id}");
        assert_eq!(s.typ.as_deref(), Some("web"));
        assert_eq!(s.env.as_deref(), Some("prod"));
        assert_eq!(
            s.trace_id,
            u64::from_be_bytes([9, 10, 11, 12, 13, 14, 15, 16]).to_string()
        );
        assert_eq!(s.span_id, "256");
        assert_eq!(s.parent_id, "0");
        assert_eq!(s.duration, 250_000_000);
        assert!(s.error && s.top_level);
        assert_eq!(s.meta["error.type"], "KeyError");
        assert_eq!(s.meta["error.message"], "no user");
        assert_eq!(s.meta["_dd.p.tid"], "0102030405060708");
        assert_eq!(s.meta["_dd.hostname"], "web-1");
    }

    #[test]
    fn json_logs_and_metrics() {
        let logs = js::logs(&json!({"resourceLogs": [{
            "resource": {"attributes": [{"key": "service.name", "value": {"stringValue": "api"}}]},
            "scopeLogs": [{"logRecords": [{
                "timeUnixNano": "1700000000000000000", "severityNumber": 17,
                "body": {"stringValue": "payment failed"},
                "attributes": [{"key": "order.id", "value": {"intValue": "42"}}],
                "traceId": "0102030405060708090a0b0c0d0e0f10", "spanId": "0000000000000100"
            }]}]
        }]}));
        let o = &logs_of(logs)[0];
        assert_eq!(o["message"], "payment failed");
        assert_eq!(o["status"], "error");
        assert_eq!(o["service"], "api");
        assert_eq!(o["timestamp"], 1_700_000_000_000u64);
        assert_eq!(o["order.id"], 42);
        assert_eq!(o["dd.span_id"], "256");

        let m = |v: f64| {
            js::metrics(&json!({"resourceMetrics": [{
                "resource": {"attributes": [{"key": "service.name", "value": {"stringValue": "api"}}]},
                "scopeMetrics": [{"metrics": [
                    {"name": "http.requests", "sum": {"isMonotonic": true, "aggregationTemporality": 2, "dataPoints": [
                        {"startTimeUnixNano": "1", "timeUnixNano": "60000000000", "asDouble": v, "attributes": [{"key": "code", "value": {"stringValue": "200"}}]}]}},
                    {"name": "queue.depth", "gauge": {"dataPoints": [{"timeUnixNano": "60000000000", "asInt": "7"}]}}
                ]}]
            }]}))
        };
        init();
        let first = series_of(m(10.0));
        assert_eq!(
            first.len(),
            1,
            "a counter older than the server: its first point is the baseline"
        );
        assert_eq!(first[0].metric, "queue.depth");
        assert_eq!(first[0].points, vec![(60, 7.0)]);
        let second = series_of(m(25.0));
        let c = second.iter().find(|s| s.metric == "http.requests").unwrap();
        assert_eq!(c.points, vec![(60, 15.0)]);
        assert_eq!(c.tags, vec!["code:200", "service:api"]);
    }
}
