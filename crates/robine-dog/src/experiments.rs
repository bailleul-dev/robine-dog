//! LLM Observability Experiments (the ddtrace SDK's projects, datasets and
//! experiments API, `/api/{unstable,v2}/llm-obs/v1/...`, direct or through the
//! Agent's EVP proxy) and Agent Observability (agent runs, their tool and LLM
//! calls).

use crate::AppState;
use crate::api::ApiResult;
use crate::store::pg::new_id;
use axum::Json;
use axum::body::Bytes;
use axum::extract::{OriginalUri, Query, State};
use axum::http::{Method, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;

const PROJECT: &str = "llm_project";
const DATASET: &str = "llm_dataset";
const EXPERIMENT: &str = "llm_experiment";

fn ok(v: Value) -> Response {
    (StatusCode::OK, Json(v)).into_response()
}

fn not_found() -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(json!({"errors": ["not found"]})),
    )
        .into_response()
}

fn query_map(q: &str) -> HashMap<String, String> {
    url::form_urlencoded::parse(q.as_bytes())
        .into_owned()
        .collect()
}

fn resource(typ: &str, id: &str, b: &Value) -> Value {
    let mut a = b.clone();
    if let Some(o) = a.as_object_mut() {
        o.remove("records");
        o.remove("spans");
        o.remove("metrics");
    }
    json!({"id": id, "type": typ, "attributes": a})
}

async fn find(
    st: &AppState,
    kind: &str,
    f: impl Fn(&Value) -> bool,
) -> anyhow::Result<Vec<(String, Value)>> {
    Ok(st
        .store
        .pg
        .docs(kind)
        .await?
        .into_iter()
        .filter(|d| f(&d.body))
        .map(|d| (d.id, d.body))
        .collect())
}

