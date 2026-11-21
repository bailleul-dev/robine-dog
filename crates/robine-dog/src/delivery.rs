//! Software Delivery: DORA Metrics, CD Visibility and Code Coverage.
//!
//! Deployments come from the DORA API (`POST /api/v2/dora/deployment`),
//! `datadog-ci deployment mark` (a custom span tagged `cd.*`), and GitHub
//! `deployment_status` / GitLab `Deployment Hook` webhooks. Failures come from
//! the DORA API (`POST /api/v2/dora/incident`) and robine-dog's incidents.
//! Coverage reports come from `datadog-ci coverage upload` (lcov, Cobertura,
//! JaCoCo).

use crate::AppState;
use crate::api::{ApiError, ApiResult};
use axum::Json;
use axum::body::Bytes;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::Read;

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Deployment {
    pub service: String,
    pub env: String,
    pub version: Option<String>,
    pub team: Option<String>,
    pub commit_sha: Option<String>,
    pub repository: Option<String>,
    pub started_ms: i64,
    pub finished_ms: i64,
    pub status: String,
    pub rollback: bool,
    pub source: String,
    pub url: Option<String>,
}

fn s(v: &Value) -> Option<String> {
    match v {
        Value::String(x) if !x.is_empty() => Some(x.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// Seconds, milliseconds or nanoseconds since the epoch, or RFC 3339.
fn ms(v: &Value) -> Option<i64> {
    if let Some(x) = v.as_i64() {
        return Some(match x {
            x if x > 100_000_000_000_000_000 => x / 1_000_000,
            x if x > 100_000_000_000_000 => x / 1_000,
            x if x > 100_000_000_000 => x,
            x => x * 1000,
        });
    }
    chrono::DateTime::parse_from_rfc3339(v.as_str()?)
        .ok()
        .map(|d| d.timestamp_millis())
}

/// `POST /api/v2/dora/deployment`: `{data: {attributes: {service, env,
/// started_at, finished_at (ns), version, team, git: {commit_sha,
/// repository_url}}}}`.
pub fn parse_dora_deployment(v: &Value) -> Option<Deployment> {
    let a = &v["data"]["attributes"];
    let finished = ms(&a["finished_at"])?;
    Some(Deployment {
        service: s(&a["service"])?,
        env: s(&a["env"]).unwrap_or_else(|| "none".into()),
        version: s(&a["version"]),
        team: s(&a["team"]),
        commit_sha: s(&a["git"]["commit_sha"]),
        repository: s(&a["git"]["repository_url"]),
        started_ms: ms(&a["started_at"]).unwrap_or(finished),
        finished_ms: finished,
        status: "success".into(),
        rollback: false,
        source: "dora".into(),
        url: None,
    })
}

/// A `datadog-ci deployment mark` custom span: tags `cd.env`, `cd.service`,
/// `cd.revision`, `cd.is_rollback`, as an object or `key:value` strings.
pub fn parse_custom_span(v: &Value) -> Option<Deployment> {
    let a = if v["data"]["attributes"].is_object() {
        &v["data"]["attributes"]
    } else {
        v
    };
    let mut tags: HashMap<String, String> = HashMap::new();
    match &a["tags"] {
        Value::Object(m) => {
            for (k, x) in m {
                if let Some(x) = s(x) {
                    tags.insert(k.clone(), x);
                }
            }
        }
        Value::Array(l) => {
            for t in l.iter().filter_map(|x| x.as_str()) {
                if let Some((k, x)) = t.split_once(':') {
                    tags.insert(k.into(), x.into());
                }
            }
        }
        _ => {}
    }
    let env = tags.get("cd.env")?.clone();
    let finished = ms(&a["end_time"]).or(ms(&a["end"]))?;
    let failed = a["exit_code"].as_i64().is_some_and(|c| c != 0) || a["error_message"].is_string();
    Some(Deployment {
        service: tags
            .get("cd.service")
            .or(tags.get("service"))
            .cloned()
            .unwrap_or_else(|| "unknown".into()),
        env,
        version: tags.get("cd.revision").cloned(),
        team: tags.get("team").cloned(),
        commit_sha: tags.get("git.commit.sha").cloned(),
        repository: tags.get("git.repository_url").cloned(),
        started_ms: ms(&a["start_time"]).or(ms(&a["start"])).unwrap_or(finished),
        finished_ms: finished,
        status: if failed { "failure" } else { "success" }.into(),
        rollback: tags.get("cd.is_rollback").is_some_and(|x| x == "true"),
        source: "datadog-ci".into(),
        url: tags.get("ci.pipeline.url").cloned(),
    })
}

/// GitHub `deployment_status` and GitLab `Deployment Hook`, once finished.
pub fn parse_webhook(headers: &HeaderMap, v: &Value) -> Option<Deployment> {
    let h = |k: &str| headers.get(k).and_then(|x| x.to_str().ok()).unwrap_or("");
    if h("x-github-event") == "deployment_status" {
        let d = &v["deployment"];
        let st = &v["deployment_status"];
        let status = match st["state"].as_str()? {
            "success" => "success",
            "failure" | "error" => "failure",
            _ => return None,
        };
        return Some(Deployment {
            service: s(&d["payload"]["service"]).or(s(&v["repository"]["name"]))?,
            env: s(&d["environment"]).unwrap_or_else(|| "none".into()),
            version: s(&d["ref"]),
            team: None,
            commit_sha: s(&d["sha"]),
            repository: s(&v["repository"]["html_url"]),
            started_ms: ms(&d["created_at"])?,
            finished_ms: ms(&st["updated_at"]).or(ms(&st["created_at"]))?,
            status: status.into(),
            rollback: false,
            source: "github".into(),
            url: s(&st["target_url"]).or(s(&st["log_url"])),
        });
    }
    if h("x-gitlab-event") == "Deployment Hook" {
        let status = match v["status"].as_str()? {
            "success" => "success",
            "failed" | "canceled" => "failure",
            _ => return None,
        };
        let finished = ms(&v["status_changed_at"]).or_else(|| {
            ms(&json!(
                v["status_changed_at"]
                    .as_str()?
                    .replace(" UTC", "Z")
                    .replacen(' ', "T", 1)
            ))
        })?;
        return Some(Deployment {
            service: s(&v["project"]["name"])?,
            env: s(&v["environment"]).unwrap_or_else(|| "none".into()),
            version: s(&v["ref"]),
            team: None,
            commit_sha: v["commit_url"]
                .as_str()
                .and_then(|u| u.rsplit('/').next())
                .map(String::from)
                .or(s(&v["short_sha"])),
            repository: s(&v["project"]["web_url"]),
            started_ms: finished,
            finished_ms: finished,
            status: status.into(),
            rollback: false,
            source: "gitlab".into(),
            url: s(&v["deployable_url"]),
        });
    }
    None
}

pub async fn store(st: &AppState, d: &Deployment) -> anyhow::Result<()> {
    let c = st.store.pg.conn().await?;
    c.execute(
        "INSERT INTO deployments (service, env, version, team, commit_sha, repository, started_ms, finished_ms, status, rollback, source, url)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)",
        &[&d.service, &d.env, &d.version, &d.team, &d.commit_sha, &d.repository, &d.started_ms, &d.finished_ms, &d.status, &d.rollback, &d.source, &d.url],
    )
    .await?;
    Ok(())
}

fn accepted(r: anyhow::Result<()>) -> Response {
    match r {
        Ok(()) => (
            StatusCode::ACCEPTED,
            Json(json!({"data": {"id": crate::store::pg::new_id()}})),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"errors": [e.to_string()]})),
        )
            .into_response(),
    }
}

