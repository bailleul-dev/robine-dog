//! Real User Monitoring: the events of Datadog's browser SDK
//! (`@datadog/browser-rum`), pointed here with its `proxy` option:
//!
//! ```js
//! datadogRum.init({ applicationId: "shop", clientToken: "<client token>",
//!                   proxy: "https://<robine-dog>/rum/proxy", service: "shop-web", env: "prod" })
//! ```
//!
//! The SDK then posts to `/rum/proxy?ddforward=/api/v2/rum?…` batches of
//! newline-delimited events (views, actions, resources, errors, long tasks).
//! `/api/v2/rum` takes the same directly. They are stored with the columns
//! of logs: `source` is the event type, `message` what it is about, and the
//! whole event is the attributes (`@view.url_path`, `@session.id`…). A view
//! is sent again each time it changes: readers keep its latest values.
//!
//! Errors are also stored as error logs (`source:browser`), so Error
//! Tracking groups them. Browser logs (`@datadog/browser-logs`) come through
//! the same proxy; session replays are dropped.

use crate::AppState;
use crate::api::logs::{HistogramReq, HistogramResp, SearchReq, SearchResp};
use crate::api::{ApiResult, Scope, compile, time_clause};
use crate::intake::logs::{Defaults, LogRow, decompress, now_ms, process, split_tags};
use crate::intake::sentry::{authorized, cors};
use crate::query::sql::Dataset;
use crate::store::greptime::{f64_at, i64_at, str_at};
use crate::store::writer::Insertable;
use axum::Json;
use axum::body::Bytes;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use serde_json::{Map, Value, json};
use std::collections::HashMap;

const TABLE: &str = "rum_events";

/// A [`LogRow`] bound for the `rum_events` table.
pub struct RumRow(pub LogRow);

impl Insertable for RumRow {
    const TABLE: &'static str = TABLE;
    const COLUMNS: &'static str = <LogRow as Insertable>::COLUMNS;

    fn values(&self, out: &mut String) {
        self.0.values(out)
    }
}

