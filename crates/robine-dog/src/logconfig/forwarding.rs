//! Log Forwarding: custom destinations (`/api/v2/logs/config/custom-destinations`)
//! receive the logs matching their query, after the pipelines, in batches:
//! generic HTTP (JSON array; basic auth or a custom header), Splunk HEC, or
//! Elasticsearch (`_bulk`).

use super::archives::to_line;
use crate::AppState;
use crate::api::{ApiError, ApiResult};
use crate::intake::logs::LogRow;
use crate::query::matcher::matches;
use crate::query::parser::Expr;
use crate::store::pg::new_id;
use axum::Json;
use axum::extract::{Path, State};
use serde_json::{Value, json};
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

const KIND: &str = "log_custom_destination";
const MAX_BUFFER: usize = 20_000;
const BATCH: usize = 500;

struct Compiled {
    id: String,
    filter: Expr,
    forward_tags: bool,
    dest: Value,
}

#[derive(Default, Clone, serde::Serialize)]
pub struct Status {
    pub sent: u64,
    pub dropped: u64,
    pub last_error: Option<String>,
    pub last_sent: Option<i64>,
}

#[derive(Default)]
pub struct Forwarding {
    list: RwLock<Arc<Vec<Compiled>>>,
    buffers: Mutex<HashMap<String, VecDeque<String>>>,
    status: Mutex<HashMap<String, Status>>,
}

impl Forwarding {
    fn load(&self, defs: &[(String, Value)]) {
        let list = defs
            .iter()
            .filter(|(_, d)| d["enabled"].as_bool() != Some(false))
            .filter_map(|(id, d)| {
                Some(Compiled {
                    id: id.clone(),
                    filter: crate::query::parser::parse(d["query"].as_str().unwrap_or("")).ok()?,
                    forward_tags: d["forward_tags"].as_bool().unwrap_or(true),
                    dest: d["forwarder_destination"].clone(),
                })
            })
            .collect();
        *self.list.write().unwrap_or_else(|e| e.into_inner()) = Arc::new(list);
    }

    /// Queues a log for every destination whose query matches it.
    pub fn add(&self, l: &LogRow) {
        let list = self.list.read().unwrap_or_else(|e| e.into_inner()).clone();
        if list.is_empty() {
            return;
        }
        let mut b = self.buffers.lock().unwrap_or_else(|e| e.into_inner());
        for d in list.iter().filter(|d| matches(&d.filter, l)) {
            let q = b.entry(d.id.clone()).or_default();
            if q.len() >= MAX_BUFFER {
                q.pop_front();
                self.status
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .entry(d.id.clone())
                    .or_default()
                    .dropped += 1;
            }
            q.push_back(to_line(l, d.forward_tags));
        }
    }

    async fn flush(&self, http: &reqwest::Client) {
        let list = self.list.read().unwrap_or_else(|e| e.into_inner()).clone();
        for d in list.iter() {
            loop {
                let batch: Vec<String> = {
                    let mut b = self.buffers.lock().unwrap_or_else(|e| e.into_inner());
                    let Some(q) = b.get_mut(&d.id) else { break };
                    let n = q.len().min(BATCH);
                    q.drain(..n).collect()
                };
                if batch.is_empty() {
                    break;
                }
                let r = send(http, &d.dest, &batch).await;
                let mut s = self.status.lock().unwrap_or_else(|e| e.into_inner());
                let s = s.entry(d.id.clone()).or_default();
                match r {
                    Ok(()) => {
                        s.sent += batch.len() as u64;
                        s.last_sent = Some(crate::intake::logs::now_ms());
                        s.last_error = None;
                    }
                    Err(e) => {
                        s.dropped += batch.len() as u64;
                        s.last_error = Some(e.to_string());
                        break;
                    }
                }
            }
        }
    }

    pub fn status(&self, id: &str) -> Status {
        self.status
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(id)
            .cloned()
            .unwrap_or_default()
    }
}

