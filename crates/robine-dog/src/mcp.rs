//! A Model Context Protocol server, as Datadog's: `POST /v1/mcp`
//! (streamable HTTP, answered in JSON), the key in `DD_API_KEY` or in
//! `Authorization: Bearer`, and the tools of Datadog's `core` toolset under
//! their names (`search_datadog_logs`, `get_datadog_metric`…).
//!
//! ```json
//! { "mcpServers": { "robine-dog": { "type": "http", "url": "https://<robine-dog>/v1/mcp",
//!     "headers": { "DD_API_KEY": "<API key>" } } } }
//! ```
//!
//! Each tool is a call to the API of the UI, so a model reads exactly what
//! the pages show. Times are given as Datadog's tools take them: `now-1h`,
//! `now`, an ISO date or epoch seconds.

use crate::AppState;
use axum::Json;
use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, Method, Request, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};
use tower::ServiceExt;

const PROTOCOLS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];

struct Tool {
    name: &'static str,
    description: &'static str,
    /// Its parameters beyond none: `(name, type, description)`, required first.
    params: &'static [(&'static str, &'static str, &'static str)],
    required: usize,
    writes: bool,
}

const QUERY: (&str, &str, &str) = (
    "query",
    "string",
    "Search query, in Datadog's syntax: service:shop status:error @http.status_code:>=500 \"free text\"",
);
const FROM: (&str, &str, &str) = (
    "from",
    "string",
    "Start of the time range: now-1h (default), now-15m, now-7d, an ISO 8601 date or epoch seconds",
);
const TO: (&str, &str, &str) = ("to", "string", "End of the time range: now (default)");
const LIMIT: (&str, &str, &str) = ("limit", "integer", "Maximum number of results (default 50)");
const MAX_TOKENS: (&str, &str, &str) = (
    "max_tokens",
    "integer",
    "Roughly how much to return (default 10000)",
);
const GROUP_BY: (&str, &str, &str) = (
    "group_by",
    "array",
    "Facets to group by: service, status, host, @http.status_code…",
);
const AGGREGATION: (&str, &str, &str) = (
    "aggregation",
    "string",
    "count (default), avg, sum, min, max, pc75, pc95, pc99 or cardinality",
);
const MEASURE: (&str, &str, &str) = (
    "metric",
    "string",
    "The measure to aggregate when not counting: @duration, @http.response_time…",
);

