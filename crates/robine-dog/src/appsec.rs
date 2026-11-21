//! App & API Protection and Code Security (SCA).
//!
//! Tracers run with `DD_APPSEC_ENABLED=true` tag the spans of attacked
//! requests with `_dd.appsec.json` (the WAF's triggered rules): each trigger
//! becomes a `source:appsec` log, so detection rules, the Log Explorer and
//! monitors work on attacks. The API inventory is the `http.route` of service
//! entry spans.
//!
//! Tracers' telemetry (`/telemetry/proxy`, `app-dependencies-loaded`) lists the
//! libraries each service loads; they are checked against OSV (osv.dev, open
//! source vulnerability database) once a day. `ROBINE_DOG_OSV_URL=` (empty)
//! turns the lookup off.

use crate::AppState;
use crate::api::{ApiError, ApiResult};
use crate::intake::logs::LogRow;
use crate::intake::traces::SpanRow;
use crate::store::greptime::{i64_at, str_at};
use axum::Json;
use axum::body::Bytes;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use serde::Deserialize;
use serde_json::{Map, Value, json};
use std::time::Duration;

fn meta<'a>(s: &'a SpanRow, k: &str) -> Option<&'a str> {
    s.meta.get(k).and_then(|v| v.as_str())
}

/// The attacks recorded on a span, as logs.
pub fn attacks(s: &SpanRow) -> Vec<LogRow> {
    let Some(raw) = s.meta.get("_dd.appsec.json") else {
        return Vec::new();
    };
    let v: Value = match raw {
        Value::String(x) => serde_json::from_str(x).unwrap_or(Value::Null),
        other => other.clone(),
    };
    let ip = meta(s, "http.client_ip")
        .or(meta(s, "network.client.ip"))
        .or(meta(s, "actor.ip"));
    let method = meta(s, "http.method").unwrap_or("");
    let route = meta(s, "http.route")
        .or(meta(s, "http.url"))
        .unwrap_or(&s.resource);
    let blocked = meta(s, "appsec.blocked") == Some("true");
    v["triggers"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|t| {
            let rule = &t["rule"];
            let name = rule["name"].as_str().unwrap_or("WAF rule");
            let typ = rule["tags"]["type"].as_str().unwrap_or("unknown");
            let param = t["rule_matches"][0]["parameters"][0].clone();
            let mut a = Map::new();
            a.insert(
                "appsec".into(),
                json!({"rule": {"id": rule["id"], "name": name}, "type": typ,
                    "category": rule["tags"]["category"], "blocked": blocked,
                    "address": param["address"], "value": param["value"]}),
            );
            a.insert(
                "http".into(),
                json!({"method": method, "route": meta(s, "http.route"), "url": meta(s, "http.url"),
                    "status_code": meta(s, "http.status_code"), "useragent": meta(s, "http.useragent")}),
            );
            if let Some(ip) = ip {
                a.insert("network".into(), json!({"client": {"ip": ip}}));
            }
            if let Some(u) = meta(s, "usr.id") {
                a.insert("usr".into(), json!({"id": u}));
            }
            LogRow {
                ts: s.start / 1_000_000,
                status: if blocked { "info" } else { "warn" }.into(),
                service: Some(s.service.clone()),
                host: meta(s, "_dd.hostname").map(String::from),
                source: Some("appsec".into()),
                message: format!("{name} ({typ}) on {method} {route}"),
                tags: s.env.iter().map(|e| format!("env:{e}")).collect(),
                attributes: a,
                trace_id: Some(s.trace_id.clone()),
                span_id: Some(s.span_id.clone()),
            }
        })
        .collect()
}

#[derive(Debug, Deserialize)]
pub struct Range {
    pub from: i64,
    pub to: i64,
}

