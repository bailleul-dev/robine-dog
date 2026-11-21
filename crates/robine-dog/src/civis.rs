//! CI Visibility: test events of the ddtrace test integrations
//! (`citestcycle`, agentless or through the Agent's EVP proxy), JUnit reports
//! uploaded by `datadog-ci junit upload`, and pipeline executions (Datadog's
//! pipeline API, GitHub Actions and GitLab webhooks). `ci.*` metrics are
//! emitted for monitors.

use crate::AppState;
use crate::api::{ApiError, ApiResult};
use crate::metrics::Sample;
use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::{Value, json};
use std::io::Read;

/// A test-level event (test, suite, module or session).
#[derive(Debug, Clone, PartialEq)]
pub struct TestEvent {
    pub level: String,
    pub session_id: Option<String>,
    pub service: String,
    pub env: Option<String>,
    pub name: String,
    pub suite: Option<String>,
    pub module: Option<String>,
    pub status: String,
    pub framework: Option<String>,
    pub start_ns: i64,
    pub duration: i64,
    pub error_message: Option<String>,
    pub branch: Option<String>,
    pub commit_sha: Option<String>,
    pub repository: Option<String>,
    pub meta: Value,
}

/// A pipeline, stage or job execution.
#[derive(Debug, Clone, PartialEq)]
pub struct PipelineEvent {
    pub provider: String,
    pub level: String,
    pub unique_id: String,
    pub pipeline_id: String,
    pub name: String,
    pub pipeline_name: String,
    pub stage: Option<String>,
    pub status: String,
    pub start_ms: i64,
    pub end_ms: i64,
    pub branch: Option<String>,
    pub commit_sha: Option<String>,
    pub repository: Option<String>,
    pub url: Option<String>,
}

