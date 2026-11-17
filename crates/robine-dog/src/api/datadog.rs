//! Datadog's public API for reading: what its clients, SDKs and scripts
//! call with `DD-API-KEY`. Each route translates Datadog's request to the
//! API of the UI and the answer back to Datadog's shape.
//!
//! - Logs: `POST /api/v2/logs/events/search`, `GET /api/v2/logs/events`
//! - Spans: `POST /api/v2/spans/events/search`, `GET /api/v2/spans/events`
//! - RUM: `POST /api/v2/rum/events/search`, `GET /api/v2/rum/events`
//! - Events: `POST /api/v2/events/search`, `GET /api/v2/events`, `GET /api/v1/events`
//! - Metrics: `GET /api/v1/query`, `GET /api/v1/metrics`, `GET|PUT /api/v1/metrics/{name}`,
//!   `GET /api/v2/metrics`, `GET /api/v1/search`
//! - Hosts: `GET /api/v1/hosts`, `GET /api/v1/hosts/totals`
//! - Incidents: `/api/v2/incidents`, `/api/v2/incidents/{id}`
//! - Notebooks: `/api/v1/notebooks`, `/api/v1/notebooks/{id}`

use crate::AppState;
use crate::intake::logs::now_ms;
use crate::mcp::{parse_time, ui};
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{Method, StatusCode};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};
use std::collections::HashMap;

type Params = Query<HashMap<String, String>>;

fn bad(message: impl Into<String>) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({"errors": [message.into()]})),
    )
        .into_response()
}

/// An error of the UI's API, as Datadog words its own.
fn failed(message: String) -> Response {
    let status = if message == "not found" {
        StatusCode::NOT_FOUND
    } else {
        StatusCode::BAD_REQUEST
    };
    (status, Json(json!({"errors": [message]}))).into_response()
}

fn iso(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|d| d.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
        .unwrap_or_default()
}