/// The SDK's API, after `llm-obs/v1/`.
pub async fn handle(
    st: &AppState,
    method: &Method,
    path: &str,
    query: &str,
    body: &[u8],
) -> anyhow::Result<Response> {
    let q = query_map(query);
    let v: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
    let attrs = &v["data"]["attributes"];
    let seg: Vec<&str> = path.trim_matches('/').split('/').collect();
    let now = crate::intake::logs::now_ms();
    let pg = &st.store.pg;
    Ok(match (method.as_str(), seg.as_slice()) {
        ("POST", ["projects"]) => {
            let name = attrs["name"].as_str().unwrap_or("default").to_string();
            let found = find(st, PROJECT, |b| b["name"] == name.as_str()).await?;
            let (id, b) = match found.into_iter().next() {
                Some(x) => x,
                None => {
                    let id = new_id();
                    let b = json!({"name": name, "description": attrs["description"], "created_at": now});
                    pg.put_doc(PROJECT, &id, &b).await?;
                    (id, b)
                }
            };
            ok(json!({"data": resource("projects", &id, &b)}))
        }
        ("GET", ["projects"]) => {
            let name = q.get("filter[name]");
            let l = find(st, PROJECT, |b| {
                name.is_none_or(|n| b["name"] == n.as_str())
            })
            .await?;
            ok(
                json!({"data": l.iter().map(|(i, b)| resource("projects", i, b)).collect::<Vec<_>>()}),
            )
        }
        ("POST", [project, "datasets"]) => {
            let id = new_id();
            let b = json!({"name": attrs["name"], "description": attrs["description"], "project_id": project,
                "current_version": 0, "records": [], "created_at": now});
            pg.put_doc(DATASET, &id, &b).await?;
            ok(json!({"data": resource("datasets", &id, &b)}))
        }
        ("GET", [project, "datasets"]) => {
            let name = q.get("filter[name]");
            let l = find(st, DATASET, |b| {
                b["project_id"] == *project && name.is_none_or(|n| b["name"] == n.as_str())
            })
            .await?;
            ok(
                json!({"data": l.iter().map(|(i, b)| resource("datasets", i, b)).collect::<Vec<_>>()}),
            )
        }
        ("POST", ["datasets", "delete"]) => {
            for id in attrs["dataset_ids"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|x| x.as_str())
            {
                pg.delete_doc(DATASET, id).await?;
            }
            ok(json!({"data": []}))
        }
        ("POST", [_, "datasets", id, "batch_update"]) => {
            let Some(d) = pg.doc(DATASET, id).await? else {
                return Ok(not_found());
            };
            let mut b = d.body;
            let mut records: Vec<Value> = b["records"].as_array().cloned().unwrap_or_default();
            let changed = attrs["insert_records"]
                .as_array()
                .is_some_and(|l| !l.is_empty())
                || attrs["update_records"]
                    .as_array()
                    .is_some_and(|l| !l.is_empty())
                || attrs["delete_records"]
                    .as_array()
                    .is_some_and(|l| !l.is_empty());
            let version = b["current_version"].as_i64().unwrap_or(0)
                + i64::from(changed && attrs["create_new_version"].as_bool() != Some(false));
            let deleted: Vec<&str> = attrs["delete_records"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|x| x.as_str())
                .collect();
            records.retain(|r| !deleted.contains(&r["id"].as_str().unwrap_or("")));
            let mut out = Vec::new();
            for u in attrs["update_records"].as_array().into_iter().flatten() {
                if let Some(r) = records.iter_mut().find(|r| r["id"] == u["id"]) {
                    for k in ["input", "expected_output", "metadata"] {
                        if !u[k].is_null() {
                            r[k] = u[k].clone();
                        }
                    }
                    if let Some(set) = u["tag_operations"]["set"].as_array() {
                        r["tags"] = json!(set);
                    }
                    r["version"] = json!(version);
                    out.push(json!({"id": r["id"], "type": "records", "attributes": {"version": version, "canonical_id": r["id"]}}));
                }
            }
            for i in attrs["insert_records"].as_array().into_iter().flatten() {
                let id = i["id"].as_str().map(String::from).unwrap_or_else(new_id);
                records.push(
                    json!({"id": id, "input": i["input"], "expected_output": i["expected_output"],
                    "metadata": i["metadata"], "tags": i["tags"], "version": version}),
                );
                out.push(json!({"id": id, "type": "records", "attributes": {"version": version, "canonical_id": id}}));
            }
            b["records"] = json!(records);
            b["current_version"] = json!(version);
            pg.put_doc(DATASET, id, &b).await?;
            ok(json!({"data": out}))
        }
        ("GET", [_, "datasets", id, "records"]) => {
            let Some(d) = pg.doc(DATASET, id).await? else {
                return Ok(not_found());
            };
            let data: Vec<Value> = d.body["records"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|r| json!({"id": r["id"], "type": "records", "attributes": {"canonical_id": r["id"],
                    "input": r["input"], "expected_output": r["expected_output"], "metadata": r["metadata"], "tags": r["tags"]}}))
                .collect();
            ok(json!({"data": data, "meta": {"after": null}}))
        }
        ("POST", ["experiments"]) => {
            let id = new_id();
            let mut b = attrs.clone();
            b["created_at"] = json!(now);
            b["status"] = json!("running");
            b["spans"] = json!([]);
            b["metrics"] = json!([]);
            pg.put_doc(EXPERIMENT, &id, &b).await?;
            ok(json!({"data": resource("experiments", &id, &b)}))
        }
        ("GET", ["experiments"]) => {
            let id = q.get("filter[id]");
            let project = q.get("filter[project_id]");
            let dataset = q.get("filter[dataset_id]");
            let mut l = find(st, EXPERIMENT, |b| {
                project.is_none_or(|p| b["project_id"] == p.as_str())
                    && dataset.is_none_or(|d| b["dataset_id"] == d.as_str())
            })
            .await?;
            if let Some(id) = id {
                l.retain(|(i, _)| i == id);
            }
            ok(json!({"data": l.iter().map(|(i, b)| {
                let mut r = resource("experiments", i, b);
                r["attributes"]["experiment"] = b["name"].clone();
                r
            }).collect::<Vec<_>>(), "meta": {"after": null}}))
        }
        ("PATCH", ["experiments", id]) => {
            let Some(d) = pg.doc(EXPERIMENT, id).await? else {
                return Ok(not_found());
            };
            let mut b = d.body;
            for k in ["status", "error"] {
                if !attrs[k].is_null() {
                    b[k] = attrs[k].clone();
                }
            }
            pg.put_doc(EXPERIMENT, id, &b).await?;
            ok(json!({"data": resource("experiments", id, &b)}))
        }
        ("POST", ["experiments", id, "events"]) => {
            let Some(d) = pg.doc(EXPERIMENT, id).await? else {
                return Ok(not_found());
            };
            let mut b = d.body;
            for k in ["spans", "metrics"] {
                let mut l = b[k].as_array().cloned().unwrap_or_default();
                l.extend(attrs[k].as_array().cloned().unwrap_or_default());
                b[k] = json!(l);
            }
            pg.put_doc(EXPERIMENT, id, &b).await?;
            (StatusCode::ACCEPTED, Json(json!({}))).into_response()
        }
        ("GET", ["experiments", id, "events"]) => {
            let Some(d) = pg.doc(EXPERIMENT, id).await? else {
                return Ok(not_found());
            };
            ok(json!({"data": {"id": id, "type": "experiment_events",
                "attributes": {"spans": d.body["spans"], "eval_metrics": d.body["metrics"], "summary_metrics": summary(&d.body)}}}))
        }
        ("PUT", ["config", "evaluators", "custom"]) => ok(json!({"data": {}})),
        _ => not_found(),
    })
}