fn s(v: &Value) -> Option<String> {
    match v {
        Value::String(x) if !x.is_empty() => Some(x.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn ungzip(headers: &HeaderMap, body: &[u8]) -> Vec<u8> {
    let gz = body.starts_with(&[0x1f, 0x8b])
        || headers
            .get(header::CONTENT_ENCODING)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.contains("gzip"));
    if !gz {
        return body.to_vec();
    }
    let mut out = Vec::new();
    match flate2::read::MultiGzDecoder::new(body).read_to_end(&mut out) {
        Ok(_) => out,
        Err(_) => body.to_vec(),
    }
}

/// A `citestcycle` payload (msgpack or JSON): `{metadata, events: [{type, content}]}`.
pub fn parse_citestcycle(root: &Value) -> Vec<TestEvent> {
    let defaults = &root["metadata"]["*"];
    let mut out = Vec::new();
    for ev in root["events"].as_array().into_iter().flatten() {
        let level = match ev["type"].as_str() {
            Some("test") => "test",
            Some("test_suite_end") => "suite",
            Some("test_module_end") => "module",
            Some("test_session_end") => "session",
            _ => continue,
        };
        let c = &ev["content"];
        let meta = &c["meta"];
        let m = |k: &str| meta[k].as_str().filter(|x| !x.is_empty()).map(String::from);
        let status = match m("test.status").as_deref() {
            Some("pass") => "pass",
            Some("fail") => "fail",
            Some("skip") => "skip",
            _ if c["error"].as_i64() == Some(1) => "fail",
            _ => "pass",
        };
        let suite = m("test.suite");
        let module = m("test.module");
        let name = match level {
            "test" => {
                m("test.name").unwrap_or_else(|| c["resource"].as_str().unwrap_or("").to_string())
            }
            "suite" => suite.clone().unwrap_or_default(),
            "module" => module.clone().unwrap_or_default(),
            _ => m("test.command").unwrap_or_else(|| "test session".into()),
        };
        out.push(TestEvent {
            level: level.into(),
            session_id: s(&c["test_session_id"]),
            service: c["service"]
                .as_str()
                .or(meta["service"].as_str())
                .or(defaults["service"].as_str())
                .unwrap_or("unknown")
                .to_string(),
            env: m("env").or(defaults["env"].as_str().map(String::from)),
            name,
            suite,
            module,
            status: status.into(),
            framework: m("test.framework"),
            start_ns: c["start"].as_i64().unwrap_or(0),
            duration: c["duration"].as_i64().unwrap_or(0),
            error_message: m("error.message").or(m("error.msg")),
            branch: m("git.branch"),
            commit_sha: m("git.commit.sha"),
            repository: m("git.repository_url"),
            meta: meta.clone(),
        });
    }
    out
}

/// The test cases of a JUnit XML report.
pub fn parse_junit(
    xml: &str,
    service: &str,
    tags: &Value,
    now_ns: i64,
) -> Result<Vec<TestEvent>, String> {
    let doc = roxmltree::Document::parse(xml).map_err(|e| format!("JUnit XML: {e}"))?;
    let t = |k: &str| tags[k].as_str().map(String::from);
    let mut out = Vec::new();
    for case in doc.descendants().filter(|n| n.has_tag_name("testcase")) {
        let suite = case.attribute("classname").map(String::from).or_else(|| {
            case.ancestors()
                .find(|a| a.has_tag_name("testsuite"))
                .and_then(|a| a.attribute("name"))
                .map(String::from)
        });
        let secs: f64 = case
            .attribute("time")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0.0);
        let child = |n: &str| case.children().find(|c| c.has_tag_name(n));
        let failure = child("failure").or(child("error"));
        let status = if failure.is_some() {
            "fail"
        } else if child("skipped").is_some() {
            "skip"
        } else {
            "pass"
        };
        out.push(TestEvent {
            level: "test".into(),
            session_id: None,
            service: service.to_string(),
            env: t("env"),
            name: case.attribute("name").unwrap_or("").to_string(),
            suite,
            module: None,
            status: status.into(),
            framework: Some("junit".into()),
            start_ns: now_ns,
            duration: (secs * 1e9) as i64,
            error_message: failure.and_then(|f| {
                f.attribute("message")
                    .map(String::from)
                    .or(f.text().map(|x| x.trim().to_string()))
            }),
            branch: t("git.branch"),
            commit_sha: t("git.commit.sha"),
            repository: t("git.repository_url"),
            meta: tags.clone(),
        });
    }
    Ok(out)
}

fn ts_ms(v: &Value) -> Option<i64> {
    let x = v.as_str()?;
    chrono::DateTime::parse_from_rfc3339(x)
        .map(|d| d.timestamp_millis())
        .ok()
        .or_else(|| {
            chrono::NaiveDateTime::parse_from_str(x.trim_end_matches(" UTC"), "%Y-%m-%d %H:%M:%S")
                .ok()
                .map(|d| d.and_utc().timestamp_millis())
        })
}

fn norm_status(x: &str) -> String {
    match x {
        "success" | "passed" => "success",
        "failure" | "failed" | "error" | "timed_out" | "startup_failure" => "error",
        "cancelled" | "canceled" => "canceled",
        "skipped" | "neutral" => "skipped",
        "blocked" | "manual" | "action_required" => "blocked",
        _ => "running",
    }
    .into()
}

/// Datadog's pipeline API: `{data: {attributes: {provider_name, resource}}}`.
pub fn parse_pipeline_api(v: &Value) -> Option<PipelineEvent> {
    let a = &v["data"]["attributes"];
    let r = &a["resource"];
    let level = r["level"].as_str()?.to_string();
    let unique_id = s(&r["unique_id"]).or(s(&r["id"]))?;
    let pipeline_id = if level == "pipeline" {
        unique_id.clone()
    } else {
        s(&r["pipeline_unique_id"])?
    };
    Some(PipelineEvent {
        provider: a["provider_name"].as_str().unwrap_or("custom").to_string(),
        name: r["name"].as_str().unwrap_or("").to_string(),
        pipeline_name: r["pipeline_name"]
            .as_str()
            .or(r["name"].as_str())
            .unwrap_or("")
            .to_string(),
        stage: r["stage_name"].as_str().map(String::from),
        status: norm_status(r["status"].as_str().unwrap_or("")),
        start_ms: ts_ms(&r["start"])?,
        end_ms: ts_ms(&r["end"])?,
        branch: r["git"]["branch"].as_str().map(String::from),
        commit_sha: r["git"]["sha"].as_str().map(String::from),
        repository: r["git"]["repository_url"].as_str().map(String::from),
        url: r["url"].as_str().map(String::from),
        level,
        unique_id,
        pipeline_id,
    })
}

/// A GitHub Actions (`workflow_run`, `workflow_job`) or GitLab (`Pipeline
/// Hook`, `Job Hook`) webhook, once the execution has finished.
pub fn parse_webhook(headers: &HeaderMap, v: &Value) -> Option<PipelineEvent> {
    let h = |k: &str| headers.get(k).and_then(|x| x.to_str().ok()).unwrap_or("");
    match (h("x-github-event"), h("x-gitlab-event")) {
        ("workflow_run", _) => {
            let r = &v["workflow_run"];
            if r["status"].as_str() != Some("completed") {
                return None;
            }
            let id = s(&r["id"])?;
            Some(PipelineEvent {
                provider: "github".into(),
                level: "pipeline".into(),
                pipeline_id: id.clone(),
                unique_id: id,
                name: r["name"].as_str().unwrap_or("").to_string(),
                pipeline_name: r["name"].as_str().unwrap_or("").to_string(),
                stage: None,
                status: norm_status(r["conclusion"].as_str().unwrap_or("")),
                start_ms: ts_ms(&r["run_started_at"]).or(ts_ms(&r["created_at"]))?,
                end_ms: ts_ms(&r["updated_at"])?,
                branch: r["head_branch"].as_str().map(String::from),
                commit_sha: r["head_sha"].as_str().map(String::from),
                repository: v["repository"]["html_url"].as_str().map(String::from),
                url: r["html_url"].as_str().map(String::from),
            })
        }
        ("workflow_job", _) => {
            let j = &v["workflow_job"];
            if j["status"].as_str() != Some("completed") {
                return None;
            }
            Some(PipelineEvent {
                provider: "github".into(),
                level: "job".into(),
                unique_id: s(&j["id"])?,
                pipeline_id: s(&j["run_id"])?,
                name: j["name"].as_str().unwrap_or("").to_string(),
                pipeline_name: j["workflow_name"].as_str().unwrap_or("").to_string(),
                stage: None,
                status: norm_status(j["conclusion"].as_str().unwrap_or("")),
                start_ms: ts_ms(&j["started_at"])?,
                end_ms: ts_ms(&j["completed_at"])?,
                branch: j["head_branch"].as_str().map(String::from),
                commit_sha: j["head_sha"].as_str().map(String::from),
                repository: v["repository"]["html_url"].as_str().map(String::from),
                url: j["html_url"].as_str().map(String::from),
            })
        }
        (_, "Pipeline Hook") => {
            let o = &v["object_attributes"];
            let status = norm_status(o["status"].as_str().unwrap_or(""));
            if status == "running" {
                return None;
            }
            let id = s(&o["id"])?;
            let name = o["name"]
                .as_str()
                .or(v["project"]["path_with_namespace"].as_str())
                .unwrap_or("")
                .to_string();
            Some(PipelineEvent {
                provider: "gitlab".into(),
                level: "pipeline".into(),
                pipeline_id: id.clone(),
                unique_id: id,
                pipeline_name: name.clone(),
                name,
                stage: None,
                status,
                start_ms: ts_ms(&o["created_at"])?,
                end_ms: ts_ms(&o["finished_at"]).or(ts_ms(&o["updated_at"]))?,
                branch: o["ref"].as_str().map(String::from),
                commit_sha: o["sha"].as_str().map(String::from),
                repository: v["project"]["web_url"].as_str().map(String::from),
                url: o["url"].as_str().map(String::from),
            })
        }
        (_, "Job Hook") => {
            let status = norm_status(v["build_status"].as_str().unwrap_or(""));
            if status == "running" {
                return None;
            }
            let end = ts_ms(&v["build_finished_at"])?;
            Some(PipelineEvent {
                provider: "gitlab".into(),
                level: "job".into(),
                unique_id: s(&v["build_id"])?,
                pipeline_id: s(&v["pipeline_id"])?,
                name: v["build_name"].as_str().unwrap_or("").to_string(),
                pipeline_name: v["project_name"].as_str().unwrap_or("").to_string(),
                stage: v["build_stage"].as_str().map(String::from),
                status,
                start_ms: ts_ms(&v["build_started_at"]).unwrap_or(end),
                end_ms: end,
                branch: v["ref"].as_str().map(String::from),
                commit_sha: v["sha"].as_str().map(String::from),
                repository: v["repository"]["homepage"].as_str().map(String::from),
                url: None,
            })
        }
        _ => None,
    }
}

// ---------------------------------------------------------------- storage

async fn store_tests(st: &AppState, events: &[TestEvent]) -> anyhow::Result<()> {
    let c = st.store.pg.conn().await?;
    let stmt = c
        .prepare(
            "INSERT INTO ci_tests (level, session_id, service, env, name, suite, module, status, framework,
                start_ns, duration, error_message, branch, commit_sha, repository, meta)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16)",
        )
        .await?;
    for e in events {
        c.execute(
            &stmt,
            &[
                &e.level,
                &e.session_id,
                &e.service,
                &e.env,
                &e.name,
                &e.suite,
                &e.module,
                &e.status,
                &e.framework,
                &e.start_ns,
                &e.duration,
                &e.error_message,
                &e.branch,
                &e.commit_sha,
                &e.repository,
                &e.meta,
            ],
        )
        .await?;
        let ts = e.start_ns / 1_000_000;
        let mut tags = vec![
            format!("test_level:{}", e.level),
            format!("test_service:{}", e.service),
            format!("test_status:{}", e.status),
        ];
        if let Some(b) = &e.branch {
            tags.push(format!("git_branch:{b}"));
        }
        st.aggregator
            .add(ts, "ci.test.count", None, tags.clone(), Sample::Count(1.0));
        st.aggregator.add(
            ts,
            "ci.test.duration",
            None,
            tags,
            Sample::Distribution(e.duration as f64 / 1e9),
        );
    }
    Ok(())
}

