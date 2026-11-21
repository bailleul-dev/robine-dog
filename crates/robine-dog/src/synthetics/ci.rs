//! Tests run from CI, with the API `datadog-ci synthetics run-tests` uses:
//! a trigger makes a batch of runs (one per test and device), the batch is
//! polled until every run is done, then the runs' results are fetched.
//! Point `datadog-ci` here with `DD_API_HOST_OVERRIDE=https://robine-dog…`.

use super::{Overrides, Retry, RunResult, Test, browser, load, new_result_id, now_ms};
use crate::AppState;
use crate::api::{ApiError, ApiResult};
use axum::Json;
use axum::extract::{Path, Query, State};
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::collections::HashMap;

/// The only place tests run from.
pub const LOCATION: &str = "robine-dog";
const BATCH_KIND: &str = "synthetics_batch";
const DEFAULT_TIMEOUT_MS: i64 = 30 * 60_000;

pub fn location() -> Value {
    json!({"id": 1, "name": LOCATION, "display_name": "robine-dog", "region": "self-hosted", "is_active": true})
}

/// `GET /api/v1/synthetics/locations`.
pub async fn locations() -> Json<Value> {
    Json(json!({"locations": [location()]}))
}

/// `GET /api/v1/synthetics/settings`.
pub async fn settings() -> Json<Value> {
    Json(json!({"onDemandConcurrencyCap": 10}))
}

/// What a CI run changes in a test (`datadog-ci`'s config overrides).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TestPayload {
    #[serde(rename = "public_id", default)]
    pub public_id: Option<String>,
    #[serde(rename = "local_test_definition", default)]
    pub local_test_definition: Option<Test>,
    #[serde(default)]
    pub execution_rule: Option<String>,
    #[serde(default)]
    pub start_url: Option<String>,
    #[serde(default)]
    pub variables: HashMap<String, String>,
    #[serde(default)]
    pub headers: Map<String, Value>,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub body_type: Option<String>,
    #[serde(default)]
    pub basic_auth: Option<BasicAuth>,
    #[serde(default)]
    pub cookies: Option<Value>,
    #[serde(default)]
    pub follow_redirects: Option<bool>,
    #[serde(default)]
    pub allow_insecure_certificates: Option<bool>,
    #[serde(default)]
    pub device_ids: Vec<String>,
    /// Seconds.
    #[serde(default)]
    pub default_step_timeout: Option<f64>,
    #[serde(default)]
    pub retry: Option<Retry>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct BasicAuth {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Default, Deserialize)]
pub struct BatchOptions {
    /// Milliseconds.
    #[serde(default)]
    batch_timeout: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct TriggerBody {
    tests: Vec<TestPayload>,
    #[serde(default)]
    options: Option<BatchOptions>,
    #[serde(default)]
    metadata: Option<Value>,
}

/// A run of a batch, and what is needed to answer for it before it ends.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Entry {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_id: Option<String>,
    pub test_public_id: String,
    pub test_name: String,
    pub test_type: String,
    #[serde(default)]
    pub test_subtype: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dns_server: Option<String>,
    pub execution_rule: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
    #[serde(default)]
    pub max_retries: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Batch {
    pub batch_id: String,
    pub created: i64,
    pub timeout_ms: i64,
    pub entries: Vec<Entry>,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub metadata: Value,
}

/// The overrides as a run takes them.
pub fn overrides(p: &TestPayload, t: &Test) -> Overrides {
    let mut headers = p.headers.clone();
    if let Some(bt) = &p.body_type {
        headers.insert("Content-Type".into(), json!(bt));
    }
    if let Some(b) = &p.basic_auth {
        let token = base64::engine::general_purpose::STANDARD
            .encode(format!("{}:{}", b.username, b.password));
        headers.insert("Authorization".into(), json!(format!("Basic {token}")));
    }
    let cookies = match &p.cookies {
        Some(Value::String(s)) => Some(s.clone()),
        Some(Value::Object(o)) => {
            let v = o.get("value").and_then(Value::as_str).unwrap_or_default();
            let mine = t
                .config
                .request
                .headers
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case("cookie"))
                .and_then(|(_, v)| v.as_str());
            match (o.get("append") == Some(&json!(true)), mine) {
                (true, Some(m)) => Some(format!("{m}; {v}")),
                _ => Some(v.to_string()),
            }
        }
        _ => None,
    };
    if let Some(c) = cookies {
        headers.insert("Cookie".into(), json!(c));
    }
    Overrides {
        start_url: p.start_url.clone(),
        variables: p.variables.clone(),
        headers,
        body: p.body.clone(),
        follow_redirects: p.follow_redirects,
        accept_self_signed: p.allow_insecure_certificates,
        default_step_timeout: p.default_step_timeout,
        retry: p.retry.clone(),
        ..Default::default()
    }
}

