//! Dashboard Reports (a dashboard printed to PDF by Chromium and e-mailed on
//! a cron schedule) and Sheets (saved tables over logs, spans or RUM
//! aggregations, with computed columns, evaluated in the browser).

use crate::AppState;
use crate::api::{ApiError, ApiResult};
use crate::store::pg::{Doc, new_id};
use axum::Json;
use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::header;
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};
use std::time::Duration;

const REPORT: &str = "report";
const SHEET: &str = "sheet";

fn view(d: &Doc) -> Value {
    json!({"id": d.id, "attributes": d.body, "updated_at": d.updated_at})
}

fn body_of(v: Value) -> Value {
    let a = v["data"]["attributes"].clone();
    if a.is_object() { a } else { v }
}

// ---------- reports ----------

fn check_report(b: &Value) -> Result<(), ApiError> {
    let bad = |m: &str| Err(ApiError::BadQuery(m.into()));
    if b["dashboard_id"].as_str().unwrap_or("").is_empty() {
        return bad("dashboard_id is required");
    }
    let cron = b["schedule"].as_str().unwrap_or("");
    if !cron.is_empty() && cron.split_whitespace().count() != 5 {
        return bad(
            "schedule: a 5-field cron expression (UTC), e.g. `0 8 * * 1` for Mondays at 8:00",
        );
    }
    if !b["recipients"].as_array().is_some_and(|r| {
        r.iter()
            .all(|x| x.as_str().is_some_and(|x| x.contains('@')))
    }) {
        return bad("recipients: a list of e-mail addresses");
    }
    Ok(())
}

/// The URL Chromium prints: the dashboard's public link, served locally.
async fn page_url(st: &AppState, id: &str, b: &mut Value) -> Result<String, ApiError> {
    let span = b["live_span"].as_str().unwrap_or("1d").to_string();
    let token = match b["share_token"].as_str() {
        Some(t) if st.store.pg.doc("shared_dashboard", t).await?.is_some() => t.to_string(),
        _ => {
            let Json(v) = crate::api::shared_dashboards::create(
                State(st.clone()),
                None,
                Json(
                    json!({"dashboard_id": b["dashboard_id"], "global_time": {"live_span": span}}),
                ),
            )
            .await?;
            let t = v["token"].as_str().unwrap_or("").to_string();
            b["share_token"] = json!(t);
            st.store.pg.put_doc(REPORT, id, b).await?;
            t
        }
    };
    let addr = st.cfg.listen;
    let host = if addr.ip().is_unspecified() {
        "127.0.0.1".to_string()
    } else {
        addr.ip().to_string()
    };
    let host = if host.contains(':') {
        format!("[{host}]")
    } else {
        host
    };
    Ok(format!("http://{host}:{}/p/{token}", addr.port()))
}

async fn render(st: &AppState, d: &Doc) -> Result<Vec<u8>, ApiError> {
    let mut b = d.body.clone();
    let url = page_url(st, &d.id, &mut b).await?;
    crate::synthetics::browser::pdf(&url, Duration::from_secs(6))
        .await
        .map_err(|e| ApiError::BadQuery(format!("printing the dashboard: {e}")))
}

async fn send(st: &AppState, d: &Doc) -> Result<Vec<String>, ApiError> {
    let pdf = render(st, d).await?;
    let b = &d.body;
    let to: Vec<String> = b["recipients"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|x| x.as_str().map(String::from))
        .collect();
    let name = b["name"].as_str().unwrap_or("Dashboard report");
    let file = format!(
        "{}.pdf",
        name.replace(|c: char| !c.is_ascii_alphanumeric(), "-")
    );
    let text = format!(
        "{name}: the dashboard over the last {}, attached.\n\n{}",
        b["live_span"].as_str().unwrap_or("1d"),
        b["message"].as_str().unwrap_or("")
    );
    let sent = crate::monitors::email::send_with(
        st,
        &to,
        name,
        &text,
        Some((&file, "application/pdf", &pdf)),
    )
    .await;
    let mut nb = b.clone();
    nb["last_sent"] = json!(crate::intake::logs::now_ms());
    nb["last_result"] = json!(if sent.is_empty() {
        "not sent (is ROBINE_DOG_SMTP_URL set?)".to_string()
    } else {
        format!("sent to {}", sent.join(", "))
    });
    let _ = st.store.pg.put_doc(REPORT, &d.id, &nb).await;
    Ok(sent)
}

/// Due reports, checked every minute.
pub fn spawn(st: AppState) {
    tokio::spawn(async move {
        let mut last = -1i64;
        loop {
            tokio::time::sleep(Duration::from_secs(10)).await;
            let now = chrono::Utc::now();
            let minute = now.timestamp() / 60;
            if minute == last {
                continue;
            }
            last = minute;
            let Ok(all) = st.store.pg.docs(REPORT).await else {
                continue;
            };
            for d in all {
                if d.body["schedule"]
                    .as_str()
                    .is_some_and(|c| crate::workflows::cron_matches(c, now))
                    && let Err(e) = send(&st, &d).await
                {
                    tracing::warn!(report = %d.id, error = ?e, "dashboard report");
                }
            }
        }
    });
}