const TOOLS: &[Tool] = &[
    Tool {
        name: "search_datadog_logs",
        description: "Searches logs by time and query (service, host, status, attributes, free text).",
        params: &[QUERY, FROM, TO, LIMIT, MAX_TOKENS],
        required: 0,
        writes: false,
    },
    Tool {
        name: "analyze_datadog_logs",
        description: "Counts or aggregates logs, grouped by facets: error logs by service, p95 of a measure by host…",
        params: &[QUERY, GROUP_BY, AGGREGATION, MEASURE, FROM, TO, MAX_TOKENS],
        required: 0,
        writes: false,
    },
    Tool {
        name: "search_datadog_spans",
        description: "Retrieves APM spans filtered by service, resource, status, duration and more.",
        params: &[QUERY, FROM, TO, LIMIT, MAX_TOKENS],
        required: 0,
        writes: false,
    },
    Tool {
        name: "aggregate_spans",
        description: "Aggregates APM spans (counts, averages, percentiles), grouped by facets: p95 @duration by resource_name…",
        params: &[QUERY, GROUP_BY, AGGREGATION, MEASURE, FROM, TO, MAX_TOKENS],
        required: 0,
        writes: false,
    },
    Tool {
        name: "get_datadog_trace",
        description: "Fetches a complete APM trace by trace ID.",
        params: &[("trace_id", "string", "The trace ID"), MAX_TOKENS],
        required: 1,
        writes: false,
    },
    Tool {
        name: "search_datadog_entities",
        description: "Lists the services seen by APM, with their requests, errors and latency over the time range.",
        params: &[
            ("query", "string", "Part of a service name"),
            FROM,
            TO,
            MAX_TOKENS,
        ],
        required: 0,
        writes: false,
    },
    Tool {
        name: "search_datadog_metrics",
        description: "Lists available metrics, with their type and unit.",
        params: &[
            (
                "query",
                "string",
                "Part of a metric name: redis, system.cpu",
            ),
            LIMIT,
            MAX_TOKENS,
        ],
        required: 0,
        writes: false,
    },
    Tool {
        name: "get_datadog_metric_context",
        description: "Returns a metric's metadata, its tags and their values.",
        params: &[
            ("metric_name", "string", "The metric: system.cpu.user"),
            FROM,
            TO,
            MAX_TOKENS,
        ],
        required: 1,
        writes: false,
    },
    Tool {
        name: "get_datadog_metric",
        description: "Queries metric data over time with Datadog's query language.",
        params: &[
            (
                "queries",
                "array",
                "Metric queries: avg:system.cpu.user{env:prod} by {host}, sum:trace.http.request.hits{*}.as_count()",
            ),
            FROM,
            TO,
            MAX_TOKENS,
        ],
        required: 1,
        writes: false,
    },
    Tool {
        name: "search_datadog_monitors",
        description: "Retrieves monitors with their status, query and thresholds.",
        params: &[
            ("query", "string", "Part of a monitor name"),
            (
                "status",
                "string",
                "Only monitors in this state: Alert, Warn, No Data or OK",
            ),
            MAX_TOKENS,
        ],
        required: 0,
        writes: false,
    },
    Tool {
        name: "search_datadog_events",
        description: "Searches events: monitor alerts, Watchdog stories, incidents, Agent and API events.",
        params: &[QUERY, FROM, TO, LIMIT, MAX_TOKENS],
        required: 0,
        writes: false,
    },
    Tool {
        name: "search_datadog_rum_events",
        description: "Searches RUM events (views, actions, resources, errors, long tasks); the type is `source`.",
        params: &[QUERY, FROM, TO, LIMIT, MAX_TOKENS],
        required: 0,
        writes: false,
    },
    Tool {
        name: "search_datadog_hosts",
        description: "Lists the hosts Agents report from.",
        params: &[
            ("query", "string", "Part of a host name or of a tag"),
            MAX_TOKENS,
        ],
        required: 0,
        writes: false,
    },
    Tool {
        name: "search_datadog_dashboards",
        description: "Lists dashboards.",
        params: &[("query", "string", "Part of a dashboard title"), MAX_TOKENS],
        required: 0,
        writes: false,
    },
    Tool {
        name: "get_datadog_dashboard",
        description: "Retrieves a dashboard by ID, with its widgets and their queries.",
        params: &[("dashboard_id", "string", "The dashboard ID"), MAX_TOKENS],
        required: 1,
        writes: false,
    },
    Tool {
        name: "search_datadog_incidents",
        description: "Lists incidents with their state and severity.",
        params: &[
            (
                "state",
                "string",
                "Only incidents in this state: active, stable or resolved",
            ),
            MAX_TOKENS,
        ],
        required: 0,
        writes: false,
    },
    Tool {
        name: "get_datadog_incident",
        description: "Retrieves an incident with its timeline.",
        params: &[
            (
                "incident_id",
                "string",
                "The incident ID, or its number (IR-12 or 12)",
            ),
            MAX_TOKENS,
        ],
        required: 1,
        writes: false,
    },
    Tool {
        name: "search_datadog_notebooks",
        description: "Lists notebooks.",
        params: &[("query", "string", "Part of a notebook name"), MAX_TOKENS],
        required: 0,
        writes: false,
    },
    Tool {
        name: "get_datadog_notebook",
        description: "Retrieves a notebook by ID, with its cells.",
        params: &[("notebook_id", "string", "The notebook ID"), MAX_TOKENS],
        required: 1,
        writes: false,
    },
    Tool {
        name: "create_datadog_notebook",
        description: "Creates a notebook from Markdown, for an investigation or a postmortem.",
        params: &[
            ("name", "string", "The notebook's name"),
            ("markdown", "string", "Its text, in Markdown"),
        ],
        required: 2,
        writes: true,
    },
    Tool {
        name: "append_datadog_notebook",
        description: "Adds to the end of a notebook: Markdown, a metric graph or a log graph.",
        params: &[
            ("notebook_id", "string", "The notebook ID"),
            ("markdown", "string", "Text to add, in Markdown"),
            (
                "metric_query",
                "string",
                "A metric query to graph: avg:system.cpu.user{*} by {host}",
            ),
            (
                "logs_query",
                "string",
                "A log search to graph: service:shop status:error",
            ),
        ],
        required: 1,
        writes: true,
    },
];