fn bad(m: &str) -> Response {
    (StatusCode::BAD_REQUEST, Json(json!({"errors": [m]}))).into_response()
}

/// `POST /api/v2/dora/deployment`.
pub async fn dora_deployment(State(st): State<AppState>, Json(v): Json<Value>) -> Response {
    match parse_dora_deployment(&v) {
        Some(d) => accepted(store(&st, &d).await),
        None => bad("expected data.attributes with service and finished_at"),
    }
}

/// `POST /api/v2/dora/incident`: `{data: {attributes: {services, env, name,
/// severity, started_at, finished_at, team}}}`.
pub async fn dora_incident(State(st): State<AppState>, Json(v): Json<Value>) -> Response {
    let a = &v["data"]["attributes"];
    let Some(started) = ms(&a["started_at"]) else {
        return bad("expected data.attributes.started_at");
    };
    let services: Vec<String> = match &a["services"] {
        Value::Array(l) => l.iter().filter_map(s).collect(),
        _ => s(&a["service"]).into_iter().collect(),
    };
    let r = async {
        let c = st.store.pg.conn().await?;
        c.execute(
            "INSERT INTO dora_incidents (services, env, name, severity, team, started_ms, finished_ms) VALUES ($1,$2,$3,$4,$5,$6,$7)",
            &[&services, &s(&a["env"]), &s(&a["name"]), &s(&a["severity"]), &s(&a["team"]), &started, &ms(&a["finished_at"])],
        )
        .await?;
        anyhow::Ok(())
    }
    .await;
    accepted(r)
}