async fn store_pipeline(st: &AppState, p: &PipelineEvent) -> anyhow::Result<()> {
    let c = st.store.pg.conn().await?;
    c.execute(
        "INSERT INTO ci_pipelines (provider, level, unique_id, pipeline_id, name, pipeline_name, stage, status,
            start_ms, end_ms, branch, commit_sha, repository, url)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14)
         ON CONFLICT (provider, level, unique_id) DO UPDATE SET status = excluded.status,
            start_ms = excluded.start_ms, end_ms = excluded.end_ms, name = excluded.name",
        &[
            &p.provider, &p.level, &p.unique_id, &p.pipeline_id, &p.name, &p.pipeline_name, &p.stage,
            &p.status, &p.start_ms, &p.end_ms, &p.branch, &p.commit_sha, &p.repository, &p.url,
        ],
    )
    .await?;
    let mut tags = vec![
        format!("ci_level:{}", p.level),
        format!("ci_status:{}", p.status),
        format!("ci_provider_name:{}", p.provider),
        format!("ci_pipeline_name:{}", p.pipeline_name),
    ];
    if let Some(b) = &p.branch {
        tags.push(format!("git_branch:{b}"));
    }
    let name = if p.level == "pipeline" {
        "ci.pipeline"
    } else {
        "ci.job"
    };
    st.aggregator.add(
        p.end_ms,
        &format!("{name}.count"),
        None,
        tags.clone(),
        Sample::Count(1.0),
    );
    st.aggregator.add(
        p.end_ms,
        &format!("{name}.duration"),
        None,
        tags,
        Sample::Distribution((p.end_ms - p.start_ms) as f64 / 1000.0),
    );
    Ok(())
}