fn execution_rule(p: &TestPayload, t: &Test) -> String {
    p.execution_rule
        .clone()
        .or_else(|| {
            t.options
                .extra
                .get("ci")
                .and_then(|c| c["executionRule"].as_str())
                .map(String::from)
        })
        .filter(|r| matches!(r.as_str(), "blocking" | "non_blocking" | "skipped"))
        .unwrap_or_else(|| "blocking".into())
}

/// `POST /api/v1/synthetics/tests/trigger/ci`: runs the tests (stored ones
/// by `public_id`, or local definitions) in the background.
pub async fn trigger(State(st): State<AppState>, Json(b): Json<TriggerBody>) -> ApiResult<Value> {
    if b.tests.is_empty() {
        return Err(ApiError::BadQuery("no tests to run".into()));
    }
    let mut runs: Vec<(Test, TestPayload)> = Vec::new();
    for p in b.tests {
        let t = match (&p.local_test_definition, &p.public_id) {
            (Some(def), _) => {
                let mut def = super::check(def.clone())?;
                def.public_id
                    .get_or_insert_with(|| format!("local-{}", new_result_id()));
                def
            }
            (None, Some(id)) => match load(&st, id).await {
                Ok(t) => t,
                Err(ApiError::NotFound) => {
                    return Err(ApiError::BadQuery(format!("test {id} not found")));
                }
                Err(e) => return Err(e),
            },
            (None, None) => {
                return Err(ApiError::BadQuery(
                    "each test needs a public_id or a local_test_definition".into(),
                ));
            }
        };
        runs.push((t, p));
    }
    let batch_id = uuid_like();
    let mut batch = Batch {
        batch_id: batch_id.clone(),
        created: now_ms(),
        timeout_ms: b
            .options
            .and_then(|o| o.batch_timeout)
            .unwrap_or(DEFAULT_TIMEOUT_MS)
            .clamp(60_000, 3 * 3_600_000),
        entries: vec![],
        metadata: b.metadata.unwrap_or_default(),
    };
    let mut results = Vec::new();
    let mut to_run = Vec::new();
    for (t, p) in runs {
        let rule = execution_rule(&p, &t);
        let pid = t.public_id.clone().unwrap_or_default();
        let devices: Vec<Option<String>> = if t.typ != "browser" {
            vec![None]
        } else if !p.device_ids.is_empty() {
            p.device_ids.iter().cloned().map(Some).collect()
        } else {
            t.options.device_ids.iter().cloned().map(Some).collect()
        };
        let devices = if devices.is_empty() {
            vec![Some("chrome.laptop_large".to_string())]
        } else {
            devices
        };
        let max_retries = p
            .retry
            .as_ref()
            .or(t.options.retry.as_ref())
            .map_or(0, |r| r.count.min(3));
        for device in devices {
            let skipped = rule == "skipped";
            let result_id = (!skipped).then(new_result_id);
            batch.entries.push(Entry {
                result_id: result_id.clone(),
                test_public_id: pid.clone(),
                test_name: t.name.clone(),
                test_type: t.typ.clone(),
                test_subtype: t.subtype.clone(),
                dns_server: t.config.request.dns_server.clone(),
                execution_rule: rule.clone(),
                device: device.clone(),
                max_retries,
            });
            if let Some(rid) = result_id {
                results.push(
                    json!({"device": device, "location": 1, "public_id": pid, "result_id": rid}),
                );
                let mut o = overrides(&p, &t);
                o.result_id = Some(rid);
                o.batch_id = Some(batch_id.clone());
                o.device_id = device;
                to_run.push((t.clone(), o));
            }
        }
    }
    st.store
        .pg
        .put_doc(
            BATCH_KIND,
            &batch_id,
            &serde_json::to_value(&batch).map_err(anyhow::Error::from)?,
        )
        .await?;
    for (t, o) in to_run {
        let st = st.clone();
        tokio::spawn(async move {
            super::run_and_record(&st, &t, o).await;
        });
    }
    let ids: Vec<String> = batch
        .entries
        .iter()
        .map(|e| e.test_public_id.clone())
        .collect();
    Ok(Json(json!({
        "batch_id": batch_id,
        "locations": [location()],
        "results": results,
        "triggered_check_ids": ids,
    })))
}