/// `POST /api/intake/ci/custom_spans`: `datadog-ci deployment mark`.
pub async fn custom_span(State(st): State<AppState>, Json(v): Json<Value>) -> Response {
    match parse_custom_span(&v) {
        Some(d) => accepted(store(&st, &d).await),
        None => (StatusCode::ACCEPTED, Json(json!({}))).into_response(),
    }
}

// ---------- coverage ----------

/// (covered lines, total lines) of a report.
pub fn coverage_of(format: &str, text: &str) -> Option<(i64, i64)> {
    let attr = |tag: &str, name: &str| -> Option<i64> {
        let start = text.find(tag)?;
        let rest = &text[start..];
        let end = rest.find('>')?;
        let el = &rest[..end];
        let k = format!("{name}=\"");
        let i = el.find(&k)? + k.len();
        el[i..].split('"').next()?.parse().ok()
    };
    let f = format.to_ascii_lowercase();
    if f.contains("lcov") || text.starts_with("TN:") || text.starts_with("SF:") {
        let (mut hit, mut found) = (0, 0);
        for l in text.lines() {
            if let Some(x) = l.strip_prefix("LH:") {
                hit += x.trim().parse::<i64>().unwrap_or(0);
            } else if let Some(x) = l.strip_prefix("LF:") {
                found += x.trim().parse::<i64>().unwrap_or(0);
            }
        }
        return (found > 0).then_some((hit, found));
    }
    if text.contains("<coverage") && text.contains("lines-valid") {
        return Some((
            attr("<coverage", "lines-covered")?,
            attr("<coverage", "lines-valid")?,
        ));
    }
    if text.contains("<report") {
        // JaCoCo: the report's own LINE counter is the last one.
        let i = text.rfind("<counter type=\"LINE\"")?;
        let el = &text[i..i + text[i..].find('>')?];
        let get = |n: &str| -> Option<i64> {
            let k = format!("{n}=\"");
            let j = el.find(&k)? + k.len();
            el[j..].split('"').next()?.parse().ok()
        };
        let (m, c) = (get("missed")?, get("covered")?);
        return Some((c, m + c));
    }
    None
}

/// `POST /api/v2/cicovreprt`: `datadog-ci coverage upload`, multipart with an
/// `event` JSON and gzipped `code_coverage_report_file` parts.
pub async fn coverage_upload(
    State(st): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let r = async {
        let ct = headers
            .get(axum::http::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| anyhow::anyhow!("multipart expected"))?;
        let boundary = multer::parse_boundary(ct)?;
        let stream = futures::stream::once(async move { Ok::<Bytes, std::io::Error>(body) });
        let mut form = multer::Multipart::new(stream, boundary);
        let mut event = Value::Null;
        let mut reports: Vec<String> = Vec::new();
        while let Some(field) = form.next_field().await? {
            let name = field.name().unwrap_or_default().to_string();
            let data = field.bytes().await?;
            if name == "event" {
                event = serde_json::from_slice(&data)?;
            } else if name == "code_coverage_report_file" {
                let mut out = String::new();
                if data.starts_with(&[0x1f, 0x8b]) {
                    flate2::read::GzDecoder::new(&data[..]).read_to_string(&mut out)?;
                } else {
                    out = String::from_utf8_lossy(&data).into_owned();
                }
                reports.push(out);
            }
        }
        let format = event["format"].as_str().unwrap_or("").to_string();
        let (mut covered, mut total) = (0i64, 0i64);
        for r in &reports {
            if let Some((c, t)) = coverage_of(&format, r) {
                covered += c;
                total += t;
            }
        }
        anyhow::ensure!(total > 0, "no lines found in the coverage reports");
        let g = |k: &str| s(&event[k]);
        let c = st.store.pg.conn().await?;
        c.execute(
            "INSERT INTO coverage_reports (repository, branch, commit_sha, service, flags, format, covered, total, ts)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)",
            &[
                &g("git.repository_url").unwrap_or_default(),
                &g("git.branch"),
                &g("git.commit.sha"),
                &g("service"),
                &event["report.flags"].as_array().map(|f| f.iter().filter_map(s).collect::<Vec<_>>().join(",")),
                &format,
                &covered,
                &total,
                &crate::intake::logs::now_ms(),
            ],
        )
        .await?;
        anyhow::Ok(())
    }
    .await;
    match r {
        Ok(()) => (StatusCode::ACCEPTED, Json(json!({}))).into_response(),
        Err(e) => bad(&e.to_string()),
    }
}