fn reply(r: Result<usize, String>) -> Response {
    match r {
        Ok(n) => (StatusCode::ACCEPTED, Json(json!({"accepted": n}))).into_response(),
        Err(e) => {
            tracing::warn!(error = %e, "CI Visibility payload rejected");
            (StatusCode::BAD_REQUEST, Json(json!({"errors": [e]}))).into_response()
        }
    }
}

pub async fn take_citestcycle(
    st: &AppState,
    headers: &HeaderMap,
    body: &[u8],
) -> Result<usize, String> {
    let raw = ungzip(headers, body);
    let json_ct = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.contains("json"));
    let root: Value = if json_ct {
        serde_json::from_slice(&raw).map_err(|e| e.to_string())?
    } else {
        let v = rmpv::decode::read_value(&mut &raw[..]).map_err(|e| format!("msgpack: {e}"))?;
        crate::intake::traces::mp_to_json(v)
    };
    let events = parse_citestcycle(&root);
    store_tests(st, &events).await.map_err(|e| e.to_string())?;
    Ok(events.len())
}

/// `POST /api/v2/citestcycle`.
pub async fn citestcycle(State(st): State<AppState>, headers: HeaderMap, body: Bytes) -> Response {
    reply(take_citestcycle(&st, &headers, &body).await)
}