fn schema(t: &Tool) -> Value {
    let props: Map<String, Value> = t
        .params
        .iter()
        .map(|(name, typ, description)| {
            let mut p = json!({"type": typ, "description": description});
            if *typ == "array" {
                p["items"] = json!({"type": "string"});
            }
            (name.to_string(), p)
        })
        .collect();
    let required: Vec<&str> = t.params[..t.required].iter().map(|p| p.0).collect();
    json!({
        "name": t.name,
        "description": t.description,
        "inputSchema": {"type": "object", "properties": props, "required": required},
        "annotations": {"readOnlyHint": !t.writes},
    })
}

/// `now`, `now-15m`, an ISO 8601 date, epoch seconds or milliseconds.
pub fn parse_time(s: &str, now: i64) -> Result<i64, String> {
    let s = s.trim();
    if s.is_empty() || s == "now" {
        return Ok(now);
    }
    if let Some(rest) = s.strip_prefix("now-") {
        let split = rest
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(rest.len());
        let n: i64 = rest[..split]
            .parse()
            .map_err(|_| format!("bad time: {s}"))?;
        let unit = match &rest[split..] {
            "s" => 1_000,
            "m" | "min" => 60_000,
            "h" => 3_600_000,
            "d" => 86_400_000,
            "w" => 604_800_000,
            "mo" => 30 * 86_400_000,
            _ => return Err(format!("bad time: {s} (now-15m, now-4h, now-7d…)")),
        };
        return Ok(now - n * unit);
    }
    if let Ok(n) = s.parse::<i64>() {
        return Ok(if n < 100_000_000_000 { n * 1000 } else { n });
    }
    chrono::DateTime::parse_from_rfc3339(s)
        .map(|d| d.timestamp_millis())
        .map_err(|_| format!("bad time: {s} (now-1h, an ISO 8601 date or epoch seconds)"))
}

struct Args<'a>(&'a Map<String, Value>);

impl Args<'_> {
    fn str(&self, k: &str) -> Option<String> {
        match self.0.get(k)? {
            Value::String(s) if !s.trim().is_empty() => Some(s.trim().to_string()),
            Value::Number(n) => Some(n.to_string()),
            _ => None,
        }
    }

    fn need(&self, k: &str) -> Result<String, String> {
        self.str(k).ok_or_else(|| format!("{k} is required"))
    }

    fn list(&self, k: &str) -> Vec<String> {
        match self.0.get(k) {
            Some(Value::Array(a)) => a
                .iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect(),
            Some(Value::String(s)) => s
                .split(',')
                .map(|x| x.trim().to_string())
                .filter(|x| !x.is_empty())
                .collect(),
            _ => vec![],
        }
    }

    fn range(&self) -> Result<(i64, i64), String> {
        let now = crate::intake::logs::now_ms();
        let from = parse_time(&self.str("from").unwrap_or_else(|| "now-1h".into()), now)?;
        let to = parse_time(&self.str("to").unwrap_or_default(), now)?;
        if from >= to {
            return Err("from must be before to".into());
        }
        Ok((from, to))
    }

    fn limit(&self) -> i64 {
        self.0
            .get("limit")
            .and_then(Value::as_i64)
            .unwrap_or(50)
            .clamp(1, 500)
    }
}