// ---------- UI ----------

#[derive(Debug, Deserialize)]
pub struct Range {
    pub from: i64,
    pub to: i64,
    #[serde(default)]
    pub service: Option<String>,
    #[serde(default)]
    pub env: Option<String>,
}

fn dep_json(r: &tokio_postgres::Row) -> Value {
    json!({
        "id": r.get::<_, i64>("id"), "service": r.get::<_, String>("service"), "env": r.get::<_, String>("env"),
        "version": r.get::<_, Option<String>>("version"), "team": r.get::<_, Option<String>>("team"),
        "commit_sha": r.get::<_, Option<String>>("commit_sha"), "repository": r.get::<_, Option<String>>("repository"),
        "started_ms": r.get::<_, i64>("started_ms"), "finished_ms": r.get::<_, i64>("finished_ms"),
        "status": r.get::<_, String>("status"), "rollback": r.get::<_, bool>("rollback"),
        "source": r.get::<_, String>("source"), "url": r.get::<_, Option<String>>("url"),
    })
}

async fn deployments_in(st: &AppState, q: &Range) -> anyhow::Result<Vec<tokio_postgres::Row>> {
    let c = st.store.pg.conn().await?;
    Ok(c.query(
        "SELECT * FROM deployments WHERE finished_ms BETWEEN $1 AND $2
           AND ($3::TEXT IS NULL OR service = $3) AND ($4::TEXT IS NULL OR env = $4)
         ORDER BY finished_ms DESC LIMIT 5000",
        &[&q.from, &q.to, &q.service, &q.env],
    )
    .await?)
}

/// `GET /api/ui/deployments`.
pub async fn deployments(State(st): State<AppState>, Query(q): Query<Range>) -> ApiResult<Value> {
    let rows = deployments_in(&st, &q).await?;
    Ok(Json(
        json!({"data": rows.iter().map(dep_json).collect::<Vec<_>>()}),
    ))
}

/// A failure window: (services, started, restored).
type Failure = (Vec<String>, i64, Option<i64>);

async fn failures(st: &AppState, q: &Range) -> anyhow::Result<Vec<Failure>> {
    let c = st.store.pg.conn().await?;
    let mut out: Vec<Failure> = c
        .query(
            "SELECT services, started_ms, finished_ms FROM dora_incidents WHERE started_ms BETWEEN $1 AND $2
               AND ($3::TEXT IS NULL OR env IS NULL OR env = $3)",
            &[&q.from, &q.to, &q.env],
        )
        .await?
        .iter()
        .map(|r| (r.get(0), r.get(1), r.get(2)))
        .collect();
    for d in st.store.pg.docs("incident").await? {
        let Ok(i) = serde_json::from_value::<crate::api::incidents::Incident>(d.body) else {
            continue;
        };
        if i.created >= q.from && i.created <= q.to {
            out.push((i.services, i.created, i.resolved));
        }
    }
    Ok(out)
}