fn text(v: Option<&Value>) -> Option<String> {
    match v? {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// `https://shop.example/cart/12?x=1` → (`shop.example`, `/cart/12`).
fn url_parts(url: &str) -> (Option<&str>, &str) {
    let Some((_, rest)) = url.split_once("://") else {
        return (None, url.split(['?', '#']).next().unwrap_or(url));
    };
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let path = rest[end..].split(['?', '#']).next().unwrap_or("");
    (Some(&rest[..end]), if path.is_empty() { "/" } else { path })
}

/// What an event is about, in a line.
fn summary(typ: &str, e: &Value) -> String {
    let at = |p: &str| text(e.pointer(p));
    match typ {
        "view" => at("/view/name")
            .or_else(|| at("/view/url_path"))
            .or_else(|| at("/view/url"))
            .unwrap_or_else(|| "view".into()),
        "action" => format!(
            "{} {}",
            at("/action/type").unwrap_or_else(|| "action".into()),
            at("/action/target/name").unwrap_or_default()
        )
        .trim()
        .to_string(),
        "resource" => format!(
            "{} {}",
            at("/resource/method").unwrap_or_else(|| "GET".into()),
            at("/resource/url").unwrap_or_default()
        ),
        "error" => at("/error/message").unwrap_or_else(|| "error".into()),
        "long_task" => format!(
            "long task of {:.0} ms",
            e.pointer("/long_task/duration")
                .and_then(Value::as_f64)
                .unwrap_or(0.0)
                / 1e6
        ),
        other => other.to_string(),
    }
}

/// One SDK event as a row, plus the error log an error event also is.
pub fn rows(mut e: Value, tags: &[String], now: i64) -> Option<(LogRow, Option<LogRow>)> {
    let typ = text(e.get("type"))?;
    // The SDK's reports about itself are Datadog's business.
    if typ == "telemetry" {
        return None;
    }
    // The path and host of the view, as Datadog adds them.
    if let Some(url) = text(e.pointer("/view/url")) {
        let (host, path) = url_parts(&url);
        let (host, path) = (host.map(String::from), path.to_string());
        if let Some(view) = e.get_mut("view").and_then(Value::as_object_mut) {
            view.entry("url_path").or_insert(json!(path));
            if let Some(h) = host {
                view.entry("url_host").or_insert(json!(h));
            }
        }
    }
    let service = text(e.get("service")).or_else(|| {
        tags.iter()
            .find_map(|t| t.strip_prefix("service:"))
            .map(String::from)
    });
    let ts = e.get("date").and_then(Value::as_i64).unwrap_or(now);
    let message = summary(&typ, &e);
    // Since version 6 the SDK puts its tags in each event, not in the URL.
    let mut tags = tags.to_vec();
    tags.extend(split_tags(&text(e.get("ddtags")).unwrap_or_default()));
    tags.retain(|t| !t.starts_with("service:"));
    tags.sort();
    tags.dedup();
    let error_log = (typ == "error").then(|| {
        let mut attrs = Map::new();
        let mut error = Map::new();
        error.insert(
            "kind".into(),
            json!(text(e.pointer("/error/type")).unwrap_or_else(|| "Error".into())),
        );
        error.insert("message".into(), json!(message));
        for k in ["stack", "source", "handling"] {
            if let Some(v) = text(e.pointer(&format!("/error/{k}"))) {
                error.insert(k.into(), json!(v));
            }
        }
        attrs.insert("error".into(), Value::Object(error));
        for k in ["view", "usr", "session", "application"] {
            if let Some(v) = e.get(k) {
                attrs.insert(k.into(), v.clone());
            }
        }
        LogRow {
            ts,
            status: "error".into(),
            service: service.clone(),
            host: None,
            source: Some("browser".into()),
            message: message.clone(),
            tags: tags.clone(),
            attributes: attrs,
            trace_id: None,
            span_id: None,
        }
    });
    let Value::Object(attributes) = e else {
        return None;
    };
    Some((
        LogRow {
            ts,
            status: if typ == "error" { "error" } else { "info" }.into(),
            service,
            host: None,
            source: Some(typ),
            message,
            tags,
            attributes,
            trace_id: None,
            span_id: None,
        },
        error_log,
    ))
}

fn json_lines(raw: &[u8]) -> impl Iterator<Item = Value> + '_ {
    raw.split(|b| *b == b'\n')
        .filter(|l| !l.iter().all(u8::is_ascii_whitespace))
        .filter_map(|l| serde_json::from_slice::<Value>(l).ok())
}

async fn store_rum(st: &AppState, raw: &[u8], q: &HashMap<String, String>) {
    let tags = split_tags(q.get("ddtags").map(String::as_str).unwrap_or(""));
    let now = now_ms();
    let (mut events, mut errors) = (Vec::new(), Vec::new());
    for e in json_lines(raw) {
        if let Some((row, error)) = rows(e, &tags, now) {
            events.push(RumRow(row));
            errors.extend(error);
        }
    }
    tracing::debug!(count = events.len(), "rum events received");
    if let Err(e) = st.store.rum.send(events).await {
        tracing::warn!(error = %e, "storing rum events");
    }
    for row in &mut errors {
        st.pipelines.apply(row);
        st.generated.on_log(&st.aggregator, row);
    }
    if let Err(e) = st.store.logs.send(errors).await {
        tracing::warn!(error = %e, "storing rum errors");
    }
}

async fn store_logs(st: &AppState, raw: &[u8], q: &HashMap<String, String>) {
    let defaults = Defaults {
        source: Some(
            q.get("ddsource")
                .cloned()
                .unwrap_or_else(|| "browser".into()),
        ),
        service: None,
        host: None,
        tags: split_tags(q.get("ddtags").map(String::as_str).unwrap_or("")),
    };
    let now = now_ms();
    let rows: Vec<LogRow> = json_lines(raw)
        .filter_map(|v| match v {
            Value::Object(o) => Some(o),
            _ => None,
        })
        .map(|o| {
            let mut row = process(o, &defaults, now);
            st.pipelines.apply(&mut row);
            st.generated.on_log(&st.aggregator, &row);
            row
        })
        .collect();
    if let Err(e) = st.store.logs.send(rows).await {
        tracing::warn!(error = %e, "storing browser logs");
    }
}

async fn ingest(
    st: AppState,
    path: &str,
    q: HashMap<String, String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let key = q.get("dd-api-key").cloned().or_else(|| {
        headers
            .get("dd-api-key")
            .and_then(|v| v.to_str().ok())
            .map(String::from)
    });
    if !authorized(&st, key.as_deref()) {
        return cors(StatusCode::FORBIDDEN.into_response());
    }
    let Ok(raw) = decompress(&headers, body) else {
        return cors((StatusCode::BAD_REQUEST, "bad compression").into_response());
    };
    if path.ends_with("/rum") {
        store_rum(&st, &raw, &q).await;
    } else if path.ends_with("/logs") {
        store_logs(&st, &raw, &q).await;
    }
    // Replays and anything else: accepted and dropped.
    cors((StatusCode::ACCEPTED, Json(json!({}))).into_response())
}

/// `POST /api/v2/rum?ddsource=browser&ddtags=…&dd-api-key=<client token>`.
pub async fn intake(
    State(st): State<AppState>,
    Query(q): Query<HashMap<String, String>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    ingest(st, "/api/v2/rum", q, headers, body).await
}

/// The SDK's `proxy` option: `?ddforward=<path and query, URL-encoded>`.
pub async fn proxy(
    State(st): State<AppState>,
    Query(q): Query<HashMap<String, String>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let forward = q.get("ddforward").cloned().unwrap_or_default();
    let (path, query) = forward.split_once('?').unwrap_or((forward.as_str(), ""));
    let inner: HashMap<String, String> = serde_urlencoded::from_str(query).unwrap_or_default();
    ingest(st, path, inner, headers, body).await
}

// --- reading -------------------------------------------------------------

pub async fn search(st: State<AppState>, req: Json<SearchReq>) -> ApiResult<SearchResp> {
    crate::api::logs::search_table(st, req, TABLE).await
}

pub async fn histogram(st: State<AppState>, req: Json<HistogramReq>) -> ApiResult<HistogramResp> {
    crate::api::logs::histogram_table(st, req, TABLE).await
}

#[derive(Serialize)]
pub struct ViewStats {
    service: Option<String>,
    name: String,
    views: i64,
    /// Nanoseconds, 75th percentile.
    loading_time: Option<f64>,
    largest_contentful_paint: Option<f64>,
    first_contentful_paint: Option<f64>,
    interaction_to_next_paint: Option<f64>,
    cumulative_layout_shift: Option<f64>,
    errors: i64,
}

#[derive(Serialize)]
pub struct Performance {
    sessions: i64,
    views: i64,
    errors: i64,
    actions: i64,
    pages: Vec<ViewStats>,
}

/// Web vitals per page, at the 75th percentile as Datadog shows them.
pub async fn performance(
    State(st): State<AppState>,
    Json(s): Json<Scope>,
) -> ApiResult<Performance> {
    let filter = compile(&s.query, Dataset::Logs)?;
    let time = time_clause(Dataset::Logs, s.from, s.to);
    let f = |k: &str| format!(r#"max(json_get_float(attributes, '$["view"]["{k}"]'))"#);
    // A view is sent each time it changes: its values only settle or grow.
    let pages_sql = format!(
        r#"SELECT service, name, count(*) AS n,
             approx_percentile_cont(lt, 0.75), approx_percentile_cont(lcp, 0.75), approx_percentile_cont(fcp, 0.75),
             approx_percentile_cont(inp, 0.75), approx_percentile_cont(cls, 0.75), CAST(sum(coalesce(errs, 0)) AS BIGINT)
           FROM (
             SELECT service, max(message) AS name, json_get(attributes, '$["view"]["id"]') AS vid,
               {} AS lt, {} AS lcp, {} AS fcp, {} AS inp, {} AS cls,
               max(json_get_float(attributes, '$["view"]["error"]["count"]')) AS errs
             FROM {TABLE} WHERE {time} AND source = 'view' AND {filter} GROUP BY service, vid
           ) AS v GROUP BY service, name ORDER BY n DESC LIMIT 200"#,
        f("loading_time"),
        f("largest_contentful_paint"),
        f("first_contentful_paint"),
        f("interaction_to_next_paint"),
        f("cumulative_layout_shift"),
    );
    let totals_sql = format!(
        r#"SELECT source, count(*), count(DISTINCT json_get(attributes, '$["session"]["id"]')),
             count(DISTINCT json_get(attributes, '$["view"]["id"]'))
           FROM {TABLE} WHERE {time} AND {filter} GROUP BY source"#
    );
    let (pages, totals) = tokio::try_join!(
        st.store.gt.query(&pages_sql),
        st.store.gt.query(&totals_sql)
    )?;
    let count = |typ: &str, col: usize| {
        totals
            .iter()
            .find(|r| str_at(r, 0).as_deref() == Some(typ))
            .map_or(0, |r| i64_at(r, col))
    };
    Ok(Json(Performance {
        sessions: totals.iter().map(|r| i64_at(r, 2)).max().unwrap_or(0),
        views: count("view", 3),
        errors: count("error", 1),
        actions: count("action", 1),
        pages: pages
            .iter()
            .map(|r| ViewStats {
                service: str_at(r, 0),
                name: str_at(r, 1).unwrap_or_default(),
                views: i64_at(r, 2),
                loading_time: f64_at(r, 3),
                largest_contentful_paint: f64_at(r, 4),
                first_contentful_paint: f64_at(r, 5),
                interaction_to_next_paint: f64_at(r, 6),
                cumulative_layout_shift: f64_at(r, 7),
                errors: i64_at(r, 8),
            })
            .collect(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_sdk_events() {
        let tags = vec![
            "env:prod".to_string(),
            "service:shop-web".to_string(),
            "version:1.2".to_string(),
        ];
        let (view, none) = rows(
            json!({"type": "view", "date": 1794910470000i64, "application": {"id": "shop"}, "session": {"id": "s1"},
                   "view": {"id": "v1", "url": "https://shop.example/cart/12?x=1", "loading_time": 1200000000i64}}),
            &tags,
            0,
        )
        .unwrap();
        assert!(none.is_none());
        assert_eq!(view.ts, 1794910470000);
        assert_eq!(view.service.as_deref(), Some("shop-web"));
        assert_eq!(
            (view.source.as_deref(), view.message.as_str()),
            (Some("view"), "/cart/12")
        );
        assert_eq!(view.attributes["view"]["url_host"], "shop.example");
        assert_eq!(view.tags, ["env:prod", "version:1.2"]);

        let (err, log) = rows(
            json!({"type": "error", "date": 5, "service": "front", "view": {"url": "/x"},
                   "error": {"message": "x is undefined", "type": "TypeError", "stack": "TypeError: x is undefined\n  at f @ app.js:1:2", "source": "source"}}),
            &tags,
            0,
        )
        .unwrap();
        assert_eq!(
            (err.status.as_str(), err.service.as_deref()),
            ("error", Some("front"))
        );
        let log = log.unwrap();
        assert_eq!(log.source.as_deref(), Some("browser"));
        assert_eq!(log.attributes["error"]["kind"], "TypeError");
        assert_eq!(log.attributes["view"]["url_path"], "/x");

        let (res, _) = rows(
            json!({"type": "resource", "resource": {"method": "POST", "url": "https://api/x"}}),
            &[],
            9,
        )
        .unwrap();
        assert_eq!((res.message.as_str(), res.ts), ("POST https://api/x", 9));
        assert_eq!(json_lines(b"{\"a\":1}\n\nnope\n{\"b\":2}").count(), 2);
    }
}