/// `POST /api/v2/cireport`: `datadog-ci junit upload` (multipart: `event`
/// JSON and `junit_xml_report_file` files, possibly gzipped).
pub async fn junit(State(st): State<AppState>, headers: HeaderMap, body: Bytes) -> Response {
    let r = async {
        let ct = headers
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .ok_or("no content type")?;
        let boundary = multer::parse_boundary(ct).map_err(|e| e.to_string())?;
        let stream = futures::stream::once(async move { Ok::<Bytes, std::io::Error>(body) });
        let mut form = multer::Multipart::new(stream, boundary);
        let (mut event, mut files) = (Value::Null, Vec::new());
        while let Some(f) = form.next_field().await.map_err(|e| e.to_string())? {
            let name = f.name().unwrap_or_default().to_string();
            let data = f.bytes().await.map_err(|e| e.to_string())?;
            if name == "event" {
                event = serde_json::from_slice(&data).unwrap_or(Value::Null);
            } else {
                files.push(ungzip(&HeaderMap::new(), &data));
            }
        }
        let service = event["service"].as_str().unwrap_or("unknown").to_string();
        let mut tags = event.clone();
        if let Some(t) = event["tags"].as_object() {
            for (k, v) in t {
                tags[k] = v.clone();
            }
        }
        let now = crate::intake::logs::now_ms() * 1_000_000;
        let mut all = Vec::new();
        for f in files {
            all.extend(parse_junit(
                &String::from_utf8_lossy(&f),
                &service,
                &tags,
                now,
            )?);
        }
        store_tests(&st, &all).await.map_err(|e| e.to_string())?;
        Ok::<usize, String>(all.len())
    }
    .await;
    reply(r)
}

/// `POST /api/v2/ci/pipeline`.
pub async fn pipeline_api(State(st): State<AppState>, Json(v): Json<Value>) -> Response {
    let r = match parse_pipeline_api(&v) {
        None => {
            Err("expected data.attributes.resource with level, unique_id, start, end".to_string())
        }
        Some(p) => store_pipeline(&st, &p)
            .await
            .map(|_| 1)
            .map_err(|e| e.to_string()),
    };
    reply(r)
}

/// `POST /api/v2/webhook`: GitHub Actions and GitLab webhooks.
pub async fn webhook(
    State(st): State<AppState>,
    headers: HeaderMap,
    Json(v): Json<Value>,
) -> Response {
    let r = match parse_webhook(&headers, &v) {
        None => Ok(0),
        Some(p) => store_pipeline(&st, &p)
            .await
            .map(|_| 1)
            .map_err(|e| e.to_string()),
    };
    reply(r)
}

pub fn spawn_purge(st: AppState) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(3600));
        loop {
            tick.tick().await;
            let before = crate::intake::logs::now_ms() - st.cfg.retention_days * 86_400_000;
            let r = async {
                let c = st.store.pg.conn().await?;
                c.execute(
                    "DELETE FROM ci_tests WHERE start_ns < $1",
                    &[&(before * 1_000_000)],
                )
                .await?;
                c.execute("DELETE FROM ci_pipelines WHERE end_ms < $1", &[&before])
                    .await?;
                anyhow::Ok(())
            }
            .await;
            if let Err(e) = r {
                tracing::warn!(error = %e, "purging CI Visibility data");
            }
        }
    });
}

// ---------------------------------------------------------------- queries