/// The four DORA metrics over deployments and failures.
pub fn dora(
    deps: &[Deployment],
    fails: &[Failure],
    lead: &HashMap<String, i64>,
    days: f64,
) -> Value {
    let n = deps.len() as f64;
    let mut by_service: HashMap<&str, Vec<&Deployment>> = HashMap::new();
    for d in deps {
        by_service.entry(&d.service).or_default().push(d);
    }
    // A deployment failed when it failed, was rolled back, or a failure on
    // its service started after it and before the service's next deployment.
    let mut failed = 0;
    for list in by_service.values_mut() {
        list.sort_by_key(|d| d.finished_ms);
        for (i, d) in list.iter().enumerate() {
            let next = list.get(i + 1).map(|x| x.finished_ms).unwrap_or(i64::MAX);
            let incident = fails
                .iter()
                .any(|(svcs, t, _)| svcs.contains(&d.service) && *t >= d.finished_ms && *t < next);
            if d.status == "failure" || d.rollback || incident {
                failed += 1;
            }
        }
    }
    let leads: Vec<i64> = deps
        .iter()
        .filter_map(|d| Some(d.finished_ms - lead.get(d.commit_sha.as_deref()?)?))
        .filter(|x| *x >= 0)
        .collect();
    let restores: Vec<i64> = fails
        .iter()
        .filter_map(|(_, s, e)| Some(e.as_ref()? - s))
        .collect();
    let median = |mut v: Vec<i64>| -> Option<i64> {
        v.sort();
        v.get(v.len() / 2).copied()
    };
    json!({
        "deployments": deps.len(),
        "deployment_frequency_per_day": if days > 0.0 { n / days } else { 0.0 },
        "change_failure_rate": if n > 0.0 { failed as f64 / n } else { 0.0 },
        "failed_deployments": failed,
        "lead_time_ms": median(leads),
        "time_to_restore_ms": median(restores),
        "failures": fails.len(),
    })
}

fn dep_of(r: &tokio_postgres::Row) -> Deployment {
    Deployment {
        service: r.get("service"),
        env: r.get("env"),
        version: r.get("version"),
        team: r.get("team"),
        commit_sha: r.get("commit_sha"),
        repository: r.get("repository"),
        started_ms: r.get("started_ms"),
        finished_ms: r.get("finished_ms"),
        status: r.get("status"),
        rollback: r.get("rollback"),
        source: r.get("source"),
        url: r.get("url"),
    }
}

/// `GET /api/ui/dora`: overall and per service. Lead time runs from the
/// commit's first CI pipeline to its deployment.
pub async fn dora_metrics(State(st): State<AppState>, Query(q): Query<Range>) -> ApiResult<Value> {
    let deps: Vec<Deployment> = deployments_in(&st, &q).await?.iter().map(dep_of).collect();
    let fails = failures(&st, &q).await?;
    let shas: Vec<String> = deps.iter().filter_map(|d| d.commit_sha.clone()).collect();
    let c = st.store.pg.conn().await.map_err(ApiError::from)?;
    let lead: HashMap<String, i64> = c
        .query(
            "SELECT commit_sha, min(start_ms) FROM ci_pipelines WHERE commit_sha = ANY($1) GROUP BY commit_sha",
            &[&shas],
        )
        .await
        .map_err(anyhow::Error::from)?
        .iter()
        .map(|r| (r.get(0), r.get(1)))
        .collect();
    let days = (q.to - q.from) as f64 / 86_400_000.0;
    let mut services: Vec<String> = deps.iter().map(|d| d.service.clone()).collect();
    services.sort();
    services.dedup();
    let per: Vec<Value> = services
        .iter()
        .map(|s| {
            let d: Vec<Deployment> = deps.iter().filter(|d| &d.service == s).cloned().collect();
            let f: Vec<Failure> = fails.iter().filter(|f| f.0.contains(s)).cloned().collect();
            let mut m = dora(&d, &f, &lead, days);
            m["service"] = json!(s);
            m["team"] = json!(d.iter().find_map(|x| x.team.clone()));
            m
        })
        .collect();
    let mut daily: HashMap<i64, i64> = HashMap::new();
    for d in &deps {
        *daily
            .entry(d.finished_ms.div_euclid(86_400_000) * 86_400_000)
            .or_default() += 1;
    }
    let mut daily: Vec<(i64, i64)> = daily.into_iter().collect();
    daily.sort();
    Ok(Json(
        json!({"overall": dora(&deps, &fails, &lead, days), "services": per, "daily": daily}),
    ))
}