/// `GET /api/ui/appsec/endpoints`: the API inventory.
pub async fn endpoints(State(st): State<AppState>, Query(r): Query<Range>) -> ApiResult<Value> {
    let route = "json_get_string(meta, '$[\"http.route\"]')";
    let method = "json_get_string(meta, '$[\"http.method\"]')";
    let sql = format!(
        "SELECT service, {method} AS m, {route} AS r, count(*) AS n, sum(CASE WHEN error THEN 1 ELSE 0 END) AS e,
                CAST(max(ts) AS BIGINT) AS last, avg(duration) AS d
         FROM spans WHERE {time} AND top_level AND {route} IS NOT NULL
         GROUP BY service, m, r ORDER BY n DESC LIMIT 2000",
        time = crate::api::time_clause(crate::query::sql::Dataset::Spans, r.from, r.to),
    );
    let rows = st.store.gt.query(&sql).await?;
    let data: Vec<Value> = rows
        .iter()
        .map(|x| {
            json!({"service": str_at(x, 0), "method": str_at(x, 1), "route": str_at(x, 2),
                "requests": i64_at(x, 3), "errors": i64_at(x, 4), "last_seen": i64_at(x, 5) / 1_000_000,
                "avg_duration": x.get(6).and_then(|v| v.as_f64())})
        })
        .collect();
    Ok(Json(json!({"data": data})))
}

// ---------- Code Security (SCA) ----------

fn ecosystem(language: &str) -> Option<&'static str> {
    Some(match language.to_ascii_lowercase().as_str() {
        "python" => "PyPI",
        "node" | "nodejs" | "javascript" => "npm",
        "java" | "jvm" => "Maven",
        "ruby" => "RubyGems",
        "go" | "golang" => "Go",
        "dotnet" | ".net" => "NuGet",
        "php" => "Packagist",
        _ => return None,
    })
}

/// (service, env, language, [(name, version)]) from a telemetry payload,
/// including the messages of a `message-batch`.
/// (service, env, language, [(library, version)]).
type Loaded = (String, String, String, Vec<(String, String)>);

pub fn dependencies(v: &Value) -> Option<Loaded> {
    let app = &v["application"];
    let service = app["service_name"].as_str()?.to_string();
    let env = app["env"].as_str().unwrap_or("").to_string();
    let lang = app["language_name"].as_str().unwrap_or("").to_string();
    let mut out = Vec::new();
    let mut take = |kind: &str, p: &Value| {
        if kind == "app-dependencies-loaded" || kind == "app-started" {
            for d in p["dependencies"].as_array().into_iter().flatten() {
                if let (Some(n), Some(ver)) = (d["name"].as_str(), d["version"].as_str()) {
                    out.push((n.to_string(), ver.to_string()));
                }
            }
        }
    };
    let kind = v["request_type"].as_str().unwrap_or("");
    if kind == "message-batch" {
        for m in v["payload"].as_array().into_iter().flatten() {
            take(m["request_type"].as_str().unwrap_or(""), &m["payload"]);
        }
    } else {
        take(kind, &v["payload"]);
    }
    (!out.is_empty()).then_some((service, env, lang, out))
}

/// `/telemetry/proxy/api/v2/apmtelemetry` on the trace port.
pub async fn telemetry(State(st): State<AppState>, body: Bytes) -> StatusCode {
    let Ok(v) = serde_json::from_slice::<Value>(&body) else {
        return StatusCode::ACCEPTED;
    };
    if let Some((service, env, lang, deps)) = dependencies(&v) {
        let r = async {
            let c = st.store.pg.conn().await?;
            let now = crate::intake::logs::now_ms();
            for (n, ver) in deps {
                c.execute(
                    "INSERT INTO sca_dependencies (service, env, language, name, version, last_seen) VALUES ($1,$2,$3,$4,$5,$6)
                     ON CONFLICT (service, env, name, version) DO UPDATE SET last_seen = EXCLUDED.last_seen",
                    &[&service, &env, &lang, &n, &ver, &now],
                )
                .await?;
            }
            anyhow::Ok(())
        }
        .await;
        if let Err(e) = r {
            tracing::warn!(error = %e, "telemetry dependencies");
        }
    }
    StatusCode::ACCEPTED
}

