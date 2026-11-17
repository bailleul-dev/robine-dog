//! Synthetic API tests: HTTP requests sent on a schedule from robine-dog,
//! with assertions on the response. Tests have the shape of Datadog's
//! Synthetics API (`config.request`, `config.assertions`,
//! `options.tick_every`).
//!
//! Each run is a log (`source:synthetics`, `@synthetics.check_id`) and two
//! metrics, as in Datadog: `synthetics.http.response.time` (milliseconds)
//! and `synthetics.test_runs` (tagged `status:passed|failed`), all tagged
//! `check_id` and with the test's tags. Alerting is a monitor on those.

use crate::AppState;
use crate::api::{ApiError, ApiResult};
use crate::intake::logs::LogRow;
use crate::metrics::Sample;
use axum::Json;
use axum::extract::{Path, State};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

const KIND: &str = "synthetic";
const MIN_TICK_S: i64 = 30;
const MAX_BODY: usize = 256 * 1024;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Request {
    #[serde(default)]
    pub method: String,
    pub url: String,
    #[serde(default)]
    pub headers: Map<String, Value>,
    #[serde(default)]
    pub body: Option<String>,
    /// Seconds.
    #[serde(default)]
    pub timeout: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Assertion {
    /// `statusCode`, `responseTime` (milliseconds), `body` or `header`.
    #[serde(rename = "type")]
    pub typ: String,
    /// `is`, `isNot`, `lessThan`, `moreThan`, `contains`, `doesNotContain`, `matches`.
    pub operator: String,
    /// The header's name.
    #[serde(default)]
    pub property: Option<String>,
    pub target: Value,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    pub request: Request,
    #[serde(default)]
    pub assertions: Vec<Assertion>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TestOptions {
    /// Seconds between two runs.
    #[serde(default)]
    pub tick_every: Option<i64>,
    /// Seconds a test must have been failing before it alerts (0: at once).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_failure_duration: Option<i64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Test {
    #[serde(default)]
    pub public_id: Option<String>,
    pub name: String,
    #[serde(rename = "type", default)]
    pub typ: String,
    #[serde(default)]
    pub subtype: String,
    /// `live` or `paused`.
    #[serde(default)]
    pub status: String,
    pub config: Config,
    #[serde(default)]
    pub options: TestOptions,
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub tags: Vec<String>,
    /// Whether it is alerting; computed.
    #[serde(default, skip_deserializing, skip_serializing_if = "Option::is_none")]
    pub monitor_status: Option<String>,
    /// The latest run; computed.
    #[serde(default, skip_deserializing, skip_serializing_if = "Option::is_none")]
    pub last_result: Option<RunResult>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RunResult {
    /// Epoch milliseconds.
    pub ts: i64,
    pub passed: bool,
    pub status_code: Option<u16>,
    pub duration_ms: f64,
    pub failure: Option<String>,
}

static CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .user_agent("robine-dog-synthetics")
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
        .expect("http client")
});
static LAST: LazyLock<Mutex<HashMap<String, RunResult>>> = LazyLock::new(Default::default);

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn as_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// Why an assertion does not hold, or `None`.
fn check_text(a: &Assertion, what: &str, actual: Option<&str>) -> Option<String> {
    let target = as_text(&a.target);
    let Some(actual) = actual else {
        return Some(format!("{what} is missing"));
    };
    let ok = match a.operator.as_str() {
        "is" => actual == target,
        "isNot" => actual != target,
        "contains" => actual.contains(&target),
        "doesNotContain" => !actual.contains(&target),
        "matches" => regex::Regex::new(&target).is_ok_and(|r| r.is_match(actual)),
        "doesNotMatch" => regex::Regex::new(&target).is_ok_and(|r| !r.is_match(actual)),
        other => return Some(format!("unknown operator {other}")),
    };
    (!ok).then(|| format!("{what} should {} {target:?}", a.operator))
}

fn check_number(a: &Assertion, what: &str, actual: f64) -> Option<String> {
    let target = a
        .target
        .as_f64()
        .or_else(|| a.target.as_str().and_then(|s| s.trim().parse().ok()));
    let Some(target) = target else {
        return Some(format!("{what}: the target is not a number"));
    };
    let ok = match a.operator.as_str() {
        "is" => actual == target,
        "isNot" => actual != target,
        "lessThan" => actual < target,
        "lessThanOrEqual" => actual <= target,
        "moreThan" => actual > target,
        "moreThanOrEqual" => actual >= target,
        other => return Some(format!("unknown operator {other}")),
    };
    (!ok).then(|| format!("{what} is {actual}, should be {} {target}", a.operator))
}

/// The first assertion the response fails.
pub fn first_failure(
    assertions: &[Assertion],
    status: u16,
    duration_ms: f64,
    headers: &HashMap<String, String>,
    body: &str,
) -> Option<String> {
    assertions.iter().find_map(|a| match a.typ.as_str() {
        "statusCode" => check_number(a, "status code", status as f64),
        "responseTime" => check_number(a, "response time (ms)", duration_ms.round()),
        "body" => check_text(a, "body", Some(body)),
        "header" => {
            let name = a.property.clone().unwrap_or_default().to_lowercase();
            check_text(
                a,
                &format!("header {name}"),
                headers.get(&name).map(String::as_str),
            )
        }
        other => Some(format!("unknown assertion {other}")),
    })
}

pub async fn run(t: &Test) -> RunResult {
    let r = &t.config.request;
    let method = reqwest::Method::from_bytes(
        if r.method.is_empty() {
            "GET"
        } else {
            &r.method
        }
        .to_uppercase()
        .as_bytes(),
    )
    .unwrap_or(reqwest::Method::GET);
    let timeout = Duration::from_secs_f64(r.timeout.unwrap_or(10.0).clamp(1.0, 60.0));
    let mut req = CLIENT.request(method, &r.url).timeout(timeout);
    for (k, v) in &r.headers {
        req = req.header(k, as_text(v));
    }
    if let Some(b) = r.body.as_ref().filter(|b| !b.is_empty()) {
        req = req.body(b.clone());
    }
    let started = Instant::now();
    let ts = now_ms();
    let fail = |started: Instant, status, failure: String| RunResult {
        ts,
        passed: false,
        status_code: status,
        duration_ms: started.elapsed().as_secs_f64() * 1000.0,
        failure: Some(failure),
    };
    let resp = match req.send().await {
        Ok(r) => r,
        Err(e) if e.is_timeout() => {
            return fail(started, None, "TIMEOUT: no response in time".into());
        }
        Err(e) => {
            let mut why = e.to_string();
            let mut source = std::error::Error::source(&e);
            while let Some(s) = source {
                why = s.to_string();
                source = s.source();
            }
            return fail(started, None, format!("CONNECTION: {why}"));
        }
    };
    let status = resp.status().as_u16();
    let headers: HashMap<String, String> = resp
        .headers()
        .iter()
        .filter_map(|(k, v)| Some((k.as_str().to_lowercase(), v.to_str().ok()?.to_string())))
        .collect();
    let body = match resp.bytes().await {
        Ok(b) => String::from_utf8_lossy(&b[..b.len().min(MAX_BODY)]).into_owned(),
        Err(e) => return fail(started, Some(status), format!("BODY: {e}")),
    };
    let duration_ms = started.elapsed().as_secs_f64() * 1000.0;
    let failure = first_failure(&t.config.assertions, status, duration_ms, &headers, &body);
    RunResult {
        ts,
        passed: failure.is_none(),
        status_code: Some(status),
        duration_ms,
        failure,
    }
}

/// Runs a test and records the result: memory, a log, metrics.
pub async fn run_and_record(st: &AppState, t: &Test) -> RunResult {
    let res = run(t).await;
    let id = t.public_id.clone().unwrap_or_default();
    if let Ok(mut last) = LAST.lock() {
        last.insert(id.clone(), res.clone());
    }
    let outcome = if res.passed { "passed" } else { "failed" };
    let mut tags = t.tags.clone();
    tags.push(format!("check_id:{id}"));
    let mut run_tags = tags.clone();
    run_tags.push(format!("status:{outcome}"));
    st.aggregator.add(
        res.ts,
        "synthetics.test_runs",
        None,
        run_tags,
        Sample::Count(1.0),
    );
    st.aggregator.add(
        res.ts,
        "synthetics.http.response.time",
        None,
        tags.clone(),
        Sample::Gauge(res.duration_ms),
    );
    let mut attributes = Map::new();
    attributes.insert(
        "synthetics".into(),
        json!({"check_id": id, "test_name": t.name, "passed": res.passed, "failure": res.failure}),
    );
    attributes.insert(
        "http".into(),
        json!({"url": t.config.request.url, "method": t.config.request.method, "status_code": res.status_code}),
    );
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
        tracing::warn!(error = %e, "storing a synthetic result");
    }
    alert(st, t, &id, &res).await;
    res
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
/// to the `@webhook-…` handles it names, when it starts failing and when
/// it recovers.
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
        (Some(f), true) => format!("{} {}: {f}", t.config.request.method, t.config.request.url),
        _ => format!("{} {}", t.config.request.method, t.config.request.url),
    };
    let text = if t.message.trim().is_empty() {
        detail.clone()
    } else {
        format!("{detail}\n\n{}", t.message)
    };
    let link = format!("{}/synthetics/tests", st.cfg.base_url.trim_end_matches('/'));
    let mut tags = t.tags.clone();
    tags.push(format!("check_id:{id}"));
    let notified =
        crate::monitors::webhooks::notify_plain(st, id, &title, &text, !triggered, &link, &tags)
            .await;
    tracing::info!(test = %t.name, triggered, ?notified, "synthetic test alert");
    let mut attributes = Map::new();
    attributes.insert("evt".into(), json!({"title": title, "text": text}));
    attributes.insert(
        "synthetics".into(),
        json!({"check_id": id, "test_name": t.name, "notified": notified}),
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

/// Whether a test is alerting, if it ran since this server started.
pub fn alerting(public_id: &str) -> Option<bool> {
    ALERTS
        .lock()
        .ok()
        .and_then(|a| a.get(public_id).map(|s| s.alerting))
}

async fn stored(st: &AppState) -> anyhow::Result<Vec<Test>> {
    Ok(st
        .store
        .pg
        .docs(KIND)
        .await?
        .into_iter()
        .filter_map(|d| serde_json::from_value::<Test>(d.body).ok())
        .collect())
}

/// Runs the live tests that are due, every ten seconds.
pub fn spawn(st: AppState) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(10));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tick.tick().await;
            let Ok(tests) = stored(&st).await else {
                continue;
            };
            let now = now_ms();
            for t in tests.into_iter().filter(|t| t.status != "paused") {
                let every = t.options.tick_every.unwrap_or(60).max(MIN_TICK_S) * 1000;
                let id = t.public_id.clone().unwrap_or_default();
                let due = LAST
                    .lock()
                    .map(|l| l.get(&id).is_none_or(|r| now - r.ts >= every - 1000))
                    .unwrap_or(true);
                if due {
                    let st = st.clone();
                    tokio::spawn(async move {
                        run_and_record(&st, &t).await;
                    });
                }
            }
        }
    });
}