/// A batch id as Datadog's: a UUID.
fn uuid_like() -> String {
    super::variables::generate("{{ uuid }}", 0)
}

async fn load_batch(st: &AppState, id: &str) -> Result<Batch, ApiError> {
    let d = st
        .store
        .pg
        .doc(BATCH_KIND, id)
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(serde_json::from_value(d.body).map_err(anyhow::Error::from)?)
}

/// Where each run of a batch stands, and the batch: `in_progress` until
/// every run is done, then `failed` if a blocking run failed.
pub fn status(b: &Batch, done: &HashMap<String, RunResult>, now: i64) -> (String, Vec<Value>) {
    let timed_out = now - b.created > b.timeout_ms;
    let mut any_running = false;
    let mut any_blocking_failed = false;
    let rows = b
        .entries
        .iter()
        .map(|e| {
            let res = e.result_id.as_ref().and_then(|r| done.get(r));
            let (status, timeout) = match (e.execution_rule.as_str(), res) {
                ("skipped", _) => ("skipped", false),
                (_, Some(r)) => (if r.passed { "passed" } else { "failed" }, false),
                (_, None) if timed_out => ("failed", true),
                _ => ("in_progress", false),
            };
            if status == "in_progress" {
                any_running = true;
            }
            if status == "failed" && e.execution_rule == "blocking" {
                any_blocking_failed = true;
            }
            let mut row = json!({
                "batch_id": b.batch_id,
                "execution_rule": e.execution_rule,
                "location": LOCATION,
                "status": status,
                "test_name": e.test_name,
                "test_public_id": e.test_public_id,
                "test_type": e.test_type,
                "retries": res.map(|r| r.retries),
                "max_retries": e.max_retries,
                "timed_out": timeout,
                "duration": res.map_or(0.0, |r| r.duration_ms),
            });
            if let Some(rid) = &e.result_id {
                row["result_id"] = json!(rid);
            }
            if let Some(d) = &e.device {
                row["device"] = json!({"id": d});
            }
            row
        })
        .collect();
    let s = if any_running {
        "in_progress"
    } else if any_blocking_failed {
        "failed"
    } else {
        "passed"
    };
    (s.into(), rows)
}

async fn finished(st: &AppState, b: &Batch) -> Result<HashMap<String, RunResult>, ApiError> {
    let ids: Vec<String> = b
        .entries
        .iter()
        .filter_map(|e| e.result_id.clone())
        .collect();
    Ok(st
        .store
        .pg
        .synthetic_results_by_id(&ids)
        .await?
        .into_iter()
        .filter_map(|r| {
            serde_json::from_value::<RunResult>(r.body)
                .ok()
                .map(|x| (r.result_id, x))
        })
        .collect())
}

/// `GET /api/v1/synthetics/ci/batch/{batch_id}`.
pub async fn batch(State(st): State<AppState>, Path(id): Path<String>) -> ApiResult<Value> {
    let b = load_batch(&st, &id).await?;
    let done = finished(&st, &b).await?;
    let (status, results) = status(&b, &done, now_ms());
    Ok(Json(
        json!({"data": {"status": status, "results": results}}),
    ))
}