fn osv_url() -> Option<String> {
    match std::env::var("ROBINE_DOG_OSV_URL") {
        Ok(u) if u.is_empty() => None,
        Ok(u) => Some(u.trim_end_matches('/').to_string()),
        Err(_) => Some("https://api.osv.dev".into()),
    }
}

/// Packages not checked for a day, against OSV.
async fn check(st: &AppState, base: &str) -> anyhow::Result<usize> {
    let c = st.store.pg.conn().await?;
    let now = crate::intake::logs::now_ms();
    let rows = c
        .query(
            "SELECT DISTINCT d.language, d.name, d.version FROM sca_dependencies d
             LEFT JOIN sca_packages p ON p.language = d.language AND p.name = d.name AND p.version = d.version
             WHERE p.checked_at IS NULL OR p.checked_at < $1 LIMIT 1000",
            &[&(now - 86_400_000)],
        )
        .await?;
    let pkgs: Vec<(String, String, String, &str)> = rows
        .iter()
        .filter_map(|r| {
            let lang: String = r.get(0);
            let eco = ecosystem(&lang)?;
            Some((lang, r.get(1), r.get(2), eco))
        })
        .collect();
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()?;
    for chunk in pkgs.chunks(500) {
        let queries: Vec<Value> = chunk
            .iter()
            .map(|(_, n, v, eco)| json!({"package": {"name": n, "ecosystem": eco}, "version": v}))
            .collect();
        let res: Value = http
            .post(format!("{base}/v1/querybatch"))
            .json(&json!({"queries": queries}))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        for (i, (lang, n, v, _)) in chunk.iter().enumerate() {
            let ids: Vec<String> = res["results"][i]["vulns"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|x| x["id"].as_str().map(String::from))
                .collect();
            for id in &ids {
                let known = c
                    .query_opt("SELECT 1 FROM sca_vulns WHERE id = $1", &[id])
                    .await?
                    .is_some();
                if known {
                    continue;
                }
                let d: Value = http
                    .get(format!("{base}/v1/vulns/{id}"))
                    .send()
                    .await?
                    .json()
                    .await?;
                let severity = d["database_specific"]["severity"]
                    .as_str()
                    .map(String::from)
                    .or_else(|| d["severity"][0]["score"].as_str().map(String::from));
                let fixed: Vec<String> = d["affected"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .flat_map(|a| a["ranges"].as_array().cloned().unwrap_or_default())
                    .flat_map(|r| r["events"].as_array().cloned().unwrap_or_default())
                    .filter_map(|e| e["fixed"].as_str().map(String::from))
                    .collect();
                c.execute(
                    "INSERT INTO sca_vulns (id, summary, severity, aliases, fixed) VALUES ($1,$2,$3,$4,$5) ON CONFLICT (id) DO NOTHING",
                    &[
                        id,
                        &d["summary"].as_str().or(d["details"].as_str()).map(|s| s.chars().take(500).collect::<String>()),
                        &severity,
                        &d["aliases"].as_array().map(|a| a.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join(", ")),
                        &fixed.join(", "),
                    ],
                )
                .await?;
            }
            c.execute(
                "INSERT INTO sca_packages (language, name, version, checked_at, vulns) VALUES ($1,$2,$3,$4,$5)
                 ON CONFLICT (language, name, version) DO UPDATE SET checked_at = EXCLUDED.checked_at, vulns = EXCLUDED.vulns",
                &[lang, n, v, &now, &ids],
            )
            .await?;
        }
    }
    Ok(pkgs.len())
}

pub fn spawn(st: AppState) {
    let Some(base) = osv_url() else {
        return;
    };
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(60)).await;
            match check(&st, &base).await {
                Ok(n) if n > 0 => tracing::info!(packages = n, "checked against OSV"),
                Ok(_) => tokio::time::sleep(Duration::from_secs(3540)).await,
                Err(e) => {
                    tracing::warn!(error = %e, "OSV lookup");
                    tokio::time::sleep(Duration::from_secs(3540)).await;
                }
            }
        }
    });
}