/// A call to the API of the UI, without going through the network.
async fn ui(
    st: &AppState,
    method: Method,
    path: &str,
    body: Option<Value>,
) -> Result<Value, String> {
    let req = Request::builder()
        .method(method)
        .uri(path)
        .header(header::CONTENT_TYPE, "application/json")
        .body(match &body {
            Some(b) => Body::from(b.to_string()),
            None => Body::empty(),
        })
        .map_err(|e| e.to_string())?;
    let res = crate::ui_routes()
        .with_state(st.clone())
        .oneshot(req)
        .await
        .map_err(|e| e.to_string())?;
    let status = res.status();
    let bytes = axum::body::to_bytes(res.into_body(), 64 * 1024 * 1024)
        .await
        .map_err(|e| e.to_string())?;
    if !status.is_success() {
        let text = String::from_utf8_lossy(&bytes);
        let message = serde_json::from_slice::<Value>(&bytes)
            .ok()
            .and_then(|v| v.get("error").and_then(Value::as_str).map(String::from))
            .unwrap_or_else(|| text.chars().take(500).collect());
        return Err(if status == StatusCode::NOT_FOUND && message.is_empty() {
            "not found".into()
        } else {
            message
        });
    }
    Ok(serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

fn enc(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// Keeps the items of an array whose text contains `needle`.
fn matching(v: Value, needle: Option<String>) -> Value {
    let (Value::Array(items), Some(n)) = (&v, needle.map(|n| n.to_lowercase())) else {
        return v;
    };
    Value::Array(
        items
            .iter()
            .filter(|i| i.to_string().to_lowercase().contains(&n))
            .cloned()
            .collect(),
    )
}

async fn search(st: &AppState, a: &Args<'_>, path: &str) -> Result<Value, String> {
    let (from, to) = a.range()?;
    let body = json!({"query": a.str("query").unwrap_or_default(), "from": from, "to": to, "limit": a.limit()});
    ui(st, Method::POST, path, Some(body)).await
}

async fn aggregate(st: &AppState, a: &Args<'_>, dataset: &str) -> Result<Value, String> {
    let (from, to) = a.range()?;
    let mut compute = json!({"aggregation": a.str("aggregation").unwrap_or_else(|| "count".into()), "type": "total"});
    if let Some(m) = a.str("metric") {
        compute["metric"] = json!(m);
    }
    let group_by: Vec<Value> = a
        .list("group_by")
        .into_iter()
        .map(|f| json!({"facet": f, "limit": 50}))
        .collect();
    let body = json!({
        "compute": [compute],
        "filter": {"query": a.str("query").unwrap_or_default(), "from": from, "to": to},
        "group_by": group_by,
    });
    ui(
        st,
        Method::POST,
        &format!("/{dataset}/aggregate"),
        Some(body),
    )
    .await
}

async fn call(st: &AppState, name: &str, args: &Map<String, Value>) -> Result<Value, String> {
    let a = Args(args);
    let get = |path: String| async move { ui(st, Method::GET, &path, None).await };
    match name {
        "search_datadog_logs" => search(st, &a, "/logs/search").await,
        "search_datadog_spans" => search(st, &a, "/spans/search").await,
        "search_datadog_events" => search(st, &a, "/events/search").await,
        "search_datadog_rum_events" => search(st, &a, "/rum/search").await,
        "analyze_datadog_logs" => aggregate(st, &a, "logs").await,
        "aggregate_spans" => aggregate(st, &a, "spans").await,
        "get_datadog_trace" => get(format!("/traces/{}", enc(&a.need("trace_id")?))).await,
        "search_datadog_entities" => {
            let (from, to) = a.range()?;
            let v = get(format!("/apm/services?from={from}&to={to}")).await?;
            Ok(matching(v, a.str("query")))
        }
        "search_datadog_metrics" => {
            let Value::Array(all) = matching(get("/metrics".into()).await?, a.str("query")) else {
                return Ok(Value::Null);
            };
            let total = all.len();
            let shown: Vec<Value> = all.into_iter().take(a.limit() as usize).collect();
            Ok(json!({"total": total, "metrics": shown}))
        }
        "get_datadog_metric_context" => {
            let (from, to) = a.range()?;
            get(format!(
                "/metrics/{}?from={from}&to={to}",
                enc(&a.need("metric_name")?)
            ))
            .await
        }
        "get_datadog_metric" => {
            let (from, to) = a.range()?;
            let queries = a.list("queries");
            if queries.is_empty() {
                return Err("queries is required".into());
            }
            let body = json!({"queries": queries, "from": from, "to": to, "max_series": 20});
            ui(st, Method::POST, "/metrics/query", Some(body)).await
        }
        "search_datadog_monitors" => {
            let v = matching(get("/monitors".into()).await?, a.str("query"));
            let (Value::Array(items), Some(status)) = (&v, a.str("status")) else {
                return Ok(v);
            };
            Ok(Value::Array(
                items
                    .iter()
                    .filter(|m| {
                        m["overall_state"]
                            .as_str()
                            .is_some_and(|s| s.eq_ignore_ascii_case(&status))
                    })
                    .cloned()
                    .collect(),
            ))
        }
        "search_datadog_hosts" => Ok(matching(get("/hosts".into()).await?, a.str("query"))),
        "search_datadog_dashboards" => {
            let mut v = get("/dashboards".into()).await?;
            Ok(matching(v["dashboards"].take(), a.str("query")))
        }
        "get_datadog_dashboard" => {
            get(format!("/dashboards/{}", enc(&a.need("dashboard_id")?))).await
        }
        "search_datadog_incidents" => {
            let mut v = get("/incidents".into()).await?;
            let Value::Array(items) = v["incidents"].take() else {
                return Ok(Value::Null);
            };
            let state = a.str("state");
            // The timelines are for get_datadog_incident.
            Ok(Value::Array(
                items
                    .into_iter()
                    .filter(|i| state.as_deref().is_none_or(|s| i["state"] == s))
                    .map(|mut i| {
                        if let Some(o) = i.as_object_mut() {
                            o.remove("timeline");
                        }
                        i
                    })
                    .collect(),
            ))
        }
        "get_datadog_incident" => {
            let id = a.need("incident_id")?;
            let number = id.trim_start_matches("IR-").parse::<i64>().ok();
            let mut v = get("/incidents".into()).await?;
            let Value::Array(items) = v["incidents"].take() else {
                return Err("not found".into());
            };
            items
                .into_iter()
                .find(|i| {
                    i["id"] == id.as_str()
                        || (number.is_some() && i["public_id"].as_i64() == number)
                })
                .ok_or_else(|| "not found".into())
        }
        "search_datadog_notebooks" => {
            let mut v = get("/notebooks".into()).await?;
            Ok(matching(v["data"].take(), a.str("query")))
        }
        "get_datadog_notebook" => get(format!("/notebooks/{}", enc(&a.need("notebook_id")?))).await,
        "create_datadog_notebook" => {
            let body = json!({
                "name": a.need("name")?,
                "cells": [{"id": crate::store::pg::new_id(), "type": "markdown", "text": a.need("markdown")?}],
            });
            let v = ui(st, Method::POST, "/notebooks", Some(body)).await?;
            Ok(
                json!({"id": v["id"], "url": format!("{}/notebook/{}", st.cfg.base_url.trim_end_matches('/'), v["id"].as_str().unwrap_or(""))}),
            )
        }
        "append_datadog_notebook" => {
            let id = enc(&a.need("notebook_id")?);
            let mut nb = get(format!("/notebooks/{id}")).await?;
            let mut added = 0;
            for (key, typ) in [
                ("markdown", "markdown"),
                ("metric_query", "metric"),
                ("logs_query", "logs"),
            ] {
                if let (Some(text), Some(cells)) = (a.str(key), nb["cells"].as_array_mut()) {
                    cells
                        .push(json!({"id": crate::store::pg::new_id(), "type": typ, "text": text}));
                    added += 1;
                }
            }
            if added == 0 {
                return Err("give markdown, metric_query or logs_query".into());
            }
            ui(st, Method::PUT, &format!("/notebooks/{id}"), Some(nb)).await?;
            Ok(json!({"added_cells": added}))
        }
        _ => Err(format!("unknown tool: {name}")),
    }
}

fn text_result(text: String, is_error: bool) -> Value {
    json!({"content": [{"type": "text", "text": text}], "isError": is_error})
}

async fn tools_call(st: &AppState, params: &Value) -> Value {
    let name = params["name"].as_str().unwrap_or("");
    let empty = Map::new();
    let args = params["arguments"].as_object().unwrap_or(&empty);
    let budget = args
        .get("max_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(10_000)
        .clamp(500, 100_000) as usize
        * 4;
    match call(st, name, args).await {
        Ok(v) => {
            let mut text = serde_json::to_string(&v).unwrap_or_default();
            if text.len() > budget {
                let mut cut = budget;
                while !text.is_char_boundary(cut) {
                    cut -= 1;
                }
                text.truncate(cut);
                text.push_str("\n… [truncated: narrow the query or the time range, lower limit, or raise max_tokens]");
            }
            text_result(text, false)
        }
        Err(e) => text_result(e, true),
    }
}

/// Whether the caller holds the API key. Without a key configured, only a
/// server without authentication (local development) answers.
fn authorized(st: &AppState, headers: &HeaderMap) -> bool {
    let Some(expected) = &st.cfg.api_key else {
        return st.cfg.auth_disabled;
    };
    let header = |name: &str| headers.get(name).and_then(|v| v.to_str().ok());
    let bearer = header("authorization").and_then(|v| {
        v.strip_prefix("Bearer ")
            .or_else(|| v.strip_prefix("bearer "))
    });
    [header("dd_api_key"), header("dd-api-key"), bearer]
        .into_iter()
        .flatten()
        .any(|k| k.trim() == expected)
}

fn rpc_error(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

async fn handle(st: &AppState, msg: &Value) -> Option<Value> {
    // A notification, or a response to us: nothing to answer.
    let id = msg.get("id").filter(|i| !i.is_null())?.clone();
    let method = msg.get("method")?.as_str().unwrap_or("");
    let params = msg.get("params").cloned().unwrap_or(Value::Null);
    let result = match method {
        "initialize" => {
            let asked = params["protocolVersion"].as_str().unwrap_or("");
            json!({
                "protocolVersion": if PROTOCOLS.contains(&asked) { asked } else { PROTOCOLS[0] },
                "capabilities": {"tools": {"listChanged": false}},
                "serverInfo": {"name": "robine-dog", "title": "robine-dog", "version": env!("CARGO_PKG_VERSION")},
                "instructions": "Logs, traces, metrics, monitors, events, RUM, incidents, dashboards and notebooks of this robine-dog, \
                    under the names of Datadog's MCP tools. Queries use Datadog's search and metric query syntax. \
                    Times: now-1h, now-7d, ISO 8601 or epoch seconds; the default range is the past hour.",
            })
        }
        "ping" => json!({}),
        "tools/list" => json!({"tools": TOOLS.iter().map(schema).collect::<Vec<_>>()}),
        "tools/call" => {
            if !TOOLS
                .iter()
                .any(|t| Some(t.name) == params["name"].as_str())
            {
                return Some(rpc_error(id, -32602, "unknown tool"));
            }
            tools_call(st, &params).await
        }
        "resources/list" => json!({"resources": []}),
        "prompts/list" => json!({"prompts": []}),
        _ => return Some(rpc_error(id, -32601, "method not found")),
    };
    Some(json!({"jsonrpc": "2.0", "id": id, "result": result}))
}

/// `POST /v1/mcp`.
pub async fn post(
    State(st): State<AppState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    if !authorized(&st, &headers) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(rpc_error(
                Value::Null,
                -32001,
                "a valid DD_API_KEY header is required",
            )),
        )
            .into_response();
    }
    let Ok(msg) = serde_json::from_slice::<Value>(&body) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(rpc_error(Value::Null, -32700, "parse error")),
        )
            .into_response();
    };
    let answer = match &msg {
        Value::Array(batch) => {
            let mut out = Vec::new();
            for m in batch {
                out.extend(handle(&st, m).await);
            }
            (!out.is_empty()).then_some(Value::Array(out))
        }
        m => handle(&st, m).await,
    };
    match answer {
        Some(v) => Json(v).into_response(),
        None => StatusCode::ACCEPTED.into_response(),
    }
}

/// No stream to open: everything is answered in the POST.
pub async fn get() -> Response {
    (StatusCode::METHOD_NOT_ALLOWED, [(header::ALLOW, "POST")]).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_times_and_describes_tools() {
        let now = 1_800_000_000_000;
        assert_eq!(parse_time("now", now), Ok(now));
        assert_eq!(parse_time("now-15m", now), Ok(now - 900_000));
        assert_eq!(parse_time("now-7d", now), Ok(now - 7 * 86_400_000));
        assert_eq!(parse_time("1794910470", now), Ok(1_794_910_470_000));
        assert_eq!(
            parse_time("2026-11-17T10:00:00Z", now),
            Ok(1_794_909_600_000)
        );
        assert!(parse_time("yesterday", now).is_err());

        let logs = schema(&TOOLS[0]);
        assert_eq!(logs["name"], "search_datadog_logs");
        assert_eq!(logs["inputSchema"]["properties"]["query"]["type"], "string");
        assert_eq!(logs["annotations"]["readOnlyHint"], true);
        let metric = schema(
            TOOLS
                .iter()
                .find(|t| t.name == "get_datadog_metric")
                .unwrap(),
        );
        assert_eq!(metric["inputSchema"]["required"], json!(["queries"]));
        assert_eq!(
            metric["inputSchema"]["properties"]["queries"]["items"]["type"],
            "string"
        );
        assert_eq!(enc("a b/c"), "a%20b%2Fc");
    }
}