/// `/api/unstable/llm-obs/v1/*` and `/api/v2/llm-obs/v1/*`.
pub async fn api(
    State(st): State<AppState>,
    method: Method,
    OriginalUri(uri): OriginalUri,
    body: Bytes,
) -> Response {
    let path = uri.path();
    let rest = path.split_once("llm-obs/v1/").map(|(_, r)| r).unwrap_or("");
    match handle(&st, &method, rest, uri.query().unwrap_or(""), &body).await {
        Ok(r) => r,
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"errors": [e.to_string()]})),
        )
            .into_response(),
    }
}

/// A metric's value: its score, boolean or category.
fn metric_value(m: &Value) -> Value {
    for k in ["score_value", "boolean_value", "categorical_value", "value"] {
        if !m[k].is_null() {
            return m[k].clone();
        }
    }
    Value::Null
}

/// Per evaluation label: the mean of numbers and booleans, else the most
/// frequent category.
pub fn summary(b: &Value) -> Value {
    let mut by: HashMap<String, Vec<Value>> = HashMap::new();
    for m in b["metrics"].as_array().into_iter().flatten() {
        let label = m["label"]
            .as_str()
            .or(m["metric_name"].as_str())
            .unwrap_or("metric")
            .to_string();
        by.entry(label).or_default().push(metric_value(m));
    }
    let mut out = serde_json::Map::new();
    for (label, vals) in by {
        let nums: Vec<f64> = vals
            .iter()
            .filter_map(|v| {
                v.as_f64()
                    .or_else(|| v.as_bool().map(|b| f64::from(u8::from(b))))
            })
            .collect();
        let v = if !nums.is_empty() {
            json!(nums.iter().sum::<f64>() / nums.len() as f64)
        } else {
            let mut c: HashMap<String, usize> = HashMap::new();
            for v in &vals {
                *c.entry(v.as_str().unwrap_or("").to_string()).or_default() += 1;
            }
            json!(c.into_iter().max_by_key(|x| x.1).map(|x| x.0))
        };
        out.insert(label, v);
    }
    Value::Object(out)
}