/// The request for one batch of JSON lines.
pub fn request(
    http: &reqwest::Client,
    dest: &Value,
    lines: &[String],
) -> anyhow::Result<reqwest::RequestBuilder> {
    let endpoint = dest["endpoint"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("no endpoint"))?
        .trim_end_matches('/');
    let auth = &dest["auth"];
    Ok(match dest["type"].as_str().unwrap_or("") {
        "http" => {
            let r = http
                .post(endpoint)
                .header("content-type", "application/json")
                .body(format!("[{}]", lines.join(",")));
            match auth["type"].as_str() {
                Some("basic") => r.basic_auth(
                    auth["username"].as_str().unwrap_or(""),
                    auth["password"].as_str(),
                ),
                Some("custom_header") => r.header(
                    auth["header_name"].as_str().unwrap_or("Authorization"),
                    auth["header_value"].as_str().unwrap_or(""),
                ),
                _ => r,
            }
        }
        "splunk_hec" => {
            let body: Vec<String> = lines
                .iter()
                .map(|l| {
                    let v: Value = serde_json::from_str(l).unwrap_or(Value::Null);
                    let ts = chrono::DateTime::parse_from_rfc3339(v["date"].as_str().unwrap_or(""))
                        .map(|d| d.timestamp_millis() as f64 / 1000.0)
                        .unwrap_or(0.0);
                    json!({"time": ts, "host": v["host"], "source": v["service"], "sourcetype": v["source"], "event": v}).to_string()
                })
                .collect();
            let url = if endpoint.contains("/services/collector") {
                endpoint.to_string()
            } else {
                format!("{endpoint}/services/collector/event")
            };
            http.post(url)
                .header(
                    "Authorization",
                    format!("Splunk {}", dest["access_token"].as_str().unwrap_or("")),
                )
                .body(body.join("\n"))
        }
        "elasticsearch" => {
            let name = dest["index_name"].as_str().unwrap_or("datadog-logs");
            let index = match dest["index_rotation"].as_str() {
                Some(f) if !f.is_empty() => {
                    let f = f
                        .replace("yyyy", "%Y")
                        .replace("MM", "%m")
                        .replace("dd", "%d")
                        .replace("HH", "%H");
                    format!("{name}-{}", chrono::Utc::now().format(&f))
                }
                _ => name.to_string(),
            };
            let mut body = String::new();
            for l in lines {
                body.push_str(&json!({"index": {"_index": index}}).to_string());
                body.push('\n');
                body.push_str(l);
                body.push('\n');
            }
            let r = http
                .post(format!("{endpoint}/_bulk"))
                .header("content-type", "application/x-ndjson")
                .body(body);
            match auth["username"].as_str() {
                Some(u) => r.basic_auth(u, auth["password"].as_str()),
                None => r,
            }
        }
        t => anyhow::bail!("unknown destination type {t:?}"),
    })
}

async fn send(http: &reqwest::Client, dest: &Value, lines: &[String]) -> anyhow::Result<()> {
    let res = request(http, dest, lines)?.send().await?;
    let status = res.status();
    let text = res.text().await.unwrap_or_default();
    anyhow::ensure!(
        status.is_success(),
        "{status}: {}",
        text.chars().take(200).collect::<String>()
    );
    // Elasticsearch answers 200 with per-document errors.
    anyhow::ensure!(
        !text.contains("\"errors\":true"),
        "Elasticsearch rejected documents: {}",
        text.chars().take(200).collect::<String>()
    );
    Ok(())
}

pub fn spawn(st: AppState) {
    tokio::spawn(async move {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .unwrap_or_default();
        loop {
            tokio::time::sleep(Duration::from_secs(5)).await;
            st.logconfig.forwarding.flush(&http).await;
        }
    });
}

pub async fn reload(st: &AppState) -> anyhow::Result<()> {
    let defs: Vec<(String, Value)> = st
        .store
        .pg
        .docs(KIND)
        .await?
        .into_iter()
        .map(|d| (d.id, d.body))
        .collect();
    st.logconfig.forwarding.load(&defs);
    Ok(())
}

/// Secrets are never returned.
fn view(st: &AppState, id: &str, b: &Value) -> Value {
    let mut a = b.clone();
    let d = &mut a["forwarder_destination"];
    for k in ["access_token", "password", "header_value"] {
        if let Some(o) = d.as_object_mut() {
            o.remove(k);
        }
        if let Some(o) = d["auth"].as_object_mut() {
            o.remove(k);
        }
    }
    a["status"] = json!(st.logconfig.forwarding.status(id));
    json!({"id": id, "type": "custom_destination", "attributes": a})
}