/// The recent batches, latest first (UI).
pub async fn batches(State(st): State<AppState>) -> ApiResult<Value> {
    let mut all: Vec<Batch> = st
        .store
        .pg
        .docs(BATCH_KIND)
        .await?
        .into_iter()
        .filter_map(|d| serde_json::from_value(d.body).ok())
        .collect();
    all.sort_by_key(|b| -b.created);
    all.truncate(50);
    let mut out = Vec::new();
    for b in all {
        let done = finished(&st, &b).await?;
        let (status, results) = status(&b, &done, now_ms());
        out.push(
            json!({"batch_id": b.batch_id, "created": b.created, "status": status,
            "metadata": b.metadata, "results": results}),
        );
    }
    Ok(Json(json!({"batches": out})))
}

/// A run as `datadog-ci` reads it.
pub fn server_result(e: &Entry, r: &RunResult, start_url: &str) -> Value {
    let status = |passed: bool| if passed { "passed" } else { "failed" };
    let failure = |f: &Option<String>| {
        f.as_ref()
            .map(|m| json!({"code": "ASSERTION_FAILURE", "message": m}))
    };
    let mut out = json!({
        "id": r.result_id,
        "status": status(r.passed),
        "finished_at": r.ts + r.duration_ms as i64,
        "duration": r.duration_ms,
        "timings": {"total": r.duration_ms},
    });
    if let Some(f) = failure(&r.failure) {
        out["failure"] = f;
    }
    if e.test_type == "browser" {
        out["start_url"] = json!(start_url);
        out["steps"] = r
            .steps
            .iter()
            .map(|s| {
                let mut step = json!({
                    "allow_failure": s.allow_failure,
                    "browser_errors": [],
                    "description": s.name,
                    "duration": s.duration_ms,
                    "status": status(s.passed),
                    "type": s.typ,
                    "url": s.url.clone().unwrap_or_default(),
                    "vitals_metrics": [],
                });
                if let Some(f) = &s.failure {
                    step["failure"] = json!({"message": f});
                }
                step
            })
            .collect();
    } else if e.test_subtype == "multi" {
        out["steps"] = r
            .steps
            .iter()
            .map(|s| {
                let mut step = json!({
                    "allow_failure": s.allow_failure,
                    "assertion_results": [],
                    "name": s.name,
                    "status": status(s.passed),
                    "subtype": s.typ,
                    "timings": {"total": s.duration_ms},
                });
                if let Some(f) = failure(&s.failure) {
                    step["failure"] = f;
                }
                step
            })
            .collect();
    } else {
        out["assertions"] = json!([]);
    }
    out
}

#[derive(Deserialize)]
pub struct PollQuery {
    result_ids: String,
}