fn text(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// What a search asks: Datadog's `filter`, `page` and `sort`.
struct Search {
    query: String,
    from: i64,
    to: i64,
    limit: i64,
    cursor: Option<String>,
    sort: Option<String>,
}

impl Search {
    fn new(
        query: Option<String>,
        from: Option<String>,
        to: Option<String>,
        limit: Option<i64>,
        cursor: Option<String>,
        sort: Option<String>,
    ) -> Result<Search, String> {
        let now = now_ms();
        Ok(Search {
            query: query.filter(|q| q != "*").unwrap_or_default(),
            from: parse_time(from.as_deref().unwrap_or("now-15m"), now)?,
            to: parse_time(to.as_deref().unwrap_or("now"), now)?,
            limit: limit.unwrap_or(10).clamp(1, 1000),
            cursor,
            sort,
        })
    }

    /// From a body: `{filter: {query, from, to}, page: {limit, cursor}, sort}`,
    /// possibly wrapped in `data.attributes` (spans).
    fn from_body(body: &Value) -> Result<Search, String> {
        let b = body.pointer("/data/attributes").unwrap_or(body);
        Search::new(
            b.pointer("/filter/query").and_then(text),
            b.pointer("/filter/from").and_then(text),
            b.pointer("/filter/to").and_then(text),
            b.pointer("/page/limit").and_then(Value::as_i64),
            b.pointer("/page/cursor").and_then(text),
            b.get("sort").and_then(text),
        )
    }

    /// From a query string: `filter[query]`, `page[limit]`…
    fn from_query(q: &HashMap<String, String>) -> Result<Search, String> {
        Search::new(
            q.get("filter[query]").cloned(),
            q.get("filter[from]").cloned(),
            q.get("filter[to]").cloned(),
            q.get("page[limit]").and_then(|l| l.parse().ok()),
            q.get("page[cursor]").cloned(),
            q.get("sort").cloned(),
        )
    }

    fn body(&self) -> Value {
        let mut b =
            json!({"query": self.query, "from": self.from, "to": self.to, "limit": self.limit});
        if let Some(c) = &self.cursor {
            b["cursor"] = json!(c);
        }
        b
    }
}

fn page(data: Vec<Value>, cursor: Option<String>) -> Response {
    let mut meta = json!({"status": "done", "elapsed": 0});
    if let Some(c) = cursor {
        meta["page"] = json!({"after": c});
    }
    Json(json!({"data": data, "meta": meta})).into_response()
}

/// Logs, events and RUM events are stored alike and searched alike.
async fn log_like(st: &AppState, s: Result<Search, String>, path: &str, typ: &str) -> Response {
    let s = match s {
        Ok(s) => s,
        Err(e) => return bad(e),
    };
    let mut v = match ui(st, Method::POST, path, Some(s.body())).await {
        Ok(v) => v,
        Err(e) => return failed(e),
    };
    let cursor = v["next_cursor"].as_str().map(String::from);
    let Value::Array(rows) = v["logs"].take() else {
        return page(vec![], None);
    };
    let data = rows
        .into_iter()
        .map(|mut l| {
            let mut a = Map::new();
            a.insert(
                "timestamp".into(),
                json!(iso(l["timestamp"].as_i64().unwrap_or(0))),
            );
            for k in ["status", "service", "host", "message", "tags", "attributes"] {
                if !l[k].is_null() {
                    a.insert(k.into(), l[k].take());
                }
            }
            // Datadog's reserved attributes that are columns here.
            if let Some(extra) = a.get_mut("attributes").and_then(Value::as_object_mut) {
                for (k, name) in [
                    ("source", "ddsource"),
                    ("trace_id", "trace_id"),
                    ("span_id", "span_id"),
                ] {
                    if !l[k].is_null() {
                        extra.entry(name).or_insert(l[k].take());
                    }
                }
            }
            json!({"id": l["id"].to_string(), "type": typ, "attributes": a})
        })
        .collect();
    page(data, cursor)
}

pub async fn logs_search(State(st): State<AppState>, Json(b): Json<Value>) -> Response {
    log_like(&st, Search::from_body(&b), "/logs/search", "log").await
}

pub async fn logs_list(State(st): State<AppState>, Query(q): Params) -> Response {
    log_like(&st, Search::from_query(&q), "/logs/search", "log").await
}

pub async fn rum_search(State(st): State<AppState>, Json(b): Json<Value>) -> Response {
    log_like(&st, Search::from_body(&b), "/rum/search", "rum").await
}

pub async fn rum_list(State(st): State<AppState>, Query(q): Params) -> Response {
    log_like(&st, Search::from_query(&q), "/rum/search", "rum").await
}

pub async fn events_search(State(st): State<AppState>, Json(b): Json<Value>) -> Response {
    log_like(&st, Search::from_body(&b), "/events/search", "event").await
}

pub async fn events_list(State(st): State<AppState>, Query(q): Params) -> Response {
    log_like(&st, Search::from_query(&q), "/events/search", "event").await
}

async fn spans(st: &AppState, s: Result<Search, String>) -> Response {
    let s = match s {
        Ok(s) => s,
        Err(e) => return bad(e),
    };
    let mut body = s.body();
    if let Some(sort) = &s.sort {
        body["sort"] = json!(sort);
    }
    let mut v = match ui(st, Method::POST, "/spans/search", Some(body)).await {
        Ok(v) => v,
        Err(e) => return failed(e),
    };
    let cursor = v["next_cursor"].as_str().map(String::from);
    let Value::Array(rows) = v["spans"].take() else {
        return page(vec![], None);
    };
    let data = rows
        .into_iter()
        .map(|mut sp| {
            let start = sp["start"].as_i64().unwrap_or(0);
            let duration = sp["duration"].as_i64().unwrap_or(0);
            let mut custom = sp["meta"].take();
            if let (Some(c), Some(m)) = (custom.as_object_mut(), sp["metrics"].as_object()) {
                c.extend(m.clone());
                c.insert("duration".into(), json!(duration));
            }
            json!({
                "id": sp["id"].to_string(),
                "type": "spans",
                "attributes": {
                    "service": sp["service"],
                    "operation_name": sp["name"],
                    "resource_name": sp["resource"],
                    "type": sp["type"],
                    "env": sp["env"],
                    "trace_id": sp["trace_id"],
                    "span_id": sp["span_id"],
                    "parent_id": sp["parent_id"],
                    "start_timestamp": iso(start / 1_000_000),
                    "end_timestamp": iso((start + duration) / 1_000_000),
                    "status": if sp["error"] == true { "error" } else { "ok" },
                    "single_span": false,
                    "custom": custom,
                    "tags": [],
                },
            })
        })
        .collect();
    page(data, cursor)
}

pub async fn spans_search(State(st): State<AppState>, Json(b): Json<Value>) -> Response {
    spans(&st, Search::from_body(&b)).await
}

pub async fn spans_list(State(st): State<AppState>, Query(q): Params) -> Response {
    spans(&st, Search::from_query(&q)).await
}

/// `GET /api/v1/events?start=&end=` (epoch seconds), `tags`, `sources`.
pub async fn events_v1(State(st): State<AppState>, Query(q): Params) -> Response {
    let secs = |k: &str| q.get(k).and_then(|v| v.parse::<i64>().ok());
    let (Some(start), Some(end)) = (secs("start"), secs("end")) else {
        return bad("start and end are required (epoch seconds)");
    };
    let mut query: Vec<String> = Vec::new();
    for t in q.get("tags").map(|t| t.split(',')).into_iter().flatten() {
        query.push(t.trim().to_string());
    }
    if let Some(s) = q.get("sources") {
        let any: Vec<String> = s
            .split(',')
            .map(|x| format!("source:{}", x.trim()))
            .collect();
        query.push(format!("({})", any.join(" OR ")));
    }
    let body =
        json!({"query": query.join(" "), "from": start * 1000, "to": end * 1000, "limit": 1000});
    let mut v = match ui(&st, Method::POST, "/events/search", Some(body)).await {
        Ok(v) => v,
        Err(e) => return failed(e),
    };
    let Value::Array(rows) = v["logs"].take() else {
        return Json(json!({"events": []})).into_response();
    };
    let events: Vec<Value> = rows
        .into_iter()
        .map(|e| {
            let message = e["message"].as_str().unwrap_or("");
            let (title, rest) = message.split_once('\n').unwrap_or((message, ""));
            json!({
                "id": e["id"],
                "title": e.pointer("/attributes/evt/title").and_then(Value::as_str).unwrap_or(title),
                "text": e.pointer("/attributes/evt/text").and_then(Value::as_str).unwrap_or(rest),
                "date_happened": e["timestamp"].as_i64().unwrap_or(0) / 1000,
                "alert_type": match e["status"].as_str() {
                    Some("error") => "error",
                    Some("warn") => "warning",
                    Some("ok") => "success",
                    _ => "info",
                },
                "priority": "normal",
                "host": e["host"],
                "source": e["source"],
                "tags": e["tags"],
                "url": format!("/event/explorer?query={}", e["id"]),
            })
        })
        .collect();
    Json(json!({"events": events})).into_response()
}

/// Splits `a{x,y},b{*}` on the commas outside braces.
fn split_queries(q: &str) -> Vec<String> {
    let (mut out, mut depth, mut cur) = (Vec::new(), 0, String::new());
    for c in q.chars() {
        match c {
            '{' | '(' => depth += 1,
            '}' | ')' => depth -= 1,
            ',' if depth == 0 => {
                out.push(std::mem::take(&mut cur));
                continue;
            }
            _ => {}
        }
        cur.push(c);
    }
    out.push(cur);
    out.into_iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// `GET /api/v1/query?from=&to=&query=` (epoch seconds).
pub async fn query_v1(State(st): State<AppState>, Query(q): Params) -> Response {
    let secs = |k: &str| q.get(k).and_then(|v| v.parse::<i64>().ok());
    let (Some(from), Some(to), Some(query)) = (secs("from"), secs("to"), q.get("query")) else {
        return bad("from, to (epoch seconds) and query are required");
    };
    let queries = split_queries(query);
    let body =
        json!({"queries": queries, "from": from * 1000, "to": to * 1000, "max_series": 1000});
    let v = match ui(&st, Method::POST, "/metrics/query", Some(body)).await {
        Ok(v) => v,
        Err(e) => return failed(e),
    };
    let results = v["results"].as_array().cloned().unwrap_or_default();
    if let Some(e) = results.iter().find_map(|r| r["error"].as_str())
        && !e.starts_with("no data")
    {
        return bad(e);
    }
    let interval = v["interval_ms"].as_i64().unwrap_or(0) / 1000;
    let series: Vec<Value> = v["series"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|s| {
            let i = s["query_index"].as_u64().unwrap_or(0) as usize;
            let r = results.get(i).cloned().unwrap_or(Value::Null);
            let tags: Vec<String> = s["group"]
                .as_object()
                .into_iter()
                .flatten()
                .map(|(k, v)| format!("{k}:{}", v.as_str().unwrap_or("")))
                .collect();
            let points = s["points"].as_array().cloned().unwrap_or_default();
            let at = |p: Option<&Value>| p.and_then(|p| p[0].as_i64()).unwrap_or(0);
            json!({
                "metric": r["metric"],
                "display_name": r["metric"],
                "expression": r["query"],
                "query_index": i,
                "aggr": r["query"].as_str().and_then(|q| q.split(':').next()),
                "scope": if tags.is_empty() { "*".to_string() } else { tags.join(",") },
                "tag_set": tags,
                "unit": [r["unit"].as_str().map(|u| json!({"name": u, "short_name": u, "family": null, "scale_factor": 1.0, "plural": u})), null],
                "interval": interval,
                "length": points.len(),
                "start": at(points.first()),
                "end": at(points.last()),
                "pointlist": points,
            })
        })
        .collect();
    Json(json!({
        "status": "ok",
        "res_type": "time_series",
        "resp_version": 1,
        "query": query,
        "from_date": from * 1000,
        "to_date": to * 1000,
        "series": series,
        "values": [],
        "times": [],
        "message": "",
        "group_by": [],
    }))
    .into_response()
}

async fn metric_names(
    st: &AppState,
    since_ms: Option<i64>,
    contains: Option<&str>,
) -> Result<Vec<String>, String> {
    let v = ui(st, Method::GET, "/metrics", None).await?;
    Ok(v.as_array()
        .into_iter()
        .flatten()
        .filter(|m| since_ms.is_none_or(|s| m["last_seen"].as_i64().unwrap_or(0) >= s))
        .filter_map(|m| m["name"].as_str())
        .filter(|n| contains.is_none_or(|c| n.contains(c)))
        .map(String::from)
        .collect())
}

/// `GET /api/v1/metrics?from=<epoch seconds>`: the metrics active since.
pub async fn metrics_v1(State(st): State<AppState>, Query(q): Params) -> Response {
    let Some(from) = q.get("from").and_then(|f| f.parse::<i64>().ok()) else {
        return bad("from is required (epoch seconds)");
    };
    match metric_names(&st, Some(from * 1000), None).await {
        Ok(names) => Json(json!({"metrics": names, "from": from.to_string()})).into_response(),
        Err(e) => failed(e),
    }
}

/// `GET /api/v2/metrics`.
pub async fn metrics_v2(State(st): State<AppState>) -> Response {
    match metric_names(&st, None, None).await {
        Ok(names) => Json(json!({
            "data": names.iter().map(|n| json!({"id": n, "type": "metrics"})).collect::<Vec<_>>()
        }))
        .into_response(),
        Err(e) => failed(e),
    }
}

/// `GET /api/v1/search?q=metrics:cpu` or `hosts:web`.
pub async fn search_v1(State(st): State<AppState>, Query(q): Params) -> Response {
    let query = q.get("q").map(String::as_str).unwrap_or("");
    let (kind, needle) = query.split_once(':').unwrap_or(("", query));
    let mut results = Map::new();
    if kind != "hosts" {
        match metric_names(&st, None, Some(needle)).await {
            Ok(n) => results.insert("metrics".into(), json!(n)),
            Err(e) => return failed(e),
        };
    }
    if kind != "metrics" {
        let hosts = match ui(&st, Method::GET, "/hosts", None).await {
            Ok(h) => h,
            Err(e) => return failed(e),
        };
        let names: Vec<&str> = hosts
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|h| h["name"].as_str())
            .filter(|n| n.contains(needle))
            .collect();
        results.insert("hosts".into(), json!(names));
    }
    Json(json!({"results": results})).into_response()
}

fn metadata(m: &Value) -> Value {
    json!({
        "type": m["type"],
        "description": m["description"],
        "unit": m["unit"],
        "short_name": null,
        "per_unit": null,
        "integration": null,
        "statsd_interval": null,
    })
}

/// `GET /api/v1/metrics/{name}`: its metadata.
pub async fn metric_get(State(st): State<AppState>, Path(name): Path<String>) -> Response {
    let all = match ui(&st, Method::GET, "/metrics", None).await {
        Ok(v) => v,
        Err(e) => return failed(e),
    };
    match all
        .as_array()
        .into_iter()
        .flatten()
        .find(|m| m["name"] == name.as_str())
    {
        Some(m) => Json(metadata(m)).into_response(),
        None => failed("not found".into()),
    }
}

/// `PUT /api/v1/metrics/{name}`: unit and description.
pub async fn metric_put(
    State(st): State<AppState>,
    Path(name): Path<String>,
    Json(b): Json<Value>,
) -> Response {
    let body = json!({"unit": b["unit"], "description": b["description"]});
    let path = format!("/metrics/{}", crate::mcp::enc(&name));
    match ui(&st, Method::PATCH, &path, Some(body)).await {
        Ok(m) => Json(metadata(&m)).into_response(),
        Err(e) => failed(e),
    }
}

async fn host_list(st: &AppState, filter: Option<&str>) -> Result<Vec<Value>, String> {
    let v = ui(st, Method::GET, "/hosts", None).await?;
    let needle = filter.unwrap_or("").to_lowercase();
    Ok(v.as_array()
        .into_iter()
        .flatten()
        .filter(|h| needle.is_empty() || h.to_string().to_lowercase().contains(&needle))
        .map(|h| {
            json!({
                "name": h["name"],
                "host_name": h["name"],
                "aliases": h["aliases"],
                "apps": ["agent"],
                "sources": ["agent"],
                "up": true,
                "is_muted": false,
                "last_reported_time": now_ms() / 1000,
                "tags_by_source": {"Datadog": h["tags"]},
                "meta": {"agent_version": h["agent_version"], "platform": h["os"]},
                "metrics": {},
            })
        })
        .collect())
}

/// `GET /api/v1/hosts?filter=`.
pub async fn hosts(State(st): State<AppState>, Query(q): Params) -> Response {
    match host_list(&st, q.get("filter").map(String::as_str)).await {
        Ok(h) => {
            Json(json!({"total_matching": h.len(), "total_returned": h.len(), "host_list": h}))
                .into_response()
        }
        Err(e) => failed(e),
    }
}

/// `GET /api/v1/hosts/totals`.
pub async fn hosts_totals(State(st): State<AppState>) -> Response {
    match host_list(&st, None).await {
        Ok(h) => Json(json!({"total_active": h.len(), "total_up": h.len()})).into_response(),
        Err(e) => failed(e),
    }
}

// --- monitors --------------------------------------------------------------

/// Whether a monitor matches Datadog's monitor search syntax: `status:alert`,
/// `type:metric`, `muted:true`, `tag:"env:prod"` or a bare `env:prod`, and
/// words looked for in the name.
fn monitor_matches(m: &Value, query: &str) -> bool {
    let lower = |v: &Value| v.as_str().unwrap_or("").to_lowercase();
    let tags: Vec<String> = m["tags"]
        .as_array()
        .map(|t| t.iter().map(lower).collect())
        .unwrap_or_default();
    let muted = m["options"]["silenced"]
        .as_object()
        .is_some_and(|s| !s.is_empty());
    query.split_whitespace().all(|word| {
        let word = word.to_lowercase().replace('"', "");
        match word.split_once(':') {
            Some(("status", v)) => {
                lower(&m["overall_state"]).replace(' ', "") == v.replace(' ', "")
            }
            Some(("type", v)) => lower(&m["type"]).starts_with(v),
            Some(("muted", v)) => muted == (v == "true"),
            Some(("id", v)) => m["id"].as_i64().is_some_and(|i| v.parse() == Ok(i)),
            Some(("tag", v)) => tags.iter().any(|t| t == v),
            Some(_) => tags.contains(&word),
            None => lower(&m["name"]).contains(&word),
        }
    })
}

fn counts(values: impl Iterator<Item = String>) -> Vec<Value> {
    let mut n: HashMap<String, i64> = HashMap::new();
    for v in values {
        *n.entry(v).or_default() += 1;
    }
    let mut out: Vec<_> = n.into_iter().collect();
    out.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    out.into_iter()
        .map(|(name, count)| json!({"name": name, "count": count}))
        .collect()
}

/// `GET /api/v1/monitor/search?query=…&page=0&per_page=30`.
pub async fn monitor_search(
    State(st): State<AppState>,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let all = match ui(&st, Method::GET, "/monitors", None).await {
        Ok(v) => v.as_array().cloned().unwrap_or_default(),
        Err(e) => return failed(e),
    };
    let query = q.get("query").map(String::as_str).unwrap_or("");
    let found: Vec<&Value> = all.iter().filter(|m| monitor_matches(m, query)).collect();
    let per_page = q
        .get("per_page")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(30)
        .clamp(1, 100);
    let page = q
        .get("page")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(0);
    let name = |m: &Value, k: &str| m[k].as_str().unwrap_or("").to_string();
    let monitors: Vec<Value> = found
        .iter()
        .skip(page * per_page)
        .take(per_page)
        .map(|m| {
            let typ = name(m, "type");
            let handles: Vec<Value> = name(m, "message")
                .split_whitespace()
                .filter_map(|w| w.strip_prefix('@'))
                .map(|h| h.trim_end_matches(['.', ',', ';']))
                .filter(|h| !h.is_empty())
                .map(|h| json!({"handle": h, "name": h}))
                .collect();
            json!({
                "id": m["id"],
                "name": m["name"],
                "status": m["overall_state"],
                "query": m["query"],
                "type": typ,
                "classification": typ.split_whitespace().next().unwrap_or("metric"),
                "tags": m["tags"],
                "creator": m["creator"],
                "org_id": m["org_id"],
                "last_triggered_ts": m["last_triggered_ts"],
                "notifications": handles,
                "metrics": [],
                "scopes": [],
            })
        })
        .collect();
    Json(json!({
        "monitors": monitors,
        "counts": {
            "status": counts(found.iter().map(|m| name(m, "overall_state"))),
            "type": counts(found.iter().map(|m| name(m, "type"))),
            "muted": counts(found.iter().map(|m| {
                m["options"]["silenced"].as_object().is_some_and(|s| !s.is_empty()).to_string()
            })),
            "tag": counts(found.iter().flat_map(|m| {
                m["tags"].as_array().cloned().unwrap_or_default().into_iter()
                    .filter_map(|t| t.as_str().map(String::from))
            })),
        },
        "metadata": {
            "page": page,
            "per_page": per_page,
            "page_count": found.len().div_ceil(per_page),
            "total_count": found.len(),
        },
    }))
    .into_response()
}

// --- incidents -------------------------------------------------------------

fn incident_out(i: &Value) -> Value {
    let date = |k: &str| i[k].as_i64().map(iso);
    json!({
        "id": i["id"],
        "type": "incidents",
        "attributes": {
            "public_id": i["public_id"],
            "title": i["title"],
            "severity": i["severity"],
            "state": i["state"],
            "created": date("created"),
            "modified": date("modified"),
            "detected": date("created"),
            "resolved": date("resolved"),
            "customer_impacted": i["public"],
            "customer_impact_scope": i["summary"],
            "fields": {
                "severity": {"type": "dropdown", "value": i["severity"]},
                "state": {"type": "dropdown", "value": i["state"]},
                "summary": {"type": "textbox", "value": i["summary"]},
                "services": {"type": "autocomplete", "value": i["services"]},
            },
        },
    })
}

/// Datadog's attributes as the fields an incident has here.
fn incident_in(body: &Value) -> Value {
    let a = &body["data"]["attributes"];
    let field = |name: &str| {
        let v = &a["fields"][name]["value"];
        if v.is_null() {
            a[name].clone()
        } else {
            v.clone()
        }
    };
    let mut p = Map::new();
    for (k, v) in [
        ("title", a["title"].clone()),
        ("severity", field("severity")),
        ("state", field("state")),
        ("summary", field("summary")),
        ("services", field("services")),
        ("public", a["customer_impacted"].clone()),
    ] {
        if !v.is_null() {
            p.insert(k.into(), v);
        }
    }
    Value::Object(p)
}

/// `GET /api/v2/incidents`.
pub async fn incidents(State(st): State<AppState>) -> Response {
    match ui(&st, Method::GET, "/incidents", None).await {
        Ok(v) => {
            let data: Vec<Value> = v["incidents"]
                .as_array()
                .into_iter()
                .flatten()
                .map(incident_out)
                .collect();
            let n = data.len();
            Json(json!({"data": data, "meta": {"pagination": {"offset": 0, "next_offset": n, "size": n}}})).into_response()
        }
        Err(e) => failed(e),
    }
}

/// `POST /api/v2/incidents`.
pub async fn incident_create(State(st): State<AppState>, Json(b): Json<Value>) -> Response {
    match ui(&st, Method::POST, "/incidents", Some(incident_in(&b))).await {
        Ok(i) => (StatusCode::CREATED, Json(json!({"data": incident_out(&i)}))).into_response(),
        Err(e) => failed(e),
    }
}

/// An incident by its id or by its number (`12`).
async fn incident_id(st: &AppState, id: &str) -> Result<String, String> {
    let Ok(number) = id.parse::<i64>() else {
        return Ok(crate::mcp::enc(id));
    };
    let v = ui(st, Method::GET, "/incidents", None).await?;
    v["incidents"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|i| i["public_id"].as_i64() == Some(number))
        .and_then(|i| i["id"].as_str().map(String::from))
        .ok_or_else(|| "not found".into())
}

/// `GET|PATCH|DELETE /api/v2/incidents/{id}`.
pub async fn incident(
    State(st): State<AppState>,
    method: Method,
    Path(id): Path<String>,
    body: Option<Json<Value>>,
) -> Response {
    let id = match incident_id(&st, &id).await {
        Ok(id) => id,
        Err(e) => return failed(e),
    };
    let path = format!("/incidents/{id}");
    let res = match (method, body) {
        (Method::PATCH, Some(Json(b))) => ui(&st, Method::PUT, &path, Some(incident_in(&b))).await,
        (Method::DELETE, _) => {
            return match ui(&st, Method::DELETE, &path, None).await {
                Ok(_) => StatusCode::NO_CONTENT.into_response(),
                Err(e) => failed(e),
            };
        }
        _ => ui(&st, Method::GET, &path, None).await,
    };
    match res {
        Ok(i) => Json(json!({"data": incident_out(&i)})).into_response(),
        Err(e) => failed(e),
    }
}

// --- notebooks ---------------------------------------------------------------

/// Datadog's notebook ids are numbers; older ones here are not.
fn notebook_id(id: &Value) -> Value {
    id.as_str()
        .and_then(|s| s.parse::<i64>().ok())
        .map_or_else(|| id.clone(), |n| json!(n))
}

fn notebook_out(nb: &Value, modified: Option<i64>) -> Value {
    let cells: Vec<Value> = nb["cells"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|c| {
            let text = c["text"].as_str().unwrap_or("");
            let definition = match c["type"].as_str() {
                Some("metric") => json!({"type": "timeseries", "show_legend": true, "requests": [{"q": text, "display_type": "line"}]}),
                Some("logs") => json!({"type": "log_stream", "query": text, "indexes": []}),
                _ => json!({"type": "markdown", "text": text}),
            };
            json!({"id": c["id"], "type": "notebook_cells", "attributes": {"definition": definition}})
        })
        .collect();
    json!({
        "id": notebook_id(&nb["id"]),
        "type": "notebooks",
        "attributes": {
            "name": nb["name"],
            "cells": cells,
            "time": {"live_span": "1h"},
            "status": "published",
            "modified": modified.map(iso),
            "metadata": {"is_template": false, "take_snapshots": false, "type": null},
        },
    })
}

fn notebook_in(body: &Value) -> Value {
    let a = &body["data"]["attributes"];
    let cells: Vec<Value> = a["cells"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|c| {
            let d = &c["attributes"]["definition"];
            let (typ, text) = match d["type"].as_str() {
                Some("markdown") => ("markdown", d["text"].as_str().unwrap_or("").to_string()),
                Some("timeseries") => (
                    "metric",
                    d["requests"][0]["q"]
                        .as_str()
                        .or_else(|| d["requests"][0]["queries"][0]["query"].as_str())
                        .unwrap_or("")
                        .to_string(),
                ),
                Some("log_stream") => ("logs", d["query"].as_str().unwrap_or("").to_string()),
                // Widgets with no cell here are kept as their definition.
                _ => ("markdown", format!("```\n{d}\n```")),
            };
            let id = c["id"]
                .as_str()
                .map_or_else(crate::store::pg::new_id, String::from);
            json!({"id": id, "type": typ, "text": text})
        })
        .collect();
    json!({"name": a["name"], "cells": cells})
}

/// `GET /api/v1/notebooks`.
pub async fn notebooks(State(st): State<AppState>) -> Response {
    let list = match ui(&st, Method::GET, "/notebooks", None).await {
        Ok(v) => v,
        Err(e) => return failed(e),
    };
    let mut data = Vec::new();
    for n in list["data"].as_array().into_iter().flatten() {
        let Some(id) = n["id"].as_str() else { continue };
        if let Ok(nb) = ui(
            &st,
            Method::GET,
            &format!("/notebooks/{}", crate::mcp::enc(id)),
            None,
        )
        .await
        {
            data.push(notebook_out(&nb, n["modified_at"].as_i64()));
        }
    }
    let n = data.len();
    Json(json!({"data": data, "meta": {"page": {"total_count": n, "total_filtered_count": n}}}))
        .into_response()
}

/// `POST /api/v1/notebooks`.
pub async fn notebook_create(State(st): State<AppState>, Json(b): Json<Value>) -> Response {
    match ui(&st, Method::POST, "/notebooks", Some(notebook_in(&b))).await {
        Ok(nb) => Json(json!({"data": notebook_out(&nb, Some(now_ms()))})).into_response(),
        Err(e) => failed(e),
    }
}

/// `GET|PUT|DELETE /api/v1/notebooks/{id}`.
pub async fn notebook(
    State(st): State<AppState>,
    method: Method,
    Path(id): Path<String>,
    body: Option<Json<Value>>,
) -> Response {
    let path = format!("/notebooks/{}", crate::mcp::enc(&id));
    let res = match (method, body) {
        (Method::PUT, Some(Json(b))) => ui(&st, Method::PUT, &path, Some(notebook_in(&b))).await,
        (Method::DELETE, _) => {
            return match ui(&st, Method::DELETE, &path, None).await {
                Ok(_) => StatusCode::NO_CONTENT.into_response(),
                Err(e) => failed(e),
            };
        }
        _ => ui(&st, Method::GET, &path, None).await,
    };
    match res {
        Ok(nb) => Json(json!({"data": notebook_out(&nb, None)})).into_response(),
        Err(e) => failed(e),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn monitor_search_syntax() {
        let m = json!({"name": "High errors", "type": "log alert", "overall_state": "No Data",
                       "tags": ["team:core", "env:prod"], "options": {"silenced": {}}, "id": 7});
        assert!(monitor_matches(&m, ""));
        assert!(monitor_matches(
            &m,
            "errors type:log status:\"no data\""
                .replace("no data", "nodata")
                .as_str()
        ));
        assert!(monitor_matches(
            &m,
            "tag:\"env:prod\" team:core muted:false id:7"
        ));
        assert!(!monitor_matches(&m, "status:alert"));
        assert!(!monitor_matches(&m, "env:dev"));
    }

    use super::*;

    #[test]
    fn translates_requests_and_documents() {
        let s = Search::from_body(&json!({"filter": {"query": "service:a", "from": "now-1h", "to": "now"}, "page": {"limit": 5000, "cursor": "c"}})).unwrap();
        assert_eq!(
            (s.query.as_str(), s.limit, s.cursor.as_deref()),
            ("service:a", 1000, Some("c"))
        );
        assert_eq!(s.to - s.from, 3_600_000);
        let wrapped = Search::from_body(&json!({"data": {"attributes": {"filter": {"query": "*", "from": "1794910470000", "to": "1794910471000"}}}})).unwrap();
        assert_eq!(
            (wrapped.query.as_str(), wrapped.from),
            ("", 1_794_910_470_000)
        );
        let q = HashMap::from([
            ("filter[query]".to_string(), "status:error".to_string()),
            ("page[limit]".to_string(), "3".to_string()),
        ]);
        assert_eq!(Search::from_query(&q).unwrap().limit, 3);
        assert!(Search::from_body(&json!({"filter": {"from": "soon"}})).is_err());

        assert_eq!(
            split_queries("avg:a{x,y} by {h}, sum:b{*}"),
            ["avg:a{x,y} by {h}", "sum:b{*}"]
        );

        let p = incident_in(
            &json!({"data": {"attributes": {"title": "Down", "customer_impacted": true,
            "fields": {"severity": {"type": "dropdown", "value": "SEV-1"}, "state": {"value": "resolved"}}}}}),
        );
        assert_eq!(
            p,
            json!({"title": "Down", "severity": "SEV-1", "state": "resolved", "public": true})
        );

        let nb = json!({"id": "1794910470000", "name": "N", "cells": [
            {"id": "a", "type": "markdown", "text": "# T"}, {"id": "b", "type": "metric", "text": "avg:m{*}"}, {"id": "c", "type": "logs", "text": "status:error"}]});
        let out = notebook_out(&nb, None);
        assert_eq!(out["id"], 1_794_910_470_000i64);
        assert_eq!(
            out["attributes"]["cells"][1]["attributes"]["definition"]["requests"][0]["q"],
            "avg:m{*}"
        );
        let back = notebook_in(&json!({"data": out}));
        assert_eq!(back["cells"], nb["cells"]);
    }
}
