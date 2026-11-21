//! Synthetic tests run on a schedule from robine-dog, with the shape of
//! Datadog's Synthetics API: API tests (`subtype` `http`, `ssl`, `tcp`,
//! `dns`, `udp`, `websocket`, `grpc`, `icmp`, or `multi` for multistep
//! ones) and browser tests (`type: browser`, steps run in a headless
//! Chromium).
//!
//! Each run is a stored result (with its steps, and its screenshots for a
//! browser test), a log (`source:synthetics`, `@synthetics.check_id`) and
//! metrics, as in Datadog: `synthetics.test_runs` (tagged
//! `status:passed|failed`), `synthetics.<subtype>.response.time`
//! (milliseconds) or `synthetics.browser.test_duration`, all tagged
//! `check_id` and with the test's tags. A failing test alerts by itself.
//! Tests also run from CI (`/api/v1/synthetics/tests/trigger/ci`, see `ci`).

pub mod assertions;
pub mod browser;
pub mod ci;
mod dns;
pub mod multistep;
pub mod net;
pub mod variables;
mod x509;

use crate::AppState;
use crate::api::{ApiError, ApiResult};
use crate::intake::logs::LogRow;
use crate::metrics::Sample;
use crate::store::pg::SyntheticResult;
pub use assertions::Assertion;
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
pub use browser::BrowserStep;
pub use multistep::ApiStep;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::Duration;
use variables::Vars;

const KIND: &str = "synthetic";
const MIN_TICK_S: i64 = 30;
const MIN_BROWSER_TICK_S: i64 = 300;
/// Browser runs whose screenshots are kept, per test.
const KEEP_SCREENSHOTS: i64 = 50;

pub const API_SUBTYPES: &[&str] = &[
    "http",
    "ssl",
    "tcp",
    "dns",
    "udp",
    "websocket",
    "grpc",
    "icmp",
    "multi",
];

/// A request, for every protocol: HTTP and WebSocket ones have a `url`, the
/// others a `host` (and a `port`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Request {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub method: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub url: String,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub headers: Map<String, Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    /// Seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<f64>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub host: String,
    /// A number, or a string as Datadog accepts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<Value>,
    /// SSL: the name sent (SNI), the host's by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub servername: Option<String>,
    #[serde(rename = "dnsServer", default, skip_serializing_if = "Option::is_none")]
    pub dns_server: Option<String>,
    #[serde(
        rename = "dnsServerPort",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub dns_server_port: Option<Value>,
    /// UDP and WebSocket: what is sent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// gRPC: the service whose health is checked (empty: the server's).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service: Option<String>,
    #[serde(rename = "callType", default, skip_serializing_if = "Option::is_none")]
    pub call_type: Option<String>,
    #[serde(
        rename = "numberOfPackets",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub number_of_packets: Option<u32>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// A test's own variable: generated from a pattern (`text`), or a global
/// variable under another name (`global`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConfigVariable {
    pub name: String,
    #[serde(rename = "type", default)]
    pub typ: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pattern: Option<String>,
    /// The global variable's id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub example: Option<String>,
    #[serde(default)]
    pub secure: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub request: Request,
    #[serde(default)]
    pub assertions: Vec<Assertion>,
    /// Multistep API tests.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<ApiStep>,
    #[serde(
        rename = "configVariables",
        default,
        skip_serializing_if = "Vec::is_empty"
    )]
    pub config_variables: Vec<ConfigVariable>,
    /// Browser tests' variables.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub variables: Vec<ConfigVariable>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Retry {
    #[serde(default)]
    pub count: u32,
    /// Milliseconds.
    #[serde(default)]
    pub interval: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TestOptions {
    /// Seconds between two runs.
    #[serde(default)]
    pub tick_every: Option<i64>,
    /// Seconds a test must have been failing before it alerts (0: at once).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_failure_duration: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accept_self_signed: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub follow_redirects: Option<bool>,
    /// Browser tests: `chrome.laptop_large`, `chrome.tablet`,
    /// `chrome.mobile_small`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub device_ids: Vec<String>,
    /// Retries of a failed run before it counts as failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry: Option<Retry>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Test {
    #[serde(default)]
    pub public_id: Option<String>,
    pub name: String,
    /// `api` or `browser`.
    #[serde(rename = "type", default)]
    pub typ: String,
    #[serde(default)]
    pub subtype: String,
    /// `live` or `paused`.
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub config: Config,
    #[serde(default)]
    pub options: TestOptions,
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub locations: Vec<String>,
    /// Browser tests.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<BrowserStep>,
    /// Whether it is alerting; computed.
    #[serde(default, skip_deserializing, skip_serializing_if = "Option::is_none")]
    pub monitor_status: Option<String>,
    /// The latest run; computed.
    #[serde(default, skip_deserializing, skip_serializing_if = "Option::is_none")]
    pub last_result: Option<RunResult>,
}