/// `GET /api/ui/code-security`: vulnerable libraries by service.
pub async fn vulnerabilities(State(st): State<AppState>) -> ApiResult<Value> {
    let c = st.store.pg.conn().await.map_err(ApiError::from)?;
    let q = |sql: &'static str| {
        let c = &c;
        async move { c.query(sql, &[]).await.map_err(anyhow::Error::from) }
    };
    let vulns = q(
        "SELECT d.service, d.env, d.language, d.name, d.version, v.id, v.summary, v.severity, v.aliases, v.fixed, d.last_seen
         FROM sca_dependencies d
         JOIN sca_packages p ON p.language = d.language AND p.name = d.name AND p.version = d.version
         JOIN sca_vulns v ON v.id = ANY(p.vulns)
         ORDER BY d.service, d.name LIMIT 5000",
    )
    .await?;
    let stats = q(
        "SELECT count(DISTINCT service), count(DISTINCT (language, name, version)),
                (SELECT count(*) FROM sca_packages) FROM sca_dependencies",
    )
    .await?;
    let s = stats.first();
    Ok(Json(json!({
        "services": s.map(|r| r.get::<_, i64>(0)).unwrap_or(0),
        "libraries": s.map(|r| r.get::<_, i64>(1)).unwrap_or(0),
        "checked": s.map(|r| r.get::<_, i64>(2)).unwrap_or(0),
        "osv": osv_url().is_some(),
        "data": vulns.iter().map(|r| json!({
            "service": r.get::<_, String>(0), "env": r.get::<_, String>(1), "language": r.get::<_, String>(2),
            "library": r.get::<_, String>(3), "version": r.get::<_, String>(4), "id": r.get::<_, String>(5),
            "summary": r.get::<_, Option<String>>(6), "severity": r.get::<_, Option<String>>(7),
            "aliases": r.get::<_, Option<String>>(8), "fixed": r.get::<_, Option<String>>(9),
            "last_seen": r.get::<_, i64>(10),
        })).collect::<Vec<_>>(),
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appsec_triggers_become_logs() {
        let mut meta = Map::new();
        meta.insert("_dd.appsec.json".into(), json!(r#"{"triggers":[{"rule":{"id":"crs-942-100","name":"SQL Injection","tags":{"type":"sql_injection","category":"attack_attempt"}},"rule_matches":[{"parameters":[{"address":"server.request.query","value":"1' OR 1=1"}]}]}]}"#));
        meta.insert("http.client_ip".into(), json!("1.2.3.4"));
        meta.insert("http.method".into(), json!("GET"));
        meta.insert("http.route".into(), json!("/search"));
        let s = SpanRow {
            trace_id: "1".into(),
            span_id: "2".into(),
            parent_id: "0".into(),
            service: "shop".into(),
            name: "web.request".into(),
            resource: "GET /search".into(),
            typ: None,
            env: Some("prod".into()),
            version: None,
            start: 1_000_000_000,
            duration: 1,
            error: false,
            top_level: true,
            meta,
            metrics: Map::new(),
        };
        let l = attacks(&s);
        assert_eq!(l.len(), 1);
        assert_eq!(l[0].message, "SQL Injection (sql_injection) on GET /search");
        assert_eq!(l[0].attributes["network"]["client"]["ip"], "1.2.3.4");
        assert_eq!(l[0].ts, 1000);
    }

    #[test]
    fn telemetry_dependencies() {
        let v = json!({"request_type": "message-batch", "application": {"service_name": "shop", "env": "prod", "language_name": "python"},
            "payload": [{"request_type": "app-heartbeat", "payload": {}},
                {"request_type": "app-dependencies-loaded", "payload": {"dependencies": [{"name": "django", "version": "3.2.0"}]}}]});
        let (s, _, l, d) = dependencies(&v).unwrap();
        assert_eq!((s.as_str(), l.as_str()), ("shop", "python"));
        assert_eq!(d, vec![("django".to_string(), "3.2.0".to_string())]);
        assert_eq!(ecosystem("python"), Some("PyPI"));
    }
}