/// `GET /api/v2/synthetics/tests/poll_results?result_ids=["…"]`: the runs
/// that are done, as JSON:API, with their tests.
pub async fn poll_results(
    State(st): State<AppState>,
    Query(q): Query<PollQuery>,
) -> ApiResult<Value> {
    let ids: Vec<String> = serde_json::from_str(&q.result_ids)
        .map_err(|_| ApiError::BadQuery("result_ids is a JSON array of ids".into()))?;
    let rows = st.store.pg.synthetic_results_by_id(&ids).await?;
    let mut batches: HashMap<String, Batch> = HashMap::new();
    let mut data = Vec::new();
    let mut included: HashMap<String, Value> = HashMap::new();
    for row in rows {
        let Ok(r) = serde_json::from_value::<RunResult>(row.body) else {
            continue;
        };
        let Some(bid) = row.batch_id.clone() else {
            continue;
        };
        if !batches.contains_key(&bid)
            && let Ok(b) = load_batch(&st, &bid).await
        {
            batches.insert(bid.clone(), b);
        }
        let Some(e) = batches.get(&bid).and_then(|b| {
            b.entries
                .iter()
                .find(|e| e.result_id.as_deref() == Some(&row.result_id))
        }) else {
            continue;
        };
        let start_url = r.details["start_url"].as_str().unwrap_or_default();
        let mut attributes = json!({
            "test_type": e.test_type,
            "result": server_result(e, &r, start_url),
        });
        if let Some(d) = &e.device {
            let (w, h, _) = browser::device(d);
            attributes["device"] = json!({"id": d, "resolution": {"width": w, "height": h}});
        }
        data.push(json!({
            "id": row.result_id,
            "type": "result",
            "attributes": attributes,
            "relationships": {"test": {"data": {"id": e.test_public_id, "type": "test"}}},
        }));
        included.entry(e.test_public_id.clone()).or_insert_with(|| {
            let mut config = json!({});
            if let Some(dns) = &e.dns_server {
                config["request"] = json!({"dns_server": dns});
            }
            let mut attrs = json!({"type": e.test_type, "config": config});
            if !e.test_subtype.is_empty() {
                attrs["subtype"] = json!(e.test_subtype);
            }
            json!({"type": "test", "id": e.test_public_id, "attributes": attrs})
        });
    }
    Ok(Json(json!({
        "data": data,
        "included": included.into_values().collect::<Vec<_>>(),
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(rid: Option<&str>, rule: &str) -> Entry {
        Entry {
            result_id: rid.map(String::from),
            test_public_id: "abc-def-ghi".into(),
            test_name: "home".into(),
            test_type: "api".into(),
            test_subtype: "http".into(),
            execution_rule: rule.into(),
            ..Default::default()
        }
    }

    #[test]
    fn batch_status() {
        let b = Batch {
            batch_id: "b".into(),
            created: 0,
            timeout_ms: 60_000,
            entries: vec![
                entry(Some("1"), "blocking"),
                entry(Some("2"), "non_blocking"),
                entry(None, "skipped"),
            ],
            metadata: Value::Null,
        };
        let res = |passed| RunResult {
            passed,
            ..Default::default()
        };
        let none = HashMap::new();
        assert_eq!(status(&b, &none, 1_000).0, "in_progress");
        // Past the timeout, unfinished runs fail.
        let (s, rows) = status(&b, &none, 61_000);
        assert_eq!(s, "failed");
        assert_eq!(rows[0]["timed_out"], true);
        assert_eq!(rows[2]["status"], "skipped");
        // A failing non-blocking run does not fail the batch.
        let done = HashMap::from([("1".to_string(), res(true)), ("2".to_string(), res(false))]);
        let (s, rows) = status(&b, &done, 1_000);
        assert_eq!(s, "passed");
        assert_eq!(rows[1]["status"], "failed");
        let done = HashMap::from([("1".to_string(), res(false)), ("2".to_string(), res(true))]);
        assert_eq!(status(&b, &done, 1_000).0, "failed");
    }

    #[test]
    fn overrides_headers() {
        let t: Test = serde_json::from_value(json!({"name": "x",
            "config": {"request": {"url": "https://x", "headers": {"Cookie": "a=1"}}}}))
        .unwrap();
        let p: TestPayload = serde_json::from_value(json!({"public_id": "abc",
            "basicAuth": {"username": "u", "password": "p"}, "bodyType": "application/json",
            "cookies": {"append": true, "value": "b=2"}, "startUrl": "https://staging",
            "variables": {"USER": "ci"}, "executionRule": "non_blocking"}))
        .unwrap();
        let o = overrides(&p, &t);
        assert_eq!(o.headers["Authorization"], "Basic dTpw");
        assert_eq!(o.headers["Content-Type"], "application/json");
        assert_eq!(o.headers["Cookie"], "a=1; b=2");
        assert_eq!(o.start_url.as_deref(), Some("https://staging"));
        assert_eq!(o.variables["USER"], "ci");
        assert_eq!(execution_rule(&p, &t), "non_blocking");
        let p2 = TestPayload::default();
        assert_eq!(execution_rule(&p2, &t), "blocking");
    }

    #[test]
    fn server_results() {
        let r = RunResult {
            result_id: "42".into(),
            ts: 1_000,
            passed: false,
            duration_ms: 250.0,
            failure: Some("status is 500".into()),
            ..Default::default()
        };
        let v = server_result(&entry(Some("42"), "blocking"), &r, "");
        assert_eq!(v["status"], "failed");
        assert_eq!(v["failure"]["message"], "status is 500");
        assert_eq!(v["finished_at"], 1_250);
        assert!(v["assertions"].is_array());
    }
}