// ---------- UI ----------

/// `GET /api/ui/llm-obs/experiments`.
pub async fn list(State(st): State<AppState>) -> ApiResult<Value> {
    let projects: HashMap<String, Value> = st
        .store
        .pg
        .docs(PROJECT)
        .await?
        .into_iter()
        .map(|d| (d.id, d.body))
        .collect();
    let datasets: HashMap<String, Value> = st
        .store
        .pg
        .docs(DATASET)
        .await?
        .into_iter()
        .map(|d| (d.id, d.body))
        .collect();
    let mut l: Vec<Value> = st
        .store
        .pg
        .docs(EXPERIMENT)
        .await?
        .into_iter()
        .map(|d| {
            let b = &d.body;
            let spans = b["spans"].as_array().map(|s| s.len()).unwrap_or(0);
            let errors = b["spans"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|s| s["status"] == "error" || !s["meta"]["error"].is_null() && s["meta"]["error"] != "")
                .count();
            let ds = b["dataset_id"].as_str().unwrap_or("");
            json!({"id": d.id, "name": b["name"], "description": b["description"], "status": b["status"],
                "project": projects.get(b["project_id"].as_str().unwrap_or("")).map(|p| p["name"].clone()),
                "dataset_id": ds, "dataset": datasets.get(ds).map(|x| x["name"].clone()),
                "dataset_version": b["dataset_version"], "config": b["config"], "created_at": b["created_at"],
                "spans": spans, "errors": errors, "summary": summary(b)})
        })
        .collect();
    l.sort_by_key(|x| std::cmp::Reverse(x["created_at"].as_i64().unwrap_or(0)));
    let ds: Vec<Value> = datasets
        .iter()
        .map(|(id, d)| {
            json!({"id": id, "name": d["name"], "version": d["current_version"],
            "records": d["records"].as_array().map(|r| r.len()).unwrap_or(0)})
        })
        .collect();
    Ok(Json(json!({"experiments": l, "datasets": ds})))
}