#[derive(Debug, Deserialize)]
pub struct Filter {
    pub from: i64,
    pub to: i64,
    pub service: Option<String>,
    pub status: Option<String>,
    pub branch: Option<String>,
    pub query: Option<String>,
}

fn pg_err(e: tokio_postgres::Error) -> ApiError {
    ApiError::Internal(e.into())
}

/// `GET /api/ui/ci/tests`: test runs, summary per service, flaky and slow tests.
pub async fn tests(State(st): State<AppState>, Query(f): Query<Filter>) -> ApiResult<Value> {
    let c = st.store.pg.conn().await?;
    let (from, to) = (f.from * 1_000_000, f.to * 1_000_000);
    let pattern = f
        .query
        .as_deref()
        .filter(|q| !q.trim().is_empty())
        .map(|q| format!("%{}%", q.trim()));
    let runs = c
        .query(
            "SELECT id, service, name, suite, status, duration, start_ns, error_message, branch, commit_sha, framework
             FROM ci_tests WHERE level = 'test' AND start_ns >= $1 AND start_ns <= $2
               AND ($3::TEXT IS NULL OR service = $3) AND ($4::TEXT IS NULL OR status = $4)
               AND ($5::TEXT IS NULL OR branch = $5)
               AND ($6::TEXT IS NULL OR name ILIKE $6 OR suite ILIKE $6 OR error_message ILIKE $6)
             ORDER BY start_ns DESC LIMIT 300",
            &[&from, &to, &f.service, &f.status, &f.branch, &pattern],
        )
        .await
        .map_err(pg_err)?;
    let services = c
        .query(
            "SELECT service, count(*), count(*) FILTER (WHERE status = 'fail'), count(*) FILTER (WHERE status = 'skip'),
                count(DISTINCT session_id), coalesce(avg(duration), 0)::BIGINT
             FROM ci_tests WHERE level = 'test' AND start_ns >= $1 AND start_ns <= $2 GROUP BY service ORDER BY service",
            &[&from, &to],
        )
        .await
        .map_err(pg_err)?;
    // Flaky: passed and failed on the same commit.
    let flaky = c
        .query(
            "SELECT service, suite, name, count(DISTINCT commit_sha), max(start_ns),
                (array_agg(error_message ORDER BY start_ns DESC) FILTER (WHERE status = 'fail'))[1]
             FROM (SELECT service, suite, name, commit_sha, start_ns, status, error_message,
                     bool_or(status = 'pass') OVER w AND bool_or(status = 'fail') OVER w AS flaky
                   FROM ci_tests WHERE level = 'test' AND commit_sha IS NOT NULL AND start_ns >= $1 AND start_ns <= $2
                   WINDOW w AS (PARTITION BY service, suite, name, commit_sha)) t
             WHERE flaky GROUP BY service, suite, name ORDER BY 5 DESC LIMIT 100",
            &[&from, &to],
        )
        .await
        .map_err(pg_err)?;
    let slowest = c
        .query(
            "SELECT service, suite, name, count(*), avg(duration)::BIGINT, max(duration)
             FROM ci_tests WHERE level = 'test' AND start_ns >= $1 AND start_ns <= $2
               AND ($3::TEXT IS NULL OR service = $3)
             GROUP BY service, suite, name ORDER BY 5 DESC LIMIT 20",
            &[&from, &to, &f.service],
        )
        .await
        .map_err(pg_err)?;
    Ok(Json(json!({
        "runs": runs.iter().map(|r| json!({"id": r.get::<_, i64>(0), "service": r.get::<_, String>(1),
            "name": r.get::<_, String>(2), "suite": r.get::<_, Option<String>>(3), "status": r.get::<_, String>(4),
            "duration": r.get::<_, i64>(5), "start": r.get::<_, i64>(6) / 1_000_000,
            "error": r.get::<_, Option<String>>(7), "branch": r.get::<_, Option<String>>(8),
            "commit": r.get::<_, Option<String>>(9), "framework": r.get::<_, Option<String>>(10)})).collect::<Vec<_>>(),
        "services": services.iter().map(|r| json!({"service": r.get::<_, String>(0), "tests": r.get::<_, i64>(1),
            "failed": r.get::<_, i64>(2), "skipped": r.get::<_, i64>(3), "sessions": r.get::<_, i64>(4),
            "avg": r.get::<_, i64>(5)})).collect::<Vec<_>>(),
        "flaky": flaky.iter().map(|r| json!({"service": r.get::<_, String>(0), "suite": r.get::<_, Option<String>>(1),
            "name": r.get::<_, String>(2), "commits": r.get::<_, i64>(3), "last": r.get::<_, i64>(4) / 1_000_000,
            "error": r.get::<_, Option<String>>(5)})).collect::<Vec<_>>(),
        "slowest": slowest.iter().map(|r| json!({"service": r.get::<_, String>(0), "suite": r.get::<_, Option<String>>(1),
            "name": r.get::<_, String>(2), "runs": r.get::<_, i64>(3), "avg": r.get::<_, i64>(4),
            "max": r.get::<_, i64>(5)})).collect::<Vec<_>>(),
    })))
}

