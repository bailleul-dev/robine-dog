//! Case Management: cases with a status, priority, assignee and project,
//! opened by hand or from a monitor, a security signal or an issue, each
//! with a timeline of comments and changes. Datadog's `/api/v2/cases` shape.

use crate::AppState;
use crate::api::{ApiError, ApiResult};
use crate::store::pg::{Doc, new_id};
use axum::Json;
use axum::extract::{Path, Query, State};
use serde::Deserialize;
use serde_json::{Value, json};

const KIND: &str = "case";
const STATUSES: &[&str] = &["OPEN", "IN_PROGRESS", "CLOSED"];
const PRIORITIES: &[&str] = &["NOT_DEFINED", "P1", "P2", "P3", "P4", "P5"];

fn view(d: &Doc) -> Value {
    let mut a = d.body.clone();
    a["modified_at"] = json!(d.updated_at);
    json!({"id": d.id, "type": "case", "attributes": a})
}

fn who(id: Option<axum::Extension<crate::auth::Identity>>) -> String {
    id.map(|axum::Extension(i)| i.email.unwrap_or(i.sub))
        .unwrap_or_else(|| "api".into())
}

fn push(b: &mut Value, ts: i64, author: &str, kind: &str, text: String) {
    if !b["timeline"].is_array() {
        b["timeline"] = json!([]);
    }
    b["timeline"]
        .as_array_mut()
        .unwrap()
        .push(json!({"ts": ts, "author": author, "type": kind, "text": text}));
}

#[derive(Debug, Deserialize)]
pub struct Filter {
    #[serde(alias = "filter[status]")]
    pub status: Option<String>,
    #[serde(alias = "filter[query]", alias = "filter")]
    pub query: Option<String>,
}

/// `GET /api/v2/cases`.
pub async fn list(State(st): State<AppState>, Query(f): Query<Filter>) -> ApiResult<Value> {
    let q = f.query.unwrap_or_default().to_lowercase();
    let mut data: Vec<&Doc> = Vec::new();
    let docs = st.store.pg.docs(KIND).await?;
    for d in &docs {
        let b = &d.body;
        if f.status
            .as_deref()
            .is_some_and(|s| b["status"].as_str() != Some(s))
        {
            continue;
        }
        if !q.is_empty() && !b.to_string().to_lowercase().contains(&q) {
            continue;
        }
        data.push(d);
    }
    data.sort_by_key(|d| std::cmp::Reverse(d.body["number"].as_i64().unwrap_or(0)));
    Ok(Json(
        json!({"data": data.iter().map(|d| view(d)).collect::<Vec<_>>(),
        "meta": {"page": {"total": data.len()}}}),
    ))
}

/// `POST /api/v2/cases`.
pub async fn create(
    State(st): State<AppState>,
    id: Option<axum::Extension<crate::auth::Identity>>,
    Json(v): Json<Value>,
) -> ApiResult<Value> {
    let a = v["data"]["attributes"].clone();
    let a = if a.is_object() { a } else { v };
    let title = a["title"].as_str().unwrap_or("").trim().to_string();
    if title.is_empty() {
        return Err(ApiError::BadQuery("a case needs a title".into()));
    }
    let priority = a["priority"].as_str().unwrap_or("NOT_DEFINED");
    if !PRIORITIES.contains(&priority) {
        return Err(ApiError::BadQuery(format!(
            "priority: one of {}",
            PRIORITIES.join(", ")
        )));
    }
    let number = st
        .store
        .pg
        .docs(KIND)
        .await?
        .iter()
        .filter_map(|d| d.body["number"].as_i64())
        .max()
        .unwrap_or(0)
        + 1;
    let project = a["project"].as_str().unwrap_or("CASE").to_uppercase();
    let now = crate::intake::logs::now_ms();
    let author = who(id);
    let mut b = json!({"number": number, "key": format!("{project}-{number}"), "title": title,
        "description": a["description"].as_str().unwrap_or(""), "status": "OPEN", "priority": priority,
        "type": a["type"].as_str().unwrap_or("STANDARD"), "project": project, "assignee": a["assignee"].as_str(),
        "origin": a["origin"].clone(), "created_at": now, "created_by": author, "timeline": []});
    push(&mut b, now, &author, "created", "Case created".into());
    let d = st.store.pg.put_doc(KIND, &new_id(), &b).await?;
    Ok(Json(json!({"data": view(&d)})))
}

/// `GET /api/v2/cases/{id}`.
pub async fn get(State(st): State<AppState>, Path(id): Path<String>) -> ApiResult<Value> {
    let d = st
        .store
        .pg
        .doc(KIND, &id)
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(json!({"data": view(&d)})))
}

/// `POST /api/v2/cases/{id}/{status|priority|assign|unassign|comment|archive}`.
pub async fn action(
    State(st): State<AppState>,
    Path((id, what)): Path<(String, String)>,
    by: Option<axum::Extension<crate::auth::Identity>>,
    Json(v): Json<Value>,
) -> ApiResult<Value> {
    let d = st
        .store
        .pg
        .doc(KIND, &id)
        .await?
        .ok_or(ApiError::NotFound)?;
    let mut b = d.body;
    let a = &v["data"]["attributes"];
    let author = who(by);
    let now = crate::intake::logs::now_ms();
    let bad = |m: String| ApiError::BadQuery(m);
    match what.as_str() {
        "status" => {
            let s = a["status"].as_str().unwrap_or("");
            if !STATUSES.contains(&s) {
                return Err(bad(format!("status: one of {}", STATUSES.join(", "))));
            }
            let old = b["status"].as_str().unwrap_or("").to_string();
            push(
                &mut b,
                now,
                &author,
                "status",
                format!("Status: {old} → {s}"),
            );
            b["status"] = json!(s);
            if s == "CLOSED" {
                b["closed_at"] = json!(now);
            }
        }
        "priority" => {
            let p = a["priority"].as_str().unwrap_or("");
            if !PRIORITIES.contains(&p) {
                return Err(bad(format!("priority: one of {}", PRIORITIES.join(", "))));
            }
            push(&mut b, now, &author, "priority", format!("Priority: {p}"));
            b["priority"] = json!(p);
        }
        "assign" => {
            let to = a["assignee_id"]
                .as_str()
                .or(a["assignee"].as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            if to.is_empty() {
                return Err(bad("assignee_id is required".into()));
            }
            push(&mut b, now, &author, "assign", format!("Assigned to {to}"));
            b["assignee"] = json!(to);
        }
        "unassign" => {
            push(&mut b, now, &author, "assign", "Unassigned".into());
            b["assignee"] = Value::Null;
        }
        "comment" => {
            let text = a["comment"].as_str().unwrap_or("").trim().to_string();
            if text.is_empty() {
                return Err(bad("comment is required".into()));
            }
            push(&mut b, now, &author, "comment", text);
        }
        "archive" => {
            push(&mut b, now, &author, "archive", "Archived".into());
            b["archived"] = json!(true);
        }
        _ => return Err(ApiError::NotFound),
    }
    let d = st.store.pg.put_doc(KIND, &id, &b).await?;
    Ok(Json(json!({"data": view(&d)})))
}