/// `GET /api/ui/llm-obs/experiments/{id}`: each span with its evaluations.
pub async fn get(
    State(st): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> ApiResult<Value> {
    let d = st
        .store
        .pg
        .doc(EXPERIMENT, &id)
        .await?
        .ok_or(crate::api::ApiError::NotFound)?;
    let b = &d.body;
    let mut evals: HashMap<String, serde_json::Map<String, Value>> = HashMap::new();
    for m in b["metrics"].as_array().into_iter().flatten() {
        let span = m["span_id"]
            .as_str()
            .map(String::from)
            .unwrap_or_else(|| m["span_id"].to_string());
        let label = m["label"].as_str().unwrap_or("metric").to_string();
        evals
            .entry(span)
            .or_default()
            .insert(label, metric_value(m));
    }
    let rows: Vec<Value> = b["spans"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|s| {
            let sid = s["span_id"].as_str().map(String::from).unwrap_or_else(|| s["span_id"].to_string());
            json!({"span_id": sid, "name": s["name"], "duration": s["duration"], "status": s["status"],
                "record_id": first(&s["dataset_record_id"], &s["meta"]["dataset_record_id"]),
                "input": s["meta"]["input"], "output": s["meta"]["output"], "expected_output": s["meta"]["expected_output"],
                "error": s["meta"]["error"], "evals": evals.remove(&sid).unwrap_or_default()})
        })
        .collect();
    Ok(Json(
        json!({"id": id, "name": b["name"], "status": b["status"], "config": b["config"],
        "summary": summary(b), "rows": rows}),
    ))
}

fn first(a: &Value, b: &Value) -> Value {
    if a.is_null() { b.clone() } else { a.clone() }
}

#[derive(Debug, Deserialize)]
pub struct Range {
    pub from: i64,
    pub to: i64,
}

/// `GET /api/ui/llm-obs/agents`: agents and tools over the range.
pub async fn agents(State(st): State<AppState>, Query(r): Query<Range>) -> ApiResult<Value> {
    let (from, to) = (r.from * 1_000_000, r.to * 1_000_000);
    let c = st.store.pg.conn().await?;
    let agents = c
        .query(
            "SELECT a.ml_app, a.name, count(*)::BIGINT, avg(a.duration)::BIGINT, sum(a.error::INT)::BIGINT,
                    avg(t.tools)::FLOAT8, avg(t.llms)::FLOAT8, avg(t.tokens)::FLOAT8, max(a.trace_id)
             FROM llm_spans a JOIN (
                SELECT trace_id, count(*) FILTER (WHERE kind = 'tool') AS tools, count(*) FILTER (WHERE kind = 'llm') AS llms,
                       COALESCE(sum(total_tokens) FILTER (WHERE kind = 'llm'), 0) AS tokens
                FROM llm_spans WHERE start_ns BETWEEN $1 AND $2 GROUP BY trace_id
             ) t ON t.trace_id = a.trace_id
             WHERE a.kind = 'agent' AND a.start_ns BETWEEN $1 AND $2
             GROUP BY 1, 2 ORDER BY 3 DESC",
            &[&from, &to],
        )
        .await
        .map_err(anyhow::Error::from)?;
    let tools = c
        .query(
            "SELECT ml_app, name, count(*)::BIGINT, sum(error::INT)::BIGINT, avg(duration)::BIGINT FROM llm_spans
             WHERE kind = 'tool' AND start_ns BETWEEN $1 AND $2 GROUP BY 1, 2 ORDER BY 3 DESC LIMIT 200",
            &[&from, &to],
        )
        .await
        .map_err(anyhow::Error::from)?;
    let runs = c
        .query(
            "SELECT trace_id, ml_app, name, start_ns, duration, error FROM llm_spans
             WHERE kind = 'agent' AND (parent_id IS NULL OR parent_id IN ('', '0', 'undefined'))
               AND start_ns BETWEEN $1 AND $2
             ORDER BY start_ns DESC LIMIT 100",
            &[&from, &to],
        )
        .await
        .map_err(anyhow::Error::from)?;
    Ok(Json(json!({
        "agents": agents.iter().map(|x| json!({"ml_app": x.get::<_, String>(0), "name": x.get::<_, String>(1),
            "runs": x.get::<_, i64>(2), "avg_ns": x.get::<_, i64>(3), "errors": x.get::<_, i64>(4),
            "tools": x.get::<_, f64>(5), "llms": x.get::<_, f64>(6), "tokens": x.get::<_, f64>(7),
            "sample_trace": x.get::<_, String>(8)})).collect::<Vec<_>>(),
        "tools": tools.iter().map(|x| json!({"ml_app": x.get::<_, String>(0), "name": x.get::<_, String>(1),
            "calls": x.get::<_, i64>(2), "errors": x.get::<_, i64>(3), "avg_ns": x.get::<_, i64>(4)})).collect::<Vec<_>>(),
        "runs": runs.iter().map(|x| json!({"trace_id": x.get::<_, String>(0),
            "ml_app": x.get::<_, String>(1), "name": x.get::<_, String>(2), "start_ns": x.get::<_, i64>(3),
            "duration": x.get::<_, i64>(4), "error": x.get::<_, bool>(5)})).collect::<Vec<_>>(),
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summaries() {
        let b = json!({"metrics": [
            {"label": "exact", "metric_type": "boolean", "boolean_value": true},
            {"label": "exact", "metric_type": "boolean", "boolean_value": false},
            {"label": "score", "metric_type": "score", "score_value": 0.5},
            {"label": "tone", "metric_type": "categorical", "categorical_value": "polite"}]});
        let s = summary(&b);
        assert_eq!(s["exact"], 0.5);
        assert_eq!(s["score"], 0.5);
        assert_eq!(s["tone"], "polite");
    }
}