/// `GET /api/ui/coverage`: the latest report per repository and branch, and
/// each repository's history.
pub async fn coverage(State(st): State<AppState>) -> ApiResult<Value> {
    let c = st.store.pg.conn().await.map_err(ApiError::from)?;
    let rows = c
        .query(
            "SELECT repository, branch, commit_sha, service, flags, format, covered, total, ts FROM coverage_reports
             ORDER BY ts DESC LIMIT 2000",
            &[],
        )
        .await
        .map_err(anyhow::Error::from)?;
    let data: Vec<Value> = rows
        .iter()
        .map(|r| {
            json!({"repository": r.get::<_, String>(0), "branch": r.get::<_, Option<String>>(1),
                "commit_sha": r.get::<_, Option<String>>(2), "service": r.get::<_, Option<String>>(3),
                "flags": r.get::<_, Option<String>>(4), "format": r.get::<_, String>(5),
                "covered": r.get::<_, i64>(6), "total": r.get::<_, i64>(7), "ts": r.get::<_, i64>(8)})
        })
        .collect();
    Ok(Json(json!({"data": data})))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coverage_formats() {
        assert_eq!(
            coverage_of(
                "lcov",
                "TN:\nSF:a.rs\nLF:10\nLH:7\nend_of_record\nSF:b\nLF:5\nLH:5\n"
            ),
            Some((12, 15))
        );
        assert_eq!(
            coverage_of(
                "cobertura",
                r#"<?xml?><coverage line-rate="0.5" lines-covered="4" lines-valid="8">"#
            ),
            Some((4, 8))
        );
        assert_eq!(
            coverage_of(
                "jacoco",
                r#"<report name="x"><package><counter type="LINE" missed="1" covered="1"/></package><counter type="LINE" missed="3" covered="9"/></report>"#
            ),
            Some((9, 12))
        );
    }

    #[test]
    fn deployment_sources() {
        let d = parse_dora_deployment(
            &json!({"data": {"attributes": {"service": "shop", "env": "prod",
            "started_at": 1_700_000_000_000_000_000i64, "finished_at": 1_700_000_060_000_000_000i64,
            "git": {"commit_sha": "abc", "repository_url": "https://g/x"}}}}),
        )
        .unwrap();
        assert_eq!(
            (d.started_ms, d.finished_ms),
            (1_700_000_000_000, 1_700_000_060_000)
        );
        let m = parse_custom_span(&json!({"data": {"attributes": {"start_time": "2026-01-01T00:00:00Z",
            "end_time": "2026-01-01T00:01:00Z", "tags": {"cd.env": "prod", "cd.service": "api", "cd.is_rollback": "true"}}}}))
        .unwrap();
        assert_eq!(
            (m.service.as_str(), m.env.as_str(), m.rollback),
            ("api", "prod", true)
        );
        let mut h = HeaderMap::new();
        h.insert("x-github-event", "deployment_status".parse().unwrap());
        let g = parse_webhook(&h, &json!({"deployment_status": {"state": "success", "updated_at": "2026-01-01T00:02:00Z"},
            "deployment": {"environment": "production", "sha": "abc", "created_at": "2026-01-01T00:00:00Z"},
            "repository": {"name": "shop", "html_url": "https://github.com/o/shop"}}))
        .unwrap();
        assert_eq!((g.service.as_str(), g.env.as_str()), ("shop", "production"));
        h.insert("x-github-event", "deployment_status".parse().unwrap());
        assert!(
            parse_webhook(&h, &json!({"deployment_status": {"state": "in_progress"}})).is_none()
        );
    }

    #[test]
    fn dora_rates() {
        let d = |t: i64, sha: &str| Deployment {
            service: "a".into(),
            finished_ms: t,
            commit_sha: Some(sha.into()),
            status: "success".into(),
            ..Default::default()
        };
        let deps = vec![d(1000, "x"), d(5000, "y")];
        let fails = vec![(vec!["a".to_string()], 2000, Some(2600))];
        let lead = HashMap::from([("x".to_string(), 400), ("y".to_string(), 4000)]);
        let m = dora(&deps, &fails, &lead, 2.0);
        assert_eq!(m["deployment_frequency_per_day"], 1.0);
        assert_eq!(m["change_failure_rate"], 0.5);
        assert_eq!(m["time_to_restore_ms"], 600);
        assert_eq!(m["lead_time_ms"], 1000);
    }
}