/// `abc-def-ghi`, like Datadog's public ids.
fn public_id() -> String {
    let raw = crate::store::pg::new_id().to_lowercase();
    let letters: Vec<char> = raw.chars().filter(char::is_ascii_alphanumeric).collect();
    letters[..9.min(letters.len())]
        .chunks(3)
        .map(|c| c.iter().collect::<String>())
        .collect::<Vec<_>>()
        .join("-")
}

fn check(mut t: Test) -> Result<Test, ApiError> {
    t.name = t.name.trim().to_string();
    let url = t.config.request.url.trim().to_string();
    if t.name.is_empty() {
        return Err(ApiError::BadQuery("a test needs a name".into()));
    }
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err(ApiError::BadQuery(
            "the URL must start with http:// or https://".into(),
        ));
    }
    t.config.request.url = url;
    if t.config.request.method.is_empty() {
        t.config.request.method = "GET".into();
    }
    t.typ = "api".into();
    t.subtype = "http".into();
    if t.status != "paused" {
        t.status = "live".into();
    }
    Ok(t)
}

/// The latest run of a test, if it ran since this server started.
pub fn last_result(public_id: &str) -> Option<RunResult> {
    LAST.lock().ok().and_then(|l| l.get(public_id).cloned())
}

fn with_last(mut t: Test) -> Test {
    let id = t.public_id.clone().unwrap_or_default();
    t.last_result = LAST.lock().ok().and_then(|l| l.get(&id).cloned());
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
    let doc = st
        .store
        .pg
        .doc(KIND, &id)
        .await?
        .ok_or(ApiError::NotFound)?;
    let t: Test = serde_json::from_value(doc.body).map_err(anyhow::Error::from)?;
    Ok(Json(with_last(t)))
}