impl Test {
    fn is_browser(&self) -> bool {
        self.typ == "browser"
    }

    /// `GET https://…`, `ssl shop.example:443`, `browser https://…`.
    pub fn target(&self) -> String {
        let r = &self.config.request;
        match (self.typ.as_str(), self.subtype.as_str()) {
            ("browser", _) => format!("browser {}", r.url),
            (_, "multi") => format!("{} steps", self.config.steps.len()),
            (_, "" | "http") => format!("{} {}", r.method, r.url),
            (_, "websocket") => r.url.clone(),
            (_, sub) => match &r.port {
                Some(p) => format!("{sub} {}:{}", r.host, assertions::as_text(p)),
                None => format!("{sub} {}", r.host),
            },
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StepResult {
    pub name: String,
    #[serde(rename = "type", default)]
    pub typ: String,
    pub passed: bool,
    pub duration_ms: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<String>,
    #[serde(default)]
    pub allow_failure: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_code: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub details: Value,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub extracted: Map<String, Value>,
    /// Whether a screenshot was kept
    /// (`/synthetics/results/{result_id}/screenshots/{step}`).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub screenshot: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RunResult {
    #[serde(default)]
    pub result_id: String,
    /// Epoch milliseconds.
    pub ts: i64,
    pub passed: bool,
    pub status_code: Option<u16>,
    pub duration_ms: f64,
    pub failure: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<StepResult>,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub details: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub batch_id: Option<String>,
    /// Runs before this one that failed, retried.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub retries: u32,
}

fn is_zero(n: &u32) -> bool {
    *n == 0
}

static LAST: LazyLock<Mutex<HashMap<String, RunResult>>> = LazyLock::new(Default::default);

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Datadog's result ids are numbers, as strings.
pub fn new_result_id() -> String {
    (rand::random::<u64>() >> 1).to_string()
}

/// What a run changes, from CI: the start URL, variables, headers…
#[derive(Debug, Clone, Default)]
pub struct Overrides {
    pub result_id: Option<String>,
    pub batch_id: Option<String>,
    pub start_url: Option<String>,
    pub variables: Vars,
    pub headers: Map<String, Value>,
    pub body: Option<String>,
    pub follow_redirects: Option<bool>,
    pub accept_self_signed: Option<bool>,
    pub device_id: Option<String>,
    pub default_step_timeout: Option<f64>,
    pub retry: Option<Retry>,
}

/// The variables of a run: global ones, the test's own, and the overrides.
async fn variables_for(st: &AppState, t: &Test, o: &Overrides) -> Vars {
    let globals = variables::all(st).await.unwrap_or_default();
    let mut vars: Vars = globals
        .iter()
        .map(|g| (g.name.clone(), g.value.value.clone()))
        .collect();
    let now = now_ms();
    for v in t.config.config_variables.iter().chain(&t.config.variables) {
        let value = match v.typ.as_str() {
            "global" => globals
                .iter()
                .find(|g| Some(&g.id) == v.id.as_ref())
                .map(|g| g.value.value.clone()),
            _ => v
                .pattern
                .as_deref()
                .map(|p| variables::render(&variables::generate(p, now), &vars)),
        };
        if let Some(value) = value {
            vars.insert(v.name.clone(), value);
        }
    }
    vars.extend(o.variables.clone());
    vars
}

/// One run, without retries; and the screenshots of a browser run.
async fn run_once(st: &AppState, t: &Test, o: &Overrides) -> (RunResult, Vec<(i32, Vec<u8>)>) {
    let ts = now_ms();
    let mut vars = variables_for(st, t, o).await;
    let mut opts = t.options.clone();
    if o.follow_redirects.is_some() {
        opts.follow_redirects = o.follow_redirects;
    }
    if o.accept_self_signed.is_some() {
        opts.accept_self_signed = o.accept_self_signed;
    }
    let started = std::time::Instant::now();
    let mut res = RunResult {
        ts,
        ..Default::default()
    };
    let mut shots = vec![];
    if t.is_browser() {
        let start = variables::render(
            o.start_url.as_deref().unwrap_or(&t.config.request.url),
            &vars,
        );
        let device = o
            .device_id
            .clone()
            .or_else(|| t.options.device_ids.first().cloned())
            .unwrap_or_else(|| "chrome.laptop_large".into());
        let b = browser::run(t, &start, &device, o.default_step_timeout, &mut vars).await;
        res.failure = b.failure;
        res.steps = b.steps;
        res.details = b.details;
        shots = b.screenshots;
    } else if t.subtype == "multi" {
        let (steps, failure) = multistep::run(&t.config.steps, &opts, &mut vars).await;
        res.failure = failure;
        res.steps = steps;
    } else {
        let mut req = t.config.request.clone();
        for (k, v) in &o.headers {
            req.headers.insert(k.clone(), v.clone());
        }
        if o.body.is_some() {
            req.body = o.body.clone();
        }
        if let Some(u) = &o.start_url
            && !req.url.is_empty()
        {
            req.url = u.clone();
        }
        let req: Request = serde_json::to_value(&req)
            .ok()
            .map(|v| variables::render_value(&v, &vars))
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or(req);
        let obs = net::run(&t.subtype, &req, &t.config.assertions, &opts).await;
        res.failure = obs.failure;
        res.status_code = obs.status_code;
        res.details = obs.details;
        res.duration_ms = obs.duration_ms;
    }
    if res.duration_ms == 0.0 {
        res.duration_ms = started.elapsed().as_secs_f64() * 1000.0;
    }
    res.passed = res.failure.is_none();
    (res, shots)
}

/// Runs a test (retrying as its options say) and records the result:
/// memory, PostgreSQL, a log, metrics, and an alert for scheduled runs.
pub async fn run_and_record(st: &AppState, t: &Test, o: Overrides) -> RunResult {
    let retry = o.retry.clone().or_else(|| t.options.retry.clone());
    let (count, interval) = retry
        .map(|r| (r.count.min(3), r.interval.unwrap_or(300).min(10_000)))
        .unwrap_or((0, 0));
    let (mut res, mut shots) = run_once(st, t, &o).await;
    let mut retries = 0;
    while !res.passed && retries < count {
        tokio::time::sleep(Duration::from_millis(interval)).await;
        retries += 1;
        (res, shots) = run_once(st, t, &o).await;
    }
    res.retries = retries;
    res.result_id = o.result_id.clone().unwrap_or_else(new_result_id);
    res.batch_id = o.batch_id.clone();
    let id = t.public_id.clone().unwrap_or_default();
    if o.batch_id.is_none()
        && let Ok(mut last) = LAST.lock()
    {
        last.insert(id.clone(), res.clone());
    }
    let row = SyntheticResult {
        result_id: res.result_id.clone(),
        public_id: id.clone(),
        ts: res.ts,
        passed: res.passed,
        batch_id: res.batch_id.clone(),
        body: serde_json::to_value(&res).unwrap_or_default(),
    };
    if let Err(e) = st.store.pg.put_synthetic_result(&row, &shots).await {
        tracing::warn!(error = %e, "storing a synthetic result");
    }
    record(st, t, &id, &res).await;
    if o.batch_id.is_none() {
        alert(st, t, &id, &res).await;
    }
    res
}

/// The run as a log and metrics.
async fn record(st: &AppState, t: &Test, id: &str, res: &RunResult) {
    let outcome = if res.passed { "passed" } else { "failed" };
    let subtype = if t.is_browser() {
        "browser"
    } else if t.subtype.is_empty() {
        "http"
    } else {
        &t.subtype
    };
    let mut tags = t.tags.clone();
    tags.push(format!("check_id:{id}"));
    tags.push(format!("test_type:{subtype}"));
    if res.batch_id.is_some() {
        tags.push("trigger:ci".into());
    }
    let mut run_tags = tags.clone();
    run_tags.push(format!("status:{outcome}"));
    st.aggregator.add(
        res.ts,
        "synthetics.test_runs",
        None,
        run_tags,
        Sample::Count(1.0),
    );
    let metric = if t.is_browser() {
        "synthetics.browser.test_duration".to_string()
    } else {
        format!("synthetics.{subtype}.response.time")
    };
    st.aggregator.add(
        res.ts,
        &metric,
        None,
        tags.clone(),
        Sample::Gauge(res.duration_ms),
    );
    if let Some(days) = res.details["certificate"]["days_left"].as_f64() {
        st.aggregator.add(
            res.ts,
            "synthetics.ssl.certificate.days_left",
            None,
            tags.clone(),
            Sample::Gauge(days),
        );
    }
    let mut attributes = Map::new();
    attributes.insert(
        "synthetics".into(),
        json!({"check_id": id, "test_name": t.name, "passed": res.passed, "failure": res.failure,
               "result_id": res.result_id, "type": subtype, "batch_id": res.batch_id,
               "steps": res.steps.len()}),
    );
    if subtype == "http" {
        attributes.insert(
            "http".into(),
            json!({"url": t.config.request.url, "method": t.config.request.method, "status_code": res.status_code}),
        );
    }
    attributes.insert("duration".into(), json!((res.duration_ms * 1e6) as i64));
    let message = match &res.failure {
        None => format!("{}: passed in {:.0} ms", t.name, res.duration_ms),
        Some(f) => format!("{}: failed, {f}", t.name),
    };
    let row = LogRow {
        ts: res.ts,
        status: if res.passed { "info" } else { "error" }.into(),
        service: Some("synthetics".into()),
        host: None,
        source: Some("synthetics".into()),
        message,
        tags,
        attributes,
        trace_id: None,
        span_id: None,
    };
    if let Err(e) = st.store.logs.send(vec![row]).await {
        tracing::warn!(error = %e, "storing a synthetic result log");
    }
}

/// Where a test stands between its runs: since when it fails, and whether
/// that was announced.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct AlertState {
    pub failing_since: Option<i64>,
    pub alerting: bool,
}

const ALERT_KIND: &str = "synthetic_alert";
static ALERTS: LazyLock<Mutex<HashMap<String, AlertState>>> = LazyLock::new(Default::default);

/// The state after a run, and what to announce: `Some(true)` an alert,
/// `Some(false)` a recovery.
pub fn next_alert(
    prev: AlertState,
    passed: bool,
    ts: i64,
    min_failure_ms: i64,
) -> (AlertState, Option<bool>) {
    if passed {
        return (AlertState::default(), prev.alerting.then_some(false));
    }
    let since = prev.failing_since.unwrap_or(ts);
    let due = !prev.alerting && ts - since >= min_failure_ms;
    (
        AlertState {
            failing_since: Some(since),
            alerting: prev.alerting || due,
        },
        due.then_some(true),
    )
}

/// A test alerts by itself, as Datadog's do: an event, and its message sent
/// to the `@webhook-…` and e-mail handles it names, when it starts failing
/// and when it recovers.
async fn alert(st: &AppState, t: &Test, id: &str, res: &RunResult) {
    let known = ALERTS.lock().ok().and_then(|a| a.get(id).copied());
    let prev = match known {
        Some(p) => p,
        // After a restart: what was announced is in the database.
        None => st
            .store
            .pg
            .doc(ALERT_KIND, id)
            .await
            .ok()
            .flatten()
            .and_then(|d| serde_json::from_value(d.body).ok())
            .unwrap_or_default(),
    };
    let min = t.options.min_failure_duration.unwrap_or(0).max(0) * 1000;
    let (next, news) = next_alert(prev, res.passed, res.ts, min);
    if let Ok(mut a) = ALERTS.lock() {
        a.insert(id.to_string(), next);
    }
    let Some(triggered) = news else { return };
    if let Ok(body) = serde_json::to_value(next)
        && let Err(e) = st.store.pg.put_doc(ALERT_KIND, id, &body).await
    {
        tracing::warn!(error = %e, "storing a synthetic alert state");
    }
    let title = if triggered {
        format!("[Triggered] Synthetic test failed: {}", t.name)
    } else {
        format!("[Recovered] Synthetic test passes again: {}", t.name)
    };
    let detail = match (&res.failure, triggered) {
        (Some(f), true) => format!("{}: {f}", t.target()),
        _ => t.target(),
    };
    let text = if t.message.trim().is_empty() {
        detail.clone()
    } else {
        format!("{detail}\n\n{}", t.message)
    };
    let link = format!(
        "{}/synthetics/tests?test={id}",
        st.cfg.base_url.trim_end_matches('/')
    );
    let mut tags = t.tags.clone();
    tags.push(format!("check_id:{id}"));
    let mut notified =
        crate::monitors::webhooks::notify_plain(st, id, &title, &text, !triggered, &link, &tags)
            .await;
    notified.extend(crate::monitors::email::notify(st, &title, &text).await);
    tracing::info!(test = %t.name, triggered, ?notified, "synthetic test alert");
    let mut attributes = Map::new();
    attributes.insert("evt".into(), json!({"title": title, "text": text}));
    attributes.insert(
        "synthetics".into(),
        json!({"check_id": id, "test_name": t.name, "notified": notified, "result_id": res.result_id}),
    );
    crate::events::emit(
        st,
        vec![LogRow {
            ts: res.ts,
            status: if triggered { "error" } else { "ok" }.into(),
            service: Some("synthetics".into()),
            host: None,
            source: Some("synthetics".into()),
            message: format!("{title}\n{text}"),
            tags,
            attributes,
            trace_id: None,
            span_id: None,
        }],
    )
    .await;
}

/// The monitor id a test shows under in Manage Monitors: stable, above the
/// ids of stored monitors.
pub fn monitor_id(public_id: &str) -> i64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in public_id.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    SYNTHETIC_MONITOR_BASE + (h % 1_000_000_000) as i64
}

pub const SYNTHETIC_MONITOR_BASE: i64 = 2_000_000_000;

/// Each test as the `synthetics alert` monitor Datadog creates with it.
pub async fn as_monitors(st: &AppState) -> anyhow::Result<Vec<Value>> {
    let tests = stored(st).await?;
    Ok(tests
        .into_iter()
        .filter_map(|t| {
            let pid = t.public_id.clone()?;
            let last = last_result(&pid);
            let status = match (alerting(&pid), &last) {
                (Some(true), _) => "Alert",
                (_, Some(_)) => "OK",
                _ => "No Data",
            };
            let ts = last.as_ref().map(|r| r.ts / 1000);
            Some(json!({
                "id": monitor_id(&pid),
                "name": format!("[Synthetics] {}", t.name),
                "type": "synthetics alert",
                "query": format!("\"synthetics.check_status\".over(\"public_id:{pid}\").last(1).count_by_status()"),
                "message": t.message,
                "tags": t.tags,
                "priority": null,
                "options": {"thresholds": {}, "notify_no_data": false, "silenced": {}},
                "overall_state": status,
                "overall_state_modified": null,
                "multi": false,
                "state": {"groups": {"*": {"name": "*", "status": status, "value": null, "last_data_ts": ts, "since_ts": ts}}},
                "synthetics_test_id": pid,
                "deleted": null,
                "org_id": 1,
            }))
        })
        .collect())
}

/// Whether a test is alerting, if it ran since this server started.
pub fn alerting(public_id: &str) -> Option<bool> {
    ALERTS
        .lock()
        .ok()
        .and_then(|a| a.get(public_id).map(|s| s.alerting))
}

pub async fn stored(st: &AppState) -> anyhow::Result<Vec<Test>> {
    Ok(st
        .store
        .pg
        .docs(KIND)
        .await?
        .into_iter()
        .filter_map(|d| serde_json::from_value::<Test>(d.body).ok())
        .collect())
}

pub async fn load(st: &AppState, id: &str) -> Result<Test, ApiError> {
    let doc = st.store.pg.doc(KIND, id).await?.ok_or(ApiError::NotFound)?;
    Ok(serde_json::from_value(doc.body).map_err(anyhow::Error::from)?)
}

fn tick_ms(t: &Test) -> i64 {
    let min = if t.is_browser() {
        MIN_BROWSER_TICK_S
    } else {
        MIN_TICK_S
    };
    t.options.tick_every.unwrap_or(min.max(60)).max(min) * 1000
}

/// Runs the live tests that are due, every ten seconds; purges old results
/// every hour.
pub fn spawn(st: AppState) {
    tokio::spawn(async move {
        // The latest results survive restarts: no burst of runs at start.
        if let Ok(latest) = st.store.pg.latest_synthetic_results().await
            && let Ok(mut last) = LAST.lock()
        {
            for r in latest {
                if let Ok(res) = serde_json::from_value::<RunResult>(r.body) {
                    last.insert(r.public_id, res);
                }
            }
        }
        let running: std::sync::Arc<Mutex<std::collections::HashSet<String>>> = Default::default();
        let mut tick = tokio::time::interval(Duration::from_secs(10));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut purged = 0;
        loop {
            tick.tick().await;
            let now = now_ms();
            if now - purged > 3_600_000 {
                purged = now;
                let before = now - st.cfg.retention_days * 86_400_000;
                match st
                    .store
                    .pg
                    .purge_synthetic_results(before, KEEP_SCREENSHOTS)
                    .await
                {
                    Ok(n) if n > 0 => tracing::info!(n, "purged synthetic results"),
                    Ok(_) => {}
                    Err(e) => tracing::warn!(error = %e, "purging synthetic results"),
                }
            }
            let Ok(tests) = stored(&st).await else {
                continue;
            };
            for t in tests.into_iter().filter(|t| t.status != "paused") {
                let id = t.public_id.clone().unwrap_or_default();
                let due = LAST
                    .lock()
                    .map(|l| l.get(&id).is_none_or(|r| now - r.ts >= tick_ms(&t) - 1000))
                    .unwrap_or(true);
                // A slow run (a browser, retries) is not started twice.
                let free = running
                    .lock()
                    .map(|mut r| r.insert(id.clone()))
                    .unwrap_or(false);
                if !due || !free {
                    if free && let Ok(mut r) = running.lock() {
                        r.remove(&id);
                    }
                    continue;
                }
                let st = st.clone();
                let running = running.clone();
                tokio::spawn(async move {
                    run_and_record(&st, &t, Overrides::default()).await;
                    if let Ok(mut r) = running.lock() {
                        r.remove(&id);
                    }
                });
            }
        }
    });
}

/// `abc-def-ghi`, like Datadog's public ids.
fn public_id() -> String {
    const CHARS: &[u8] = b"abcdefghijkmnpqrstuvwxyz23456789";
    let part = || -> String {
        (0..3)
            .map(|_| CHARS[rand::random_range(0..CHARS.len())] as char)
            .collect()
    };
    format!("{}-{}-{}", part(), part(), part())
}

fn bad(why: impl Into<String>) -> ApiError {
    ApiError::BadQuery(why.into())
}

/// The request a protocol needs.
fn check_request(subtype: &str, r: &mut Request, what: &str) -> Result<(), ApiError> {
    r.url = r.url.trim().to_string();
    r.host = r.host.trim().to_string();
    match subtype {
        "http" => {
            if !(r.url.starts_with("http://")
                || r.url.starts_with("https://")
                || r.url.starts_with("{{"))
            {
                return Err(bad(format!(
                    "{what}: the URL must start with http:// or https://"
                )));
            }
            if r.method.is_empty() {
                r.method = "GET".into();
            }
        }
        "websocket" => {
            if !(r.url.starts_with("ws://")
                || r.url.starts_with("wss://")
                || r.url.starts_with("{{"))
            {
                return Err(bad(format!(
                    "{what}: the URL must start with ws:// or wss://"
                )));
            }
        }
        "ssl" | "tcp" | "dns" | "udp" | "grpc" | "icmp" => {
            if r.host.is_empty() {
                return Err(bad(format!("{what}: a host is needed")));
            }
            if matches!(subtype, "tcp" | "udp") && r.port.is_none() {
                return Err(bad(format!("{what}: a port is needed")));
            }
        }
        "wait" => {}
        other => return Err(bad(format!("{what}: unknown subtype {other}"))),
    }
    Ok(())
}

fn check(mut t: Test) -> Result<Test, ApiError> {
    t.name = t.name.trim().to_string();
    if t.name.is_empty() {
        return Err(bad("a test needs a name"));
    }
    if t.typ == "browser" {
        t.subtype.clear();
        let url = t.config.request.url.trim().to_string();
        if !(url.starts_with("http://") || url.starts_with("https://") || url.starts_with("{{")) {
            return Err(bad("the start URL must start with http:// or https://"));
        }
        t.config.request.url = url;
        if t.config.request.method.is_empty() {
            t.config.request.method = "GET".into();
        }
        for (i, s) in t.steps.iter().enumerate() {
            if !browser::STEP_TYPES.contains(&s.typ.as_str()) {
                return Err(bad(format!(
                    "step {}: the step type {} is not supported",
                    i + 1,
                    s.typ
                )));
            }
        }
        if t.options.device_ids.is_empty() {
            t.options.device_ids = vec!["chrome.laptop_large".into()];
        }
    } else {
        t.typ = "api".into();
        if t.subtype.is_empty() {
            t.subtype = "http".into();
        }
        if !API_SUBTYPES.contains(&t.subtype.as_str()) {
            return Err(bad(format!("unknown subtype {}", t.subtype)));
        }
        if t.subtype == "multi" {
            if t.config.steps.is_empty() {
                return Err(bad("a multistep test needs steps"));
            }
            for (i, s) in t.config.steps.iter_mut().enumerate() {
                if s.subtype.is_empty() {
                    s.subtype = "http".into();
                }
                let sub = s.subtype.clone();
                check_request(&sub, &mut s.request, &format!("step {}", i + 1))?;
            }
        } else {
            let sub = t.subtype.clone();
            check_request(&sub, &mut t.config.request, "the request")?;
        }
    }
    for v in t.config.config_variables.iter().chain(&t.config.variables) {
        if !v
            .name
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
            || v.name.is_empty()
        {
            return Err(bad(format!(
                "variable {:?}: names are in capitals, digits and underscores",
                v.name
            )));
        }
    }
    if t.status != "paused" {
        t.status = "live".into();
    }
    if t.locations.is_empty() {
        t.locations = vec![ci::LOCATION.into()];
    }
    Ok(t)
}

/// The latest run of a test, if it ran since this server started.
pub fn last_result(public_id: &str) -> Option<RunResult> {
    LAST.lock().ok().and_then(|l| l.get(public_id).cloned())
}

fn with_last(mut t: Test) -> Test {
    let id = t.public_id.clone().unwrap_or_default();
    t.last_result = last_result(&id);
    t.monitor_status = alerting(&id).map(|a| if a { "Alert" } else { "OK" }.to_string());
    t
}

async fn save(st: &AppState, t: &Test) -> Result<(), ApiError> {
    let id = t.public_id.clone().unwrap_or_default();
    st.store
        .pg
        .put_doc(
            KIND,
            &id,
            &serde_json::to_value(t).map_err(anyhow::Error::from)?,
        )
        .await?;
    Ok(())
}

pub async fn list(State(st): State<AppState>) -> ApiResult<Value> {
    let mut tests: Vec<Test> = stored(&st).await?.into_iter().map(with_last).collect();
    tests.sort_by_key(|t| t.name.to_lowercase());
    Ok(Json(json!({"tests": tests})))
}

pub async fn get(State(st): State<AppState>, Path(id): Path<String>) -> ApiResult<Test> {
    Ok(Json(with_last(load(&st, &id).await?)))
}

/// `/api/v1/synthetics/tests/{api|browser}/{id}`.
pub async fn get_typed(
    State(st): State<AppState>,
    Path((typ, id)): Path<(String, String)>,
) -> ApiResult<Test> {
    let t = load(&st, &id).await?;
    if t.typ != typ {
        return Err(ApiError::NotFound);
    }
    Ok(Json(with_last(t)))
}

async fn create_test(st: &AppState, t: Test) -> ApiResult<Test> {
    let mut t = check(t)?;
    t.public_id = Some(public_id());
    save(st, &t).await?;
    // The first run: at once for API tests, in the background for browser
    // ones, which take a while.
    if t.is_browser() {
        let (st, t2) = (st.clone(), t.clone());
        tokio::spawn(async move {
            run_and_record(&st, &t2, Overrides::default()).await;
        });
    } else {
        t.last_result = Some(run_and_record(st, &t, Overrides::default()).await);
    }
    Ok(Json(t))
}

pub async fn create(State(st): State<AppState>, Json(t): Json<Test>) -> ApiResult<Test> {
    create_test(&st, t).await
}

/// `/api/v1/synthetics/tests/browser`: the type is in the path.
pub async fn create_browser(
    State(st): State<AppState>,
    Json(mut t): Json<Test>,
) -> ApiResult<Test> {
    t.typ = "browser".into();
    create_test(&st, t).await
}

pub async fn create_api(State(st): State<AppState>, Json(mut t): Json<Test>) -> ApiResult<Test> {
    t.typ = "api".into();
    create_test(&st, t).await
}

pub async fn update(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Json(t): Json<Test>,
) -> ApiResult<Test> {
    let old = load(&st, &id).await?;
    let mut t = t;
    if t.typ.is_empty() {
        t.typ = old.typ;
    }
    let mut t = check(t)?;
    t.public_id = Some(id);
    save(&st, &t).await?;
    Ok(Json(with_last(t)))
}

pub async fn update_typed(
    State(st): State<AppState>,
    Path((typ, id)): Path<(String, String)>,
    Json(mut t): Json<Test>,
) -> ApiResult<Test> {
    t.typ = typ;
    update(State(st), Path(id), Json(t)).await
}

async fn remove(st: &AppState, id: &str) -> Result<bool, ApiError> {
    if !st.store.pg.delete_doc(KIND, id).await? {
        return Ok(false);
    }
    if let Ok(mut l) = LAST.lock() {
        l.remove(id);
    }
    st.store.pg.delete_synthetic_results(id).await?;
    Ok(true)
}

pub async fn delete(State(st): State<AppState>, Path(id): Path<String>) -> ApiResult<Value> {
    if !remove(&st, &id).await? {
        return Err(ApiError::NotFound);
    }
    Ok(Json(json!({})))
}

#[derive(Deserialize)]
pub struct DeleteMany {
    public_ids: Vec<String>,
}

/// `POST /api/v1/synthetics/tests/delete`.
pub async fn delete_many(
    State(st): State<AppState>,
    Json(b): Json<DeleteMany>,
) -> ApiResult<Value> {
    let mut deleted = Vec::new();
    for id in b.public_ids {
        if remove(&st, &id).await? {
            deleted.push(json!({"public_id": id, "deleted_at": chrono::Utc::now().to_rfc3339()}));
        }
    }
    Ok(Json(json!({"deleted_tests": deleted})))
}

#[derive(Deserialize)]
pub struct NewStatus {
    new_status: String,
}

/// `PUT /api/v1/synthetics/tests/{id}/status`: `live` or `paused`.
pub async fn set_status(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Json(b): Json<NewStatus>,
) -> ApiResult<bool> {
    let mut t = load(&st, &id).await?;
    if b.new_status != "live" && b.new_status != "paused" {
        return Err(bad("new_status is live or paused"));
    }
    t.status = b.new_status;
    save(&st, &t).await?;
    Ok(Json(true))
}

/// Runs the test now.
pub async fn trigger(State(st): State<AppState>, Path(id): Path<String>) -> ApiResult<RunResult> {
    let t = load(&st, &id).await?;
    Ok(Json(run_and_record(&st, &t, Overrides::default()).await))
}

#[derive(Deserialize)]
pub struct Range {
    #[serde(default)]
    from_ts: Option<i64>,
    #[serde(default)]
    to_ts: Option<i64>,
    #[serde(default)]
    limit: Option<i64>,
}

/// A test's runs, latest first (UI).
pub async fn results(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Query(r): Query<Range>,
) -> ApiResult<Value> {
    let to = r.to_ts.unwrap_or_else(now_ms);
    let from = r.from_ts.unwrap_or(to - 86_400_000);
    let rows = st
        .store
        .pg
        .synthetic_results(&id, from, to, r.limit.unwrap_or(200).clamp(1, 1000))
        .await?;
    Ok(Json(json!({
        "results": rows.into_iter().map(|r| r.body).collect::<Vec<_>>(),
    })))
}

/// One run (UI).
pub async fn result(State(st): State<AppState>, Path(result_id): Path<String>) -> ApiResult<Value> {
    let r = st
        .store
        .pg
        .synthetic_results_by_id(&[result_id])
        .await?
        .pop()
        .ok_or(ApiError::NotFound)?;
    let mut body = r.body;
    body["public_id"] = json!(r.public_id);
    Ok(Json(body))
}

pub async fn screenshot(
    State(st): State<AppState>,
    Path((result_id, step)): Path<(String, i32)>,
) -> Response {
    match st.store.pg.synthetic_screenshot(&result_id, step).await {
        Ok(Some(img)) => (
            [
                (header::CONTENT_TYPE, "image/jpeg"),
                (header::CACHE_CONTROL, "private, max-age=86400"),
            ],
            img,
        )
            .into_response(),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => {
            tracing::warn!(error = %e, "reading a screenshot");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

#[derive(Deserialize)]
pub struct Search {
    #[serde(default)]
    text: String,
    #[serde(default)]
    count: Option<usize>,
}

/// `GET /api/v1/synthetics/tests/search?text=`: words in the name, or
/// `tag:`, `type:`, `id:` filters.
pub async fn search(State(st): State<AppState>, Query(q): Query<Search>) -> ApiResult<Value> {
    let words: Vec<String> = q
        .text
        .split_whitespace()
        .map(|w| w.to_lowercase())
        .collect();
    let mut tests: Vec<Test> = stored(&st)
        .await?
        .into_iter()
        .filter(|t| {
            words.iter().all(|w| {
                if let Some(tag) = w.strip_prefix("tag:") {
                    t.tags.iter().any(|x| x.to_lowercase() == tag)
                } else if let Some(typ) = w.strip_prefix("type:") {
                    t.typ == typ || t.subtype == typ
                } else if let Some(id) = w.strip_prefix("id:") {
                    t.public_id.as_deref() == Some(id)
                } else {
                    t.name.to_lowercase().contains(w.as_str())
                }
            })
        })
        .collect();
    tests.sort_by_key(|t| t.name.to_lowercase());
    tests.truncate(q.count.unwrap_or(100).clamp(1, 1000));
    Ok(Json(json!({"tests": tests})))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alerts_once_and_recovers() {
        let s0 = AlertState::default();
        // At once without a minimum duration, and only once.
        let (s1, n1) = next_alert(s0, false, 1_000, 0);
        assert_eq!((s1.alerting, n1), (true, Some(true)));
        let (s2, n2) = next_alert(s1, false, 2_000, 0);
        assert_eq!((s2.alerting, n2), (true, None));
        let (s3, n3) = next_alert(s2, true, 3_000, 0);
        assert_eq!((s3, n3), (AlertState::default(), Some(false)));
        // A blip shorter than the minimum says nothing, either way.
        let (b1, m1) = next_alert(s0, false, 1_000, 120_000);
        assert_eq!((b1.alerting, m1), (false, None));
        assert_eq!(
            next_alert(b1, true, 61_000, 120_000),
            (AlertState::default(), None)
        );
        let (b2, _) = next_alert(b1, false, 61_000, 120_000);
        assert_eq!(next_alert(b2, false, 121_000, 120_000).1, Some(true));
    }

    #[test]
    fn checks_tests() {
        let t = |v: Value| check(serde_json::from_value(v).unwrap());
        let ok = t(json!({"name": "home", "config": {"request": {"url": "https://x"}}})).unwrap();
        assert_eq!((ok.typ.as_str(), ok.subtype.as_str()), ("api", "http"));
        assert_eq!(ok.config.request.method, "GET");
        assert!(
            t(json!({"name": "x", "subtype": "tcp", "config": {"request": {"host": "db"}}}))
                .is_err()
        );
        assert!(t(json!({"name": "x", "subtype": "tcp", "config": {"request": {"host": "db", "port": 5432}}})).is_ok());
        assert!(t(json!({"name": "x", "subtype": "websocket", "config": {"request": {"url": "https://x"}}})).is_err());
        assert!(t(json!({"name": "x", "subtype": "multi", "config": {}})).is_err());
        let multi = t(json!({"name": "x", "subtype": "multi", "config": {"steps": [
            {"name": "a", "request": {"url": "https://x"}}, {"name": "w", "subtype": "wait", "value": 1}]}}))
        .unwrap();
        assert_eq!(multi.config.steps[0].subtype, "http");
        let b = t(
            json!({"name": "x", "type": "browser", "config": {"request": {"url": "https://x"}},
            "steps": [{"type": "click", "params": {}}]}),
        )
        .unwrap();
        assert_eq!(b.options.device_ids, vec!["chrome.laptop_large"]);
        assert!(
            t(
                json!({"name": "x", "type": "browser", "config": {"request": {"url": "https://x"}},
            "steps": [{"type": "uploadFiles"}]})
            )
            .is_err()
        );
        assert!(
            t(
                json!({"name": "x", "config": {"request": {"url": "https://x"},
            "configVariables": [{"name": "lower", "type": "text", "pattern": "x"}]}})
            )
            .is_err()
        );
        let id = public_id();
        assert_eq!(id.len(), 11);
        // Unknown fields survive a round trip.
        let rt: Test = serde_json::from_value(
            json!({"name": "x", "config": {"request": {"url": "u", "proxy": {"url": "p"}}},
            "options": {"monitor_priority": 2}}),
        )
        .unwrap();
        let back = serde_json::to_value(&rt).unwrap();
        assert_eq!(back["config"]["request"]["proxy"]["url"], "p");
        assert_eq!(back["options"]["monitor_priority"], 2);
    }
}