pub async fn reports(State(st): State<AppState>) -> ApiResult<Value> {
    Ok(Json(
        json!({"data": st.store.pg.docs(REPORT).await?.iter().map(view).collect::<Vec<_>>()}),
    ))
}

async fn put_report(st: &AppState, id: &str, v: Value) -> ApiResult<Value> {
    let mut b = body_of(v);
    check_report(&b)?;
    if let Some(old) = st.store.pg.doc(REPORT, id).await? {
        for k in ["share_token", "last_sent", "last_result"] {
            if b.get(k).is_none() && old.body.get(k).is_some() {
                b[k] = old.body[k].clone();
            }
        }
    }
    Ok(Json(
        json!({"data": view(&st.store.pg.put_doc(REPORT, id, &b).await?)}),
    ))
}

pub async fn create_report(State(st): State<AppState>, Json(v): Json<Value>) -> ApiResult<Value> {
    put_report(&st, &new_id(), v).await
}

pub async fn update_report(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Json(v): Json<Value>,
) -> ApiResult<Value> {
    st.store
        .pg
        .doc(REPORT, &id)
        .await?
        .ok_or(ApiError::NotFound)?;
    put_report(&st, &id, v).await
}

pub async fn delete_report(State(st): State<AppState>, Path(id): Path<String>) -> ApiResult<Value> {
    if let Some(d) = st.store.pg.doc(REPORT, &id).await?
        && let Some(t) = d.body["share_token"].as_str()
    {
        st.store.pg.delete_doc("shared_dashboard", t).await?;
    }
    st.store.pg.delete_doc(REPORT, &id).await?;
    Ok(Json(json!({})))
}

/// `POST /reports/{id}/send`: now, whatever the schedule.
pub async fn send_now(State(st): State<AppState>, Path(id): Path<String>) -> ApiResult<Value> {
    let d = st
        .store
        .pg
        .doc(REPORT, &id)
        .await?
        .ok_or(ApiError::NotFound)?;
    let sent = send(&st, &d).await?;
    Ok(Json(json!({"sent": sent})))
}

/// `GET /reports/{id}/pdf`: a preview.
pub async fn pdf(State(st): State<AppState>, Path(id): Path<String>) -> Result<Response, ApiError> {
    let d = st
        .store
        .pg
        .doc(REPORT, &id)
        .await?
        .ok_or(ApiError::NotFound)?;
    let pdf = render(&st, &d).await?;
    Ok(([(header::CONTENT_TYPE, "application/pdf")], Body::from(pdf)).into_response())
}

// ---------- sheets ----------

pub async fn sheets(State(st): State<AppState>) -> ApiResult<Value> {
    Ok(Json(
        json!({"data": st.store.pg.docs(SHEET).await?.iter().map(view).collect::<Vec<_>>()}),
    ))
}

fn check_sheet(b: &Value) -> Result<(), ApiError> {
    if b["name"].as_str().unwrap_or("").trim().is_empty() {
        return Err(ApiError::BadQuery("a sheet needs a name".into()));
    }
    if !matches!(b["dataset"].as_str(), Some("logs" | "spans" | "rum")) {
        return Err(ApiError::BadQuery("dataset: logs, spans or rum".into()));
    }
    Ok(())
}

pub async fn create_sheet(State(st): State<AppState>, Json(v): Json<Value>) -> ApiResult<Value> {
    let b = body_of(v);
    check_sheet(&b)?;
    Ok(Json(
        json!({"data": view(&st.store.pg.put_doc(SHEET, &new_id(), &b).await?)}),
    ))
}

pub async fn update_sheet(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Json(v): Json<Value>,
) -> ApiResult<Value> {
    st.store
        .pg
        .doc(SHEET, &id)
        .await?
        .ok_or(ApiError::NotFound)?;
    let b = body_of(v);
    check_sheet(&b)?;
    Ok(Json(
        json!({"data": view(&st.store.pg.put_doc(SHEET, &id, &b).await?)}),
    ))
}

pub async fn delete_sheet(State(st): State<AppState>, Path(id): Path<String>) -> ApiResult<Value> {
    st.store.pg.delete_doc(SHEET, &id).await?;
    Ok(Json(json!({})))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_validation() {
        assert!(
            check_report(
                &json!({"dashboard_id": "x", "schedule": "0 8 * * 1", "recipients": ["a@b.co"]})
            )
            .is_ok()
        );
        assert!(check_report(&json!({"dashboard_id": "", "recipients": []})).is_err());
        assert!(
            check_report(&json!({"dashboard_id": "x", "schedule": "daily", "recipients": []}))
                .is_err()
        );
        assert!(check_report(&json!({"dashboard_id": "x", "recipients": ["nobody"]})).is_err());
    }
}