pub async fn create(State(st): State<AppState>, Json(t): Json<Test>) -> ApiResult<Test> {
    let mut t = check(t)?;
    t.public_id = Some(public_id());
    save(&st, &t).await?;
    let res = run_and_record(&st, &t).await;
    t.last_result = Some(res);
    Ok(Json(t))
}

pub async fn update(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Json(t): Json<Test>,
) -> ApiResult<Test> {
    st.store
        .pg
        .doc(KIND, &id)
        .await?
        .ok_or(ApiError::NotFound)?;
    let mut t = check(t)?;
    t.public_id = Some(id);
    save(&st, &t).await?;
    Ok(Json(with_last(t)))
}

pub async fn delete(State(st): State<AppState>, Path(id): Path<String>) -> ApiResult<Value> {
    if !st.store.pg.delete_doc(KIND, &id).await? {
        return Err(ApiError::NotFound);
    }
    if let Ok(mut l) = LAST.lock() {
        l.remove(&id);
    }
    Ok(Json(json!({})))
}

/// Runs the test now.
pub async fn trigger(State(st): State<AppState>, Path(id): Path<String>) -> ApiResult<RunResult> {
    let doc = st
        .store
        .pg
        .doc(KIND, &id)
        .await?
        .ok_or(ApiError::NotFound)?;
    let t: Test = serde_json::from_value(doc.body).map_err(anyhow::Error::from)?;
    Ok(Json(run_and_record(&st, &t).await))
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
    fn assertions() {
        let a: Vec<Assertion> = serde_json::from_value(json!([
            {"type": "statusCode", "operator": "is", "target": 200},
            {"type": "responseTime", "operator": "lessThan", "target": 500},
            {"type": "body", "operator": "contains", "target": "ok"},
            {"type": "header", "property": "Content-Type", "operator": "matches", "target": "^application/json"}
        ]))
        .unwrap();
        let h = HashMap::from([(
            "content-type".to_string(),
            "application/json; charset=utf-8".to_string(),
        )]);
        assert_eq!(
            first_failure(&a, 200, 120.0, &h, r#"{"status":"ok"}"#),
            None
        );
        assert_eq!(
            first_failure(&a, 503, 120.0, &h, "").as_deref(),
            Some("status code is 503, should be is 200")
        );
        assert!(
            first_failure(&a, 200, 900.0, &h, "ok")
                .unwrap()
                .starts_with("response time")
        );
        assert!(
            first_failure(&a, 200, 10.0, &h, "down")
                .unwrap()
                .starts_with("body")
        );
        assert!(
            first_failure(&a, 200, 10.0, &HashMap::new(), "ok")
                .unwrap()
                .contains("missing")
        );
        let id = public_id();
        assert_eq!(id.len(), 11);
    }
}