fn check(a: &Value) -> Result<(), ApiError> {
    let bad = |m: &str| Err(ApiError::BadQuery(m.into()));
    if a["name"].as_str().unwrap_or("").is_empty() {
        return bad("attributes.name is required");
    }
    let d = &a["forwarder_destination"];
    if !matches!(
        d["type"].as_str(),
        Some("http" | "splunk_hec" | "elasticsearch")
    ) {
        return bad("forwarder_destination.type: http, splunk_hec or elasticsearch");
    }
    if !d["endpoint"]
        .as_str()
        .is_some_and(|e| e.starts_with("http://") || e.starts_with("https://"))
    {
        return bad("forwarder_destination.endpoint: an http(s) URL");
    }
    crate::query::parser::parse(a["query"].as_str().unwrap_or(""))
        .map_err(|e| ApiError::BadQuery(format!("query: {e:?}")))?;
    Ok(())
}

pub async fn list(State(st): State<AppState>) -> ApiResult<Value> {
    let docs = st.store.pg.docs(KIND).await?;
    Ok(Json(
        json!({"data": docs.iter().map(|d| view(&st, &d.id, &d.body)).collect::<Vec<_>>()}),
    ))
}

pub async fn get(State(st): State<AppState>, Path(id): Path<String>) -> ApiResult<Value> {
    let d = st
        .store
        .pg
        .doc(KIND, &id)
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(json!({"data": view(&st, &id, &d.body)})))
}

async fn put(st: &AppState, id: &str, mut a: Value) -> ApiResult<Value> {
    if let Some(old) = st.store.pg.doc(KIND, id).await? {
        // A PATCH without secrets keeps the stored ones.
        for (path, k) in [
            ("", "access_token"),
            ("auth", "password"),
            ("auth", "header_value"),
        ] {
            let (new, prev) = if path.is_empty() {
                (
                    &a["forwarder_destination"][k],
                    &old.body["forwarder_destination"][k],
                )
            } else {
                (
                    &a["forwarder_destination"][path][k],
                    &old.body["forwarder_destination"][path][k],
                )
            };
            if new.is_null() && !prev.is_null() {
                let prev = prev.clone();
                if path.is_empty() {
                    a["forwarder_destination"][k] = prev;
                } else {
                    a["forwarder_destination"][path][k] = prev;
                }
            }
        }
        for (k, v) in old.body.as_object().cloned().unwrap_or_default() {
            if a.get(&k).is_none() {
                a[k] = v;
            }
        }
    }
    if a["enabled"].is_null() {
        a["enabled"] = json!(true);
    }
    if a["query"].is_null() {
        a["query"] = json!("");
    }
    check(&a)?;
    st.store.pg.put_doc(KIND, id, &a).await?;
    reload(st).await?;
    Ok(Json(json!({"data": view(st, id, &a)})))
}

pub async fn create(State(st): State<AppState>, Json(v): Json<Value>) -> ApiResult<Value> {
    put(&st, &new_id(), v["data"]["attributes"].clone()).await
}

pub async fn update(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Json(v): Json<Value>,
) -> ApiResult<Value> {
    st.store
        .pg
        .doc(KIND, &id)
        .await?
        .ok_or(ApiError::NotFound)?;
    put(&st, &id, v["data"]["attributes"].clone()).await
}

pub async fn delete(State(st): State<AppState>, Path(id): Path<String>) -> ApiResult<Value> {
    st.store.pg.delete_doc(KIND, &id).await?;
    reload(&st).await?;
    Ok(Json(json!({})))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_per_destination() {
        let http = reqwest::Client::new();
        let lines = vec![r#"{"date":"2026-01-01T00:00:00.000Z","host":"h","service":"s","source":"nginx","message":"hi"}"#.to_string()];
        let r = request(
            &http,
            &json!({"type": "splunk_hec", "endpoint": "https://splunk:8088", "access_token": "t"}),
            &lines,
        )
        .unwrap()
        .build()
        .unwrap();
        assert_eq!(
            r.url().as_str(),
            "https://splunk:8088/services/collector/event"
        );
        assert_eq!(r.headers()["authorization"], "Splunk t");
        let r = request(
            &http,
            &json!({"type": "elasticsearch", "endpoint": "http://es:9200", "index_name": "logs"}),
            &lines,
        )
        .unwrap()
        .build()
        .unwrap();
        assert_eq!(r.url().path(), "/_bulk");
        let body = String::from_utf8(r.body().unwrap().as_bytes().unwrap().to_vec()).unwrap();
        assert!(body.starts_with(r#"{"index":{"_index":"logs"}}"#));
        assert!(
            request(
                &http,
                &json!({"type": "kafka", "endpoint": "http://x"}),
                &lines
            )
            .is_err()
        );
    }
}