/// `GET /api/ui/ci/pipelines`: executions and a summary per pipeline.
pub async fn pipelines(State(st): State<AppState>, Query(f): Query<Filter>) -> ApiResult<Value> {
    let c = st.store.pg.conn().await?;
    let rows = c
        .query(
            "SELECT id, provider, pipeline_id, name, status, start_ms, end_ms, branch, commit_sha, repository, url
             FROM ci_pipelines WHERE level = 'pipeline' AND end_ms >= $1 AND end_ms <= $2
               AND ($3::TEXT IS NULL OR name = $3) AND ($4::TEXT IS NULL OR status = $4)
               AND ($5::TEXT IS NULL OR branch = $5)
             ORDER BY end_ms DESC LIMIT 300",
            &[&f.from, &f.to, &f.service, &f.status, &f.branch],
        )
        .await
        .map_err(pg_err)?;
    let summary = c
        .query(
            "SELECT pipeline_name, provider, count(*), count(*) FILTER (WHERE status = 'success'),
                percentile_cont(0.5) WITHIN GROUP (ORDER BY end_ms - start_ms)::BIGINT,
                percentile_cont(0.95) WITHIN GROUP (ORDER BY end_ms - start_ms)::BIGINT,
                (array_agg(status ORDER BY end_ms DESC))[1], max(repository)
             FROM ci_pipelines WHERE level = 'pipeline' AND end_ms >= $1 AND end_ms <= $2
             GROUP BY pipeline_name, provider ORDER BY 3 DESC",
            &[&f.from, &f.to],
        )
        .await
        .map_err(pg_err)?;
    Ok(Json(json!({
        "executions": rows.iter().map(|r| json!({"id": r.get::<_, i64>(0), "provider": r.get::<_, String>(1),
            "pipeline_id": r.get::<_, String>(2), "name": r.get::<_, String>(3), "status": r.get::<_, String>(4),
            "start": r.get::<_, i64>(5), "end": r.get::<_, i64>(6), "branch": r.get::<_, Option<String>>(7),
            "commit": r.get::<_, Option<String>>(8), "repository": r.get::<_, Option<String>>(9),
            "url": r.get::<_, Option<String>>(10)})).collect::<Vec<_>>(),
        "pipelines": summary.iter().map(|r| json!({"name": r.get::<_, String>(0), "provider": r.get::<_, String>(1),
            "executions": r.get::<_, i64>(2), "successes": r.get::<_, i64>(3), "p50": r.get::<_, i64>(4),
            "p95": r.get::<_, i64>(5), "last_status": r.get::<_, String>(6),
            "repository": r.get::<_, Option<String>>(7)})).collect::<Vec<_>>(),
    })))
}

/// `GET /api/ui/ci/pipelines/{provider}/{id}/jobs`.
pub async fn jobs(
    State(st): State<AppState>,
    Path((provider, id)): Path<(String, String)>,
) -> ApiResult<Value> {
    let c = st.store.pg.conn().await?;
    let rows = c
        .query(
            "SELECT level, name, stage, status, start_ms, end_ms, url FROM ci_pipelines
             WHERE provider = $1 AND pipeline_id = $2 AND level <> 'pipeline' ORDER BY start_ms",
            &[&provider, &id],
        )
        .await
        .map_err(pg_err)?;
    Ok(Json(json!(rows.iter().map(|r| json!({"level": r.get::<_, String>(0), "name": r.get::<_, String>(1),
        "stage": r.get::<_, Option<String>>(2), "status": r.get::<_, String>(3), "start": r.get::<_, i64>(4),
        "end": r.get::<_, i64>(5), "url": r.get::<_, Option<String>>(6)})).collect::<Vec<_>>())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn citestcycle_events() {
        let v = json!({"version": 1, "metadata": {"*": {"language": "python", "env": "ci"}}, "events": [
            {"type": "test", "version": 2, "content": {"trace_id": 1, "span_id": 2, "test_session_id": 77, "service": "shop",
             "start": 5, "duration": 9, "error": 1, "meta": {"test.name": "test_cart", "test.suite": "tests/test_cart.py",
             "test.status": "fail", "test.framework": "pytest", "git.branch": "main", "git.commit.sha": "abc", "error.message": "boom"}}},
            {"type": "test_session_end", "content": {"service": "shop", "test_session_id": 77, "start": 1, "duration": 20,
             "meta": {"test.status": "fail", "test.command": "pytest"}}},
            {"type": "span", "content": {}}
        ]});
        let e = parse_citestcycle(&v);
        assert_eq!(e.len(), 2);
        assert_eq!(
            (
                e[0].name.as_str(),
                e[0].status.as_str(),
                e[0].env.as_deref()
            ),
            ("test_cart", "fail", Some("ci"))
        );
        assert_eq!(e[0].session_id.as_deref(), Some("77"));
        assert_eq!(
            (e[1].level.as_str(), e[1].name.as_str()),
            ("session", "pytest")
        );
    }

    #[test]
    fn junit_report() {
        let xml = r#"<testsuites><testsuite name="api"><testcase classname="api.Cart" name="adds" time="0.5"/>
            <testcase classname="api.Cart" name="removes" time="1"><failure message="expected 1"/></testcase>
            <testcase name="later"><skipped/></testcase></testsuite></testsuites>"#;
        let t = parse_junit(xml, "shop", &json!({"git.branch": "main"}), 0).unwrap();
        assert_eq!(t.len(), 3);
        assert_eq!(
            (t[1].status.as_str(), t[1].error_message.as_deref()),
            ("fail", Some("expected 1"))
        );
        assert_eq!(t[2].suite.as_deref(), Some("api"));
        assert_eq!(t[0].duration, 500_000_000);
        assert_eq!(t[0].branch.as_deref(), Some("main"));
    }

    #[test]
    fn pipelines_from_api_and_webhooks() {
        let api = json!({"data": {"type": "cipipeline_resource_request", "attributes": {"provider_name": "jenkins",
            "resource": {"level": "pipeline", "unique_id": "p1", "name": "build", "status": "error",
            "start": "2026-11-21T10:00:00Z", "end": "2026-11-21T10:05:00Z", "git": {"branch": "main", "sha": "abc"}}}}});
        let p = parse_pipeline_api(&api).unwrap();
        assert_eq!(
            (p.status.as_str(), p.end_ms - p.start_ms),
            ("error", 300_000)
        );
        let mut h = HeaderMap::new();
        h.insert("x-gitlab-event", "Job Hook".parse().unwrap());
        let job = json!({"build_id": 9, "pipeline_id": 4, "build_name": "test", "build_stage": "test", "build_status": "failed",
            "build_started_at": "2026-11-21 10:00:00 UTC", "build_finished_at": "2026-11-21 10:01:00 UTC", "project_name": "shop"});
        let j = parse_webhook(&h, &job).unwrap();
        assert_eq!(
            (j.level.as_str(), j.status.as_str(), j.pipeline_id.as_str()),
            ("job", "error", "4")
        );
        let mut g = HeaderMap::new();
        g.insert("x-github-event", "workflow_run".parse().unwrap());
        let run = json!({"workflow_run": {"id": 3, "name": "CI", "status": "in_progress"}});
        assert!(parse_webhook(&g, &run).is_none());
    }
}
