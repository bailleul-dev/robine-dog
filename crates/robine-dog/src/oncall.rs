//! On-Call: rotation schedules, escalation policies per team, and pages. A
//! monitor whose message names `@oncall-<team>` pages that team when it
//! alerts: the policy's steps e-mail whoever is on call, one step after the
//! other, until someone acknowledges in the UI. The page resolves when the
//! monitor recovers.

use crate::AppState;
use crate::api::{ApiError, ApiResult};
use crate::store::pg::{Doc, new_id};
use axum::Json;
use axum::extract::{Path, State};
use regex::Regex;
use serde_json::{Value, json};
use std::sync::LazyLock;

const SCHEDULE: &str = "oncall_schedule";
const POLICY: &str = "escalation_policy";
const PAGE: &str = "oncall_page";

static HANDLE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"@oncall-([A-Za-z0-9_.-]+[A-Za-z0-9_])").unwrap());

pub fn teams_in(message: &str) -> Vec<String> {
    let mut out = Vec::new();
    for c in HANDLE.captures_iter(message) {
        if !out.contains(&c[1].to_string()) {
            out.push(c[1].to_string());
        }
    }
    out
}

/// Who a schedule puts on call at `t`: members take turns of
/// `length_hours`, from `start`.
pub fn on_call(schedule: &Value, t: i64) -> Option<String> {
    let members: Vec<&str> = schedule["members"]
        .as_array()?
        .iter()
        .filter_map(|m| m.as_str())
        .collect();
    if members.is_empty() {
        return None;
    }
    let start = schedule["start"].as_i64().unwrap_or(0);
    let len =
        (schedule["length_hours"].as_f64().unwrap_or(168.0) * 3_600_000.0).max(60_000.0) as i64;
    let turn = (t - start).div_euclid(len);
    Some(members[turn.rem_euclid(members.len() as i64) as usize].to_string())
}

async fn docs(st: &AppState, kind: &str) -> anyhow::Result<Vec<Doc>> {
    st.store.pg.docs(kind).await
}

fn with_id(d: &Doc) -> Value {
    let mut v = d.body.clone();
    v["id"] = json!(d.id);
    v
}

/// The addresses a step reaches now: schedules' on-call members and users.
async fn targets(st: &AppState, step: &Value, now: i64) -> Vec<String> {
    let schedules = docs(st, SCHEDULE).await.unwrap_or_default();
    let mut out = Vec::new();
    for t in step["targets"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|x| x.as_str())
    {
        let who = match t.strip_prefix("schedule:") {
            Some(id) => schedules
                .iter()
                .find(|s| s.id == id || s.body["name"].as_str() == Some(id))
                .and_then(|s| on_call(&s.body, now)),
            None => Some(t.trim_start_matches("user:").to_string()),
        };
        if let Some(w) = who
            && !out.contains(&w)
        {
            out.push(w);
        }
    }
    out
}

async fn policy_of(st: &AppState, team: &str) -> Option<Value> {
    docs(st, POLICY)
        .await
        .ok()?
        .into_iter()
        .find(|d| d.body["team"].as_str() == Some(team))
        .map(|d| d.body)
}

/// Notifies the page's current step; records it in the page's timeline.
async fn run_step(st: &AppState, id: &str, page: &mut Value, now: i64) {
    let team = page["team"].as_str().unwrap_or("").to_string();
    let Some(policy) = policy_of(st, &team).await else {
        push(page, now, format!("No escalation policy for team {team}"));
        return;
    };
    let steps = policy["steps"].as_array().cloned().unwrap_or_default();
    let i = page["step"].as_u64().unwrap_or(0) as usize;
    let Some(step) = steps.get(i % steps.len().max(1)) else {
        return;
    };
    let to = targets(st, step, now).await;
    let body = format!(
        "{}\n\nAcknowledge: {}/on-call?page={id}",
        page["message"].as_str().unwrap_or(""),
        st.cfg.base_url
    );
    let title = format!("[Page] {}", page["title"].as_str().unwrap_or(""));
    let sent = crate::monitors::email::send(st, &to, &title, &body).await;
    push(
        page,
        now,
        format!(
            "Step {}: paged {}",
            i + 1,
            if to.is_empty() {
                "nobody (no target)".into()
            } else {
                to.join(", ")
            }
        ),
    );
    if sent.len() < to.len() {
        push(
            page,
            now,
            "Some e-mails were not sent (is ROBINE_DOG_SMTP_URL set?)".into(),
        );
    }
    page["step_at"] = json!(now);
}

fn push(page: &mut Value, ts: i64, text: String) {
    if !page["timeline"].is_array() {
        page["timeline"] = json!([]);
    }
    page["timeline"]
        .as_array_mut()
        .unwrap()
        .push(json!({"ts": ts, "text": text}));
}

async fn open_page(
    st: &AppState,
    team: &str,
    source: &str,
    title: &str,
    message: &str,
    now: i64,
) -> anyhow::Result<String> {
    let id = new_id();
    let mut page = json!({"team": team, "source": source, "title": title, "message": message,
        "status": "triggered", "step": 0, "created_at": now, "timeline": []});
    push(&mut page, now, "Triggered".into());
    run_step(st, &id, &mut page, now).await;
    st.store.pg.put_doc(PAGE, &id, &page).await?;
    Ok(id)
}

/// From a monitor notification: opens a page per `@oncall-<team>` when it
/// alerts (once per monitor group), resolves them when it recovers.
pub async fn notify(
    st: &AppState,
    source: &str,
    title: &str,
    message: &str,
    recovered: bool,
) -> Vec<String> {
    let teams = teams_in(message);
    if teams.is_empty() {
        return Vec::new();
    }
    let now = crate::intake::logs::now_ms();
    let pages = docs(st, PAGE).await.unwrap_or_default();
    let mut out = Vec::new();
    for team in teams {
        let open = pages.iter().find(|p| {
            p.body["team"].as_str() == Some(team.as_str())
                && p.body["source"].as_str() == Some(source)
                && p.body["status"].as_str() != Some("resolved")
        });
        match (open, recovered) {
            (Some(p), true) => {
                let mut b = p.body.clone();
                b["status"] = json!("resolved");
                push(&mut b, now, "Resolved: the monitor recovered".into());
                let _ = st.store.pg.put_doc(PAGE, &p.id, &b).await;
            }
            (None, false) => {
                let opened = open_page(st, &team, source, title, message, now).await;
                if opened.is_ok() {
                    out.push(format!("@oncall-{team}"));
                }
            }
            _ => {}
        }
    }
    out
}

/// Escalates unacknowledged pages every 30 seconds.
pub fn spawn(st: AppState) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(30));
        loop {
            tick.tick().await;
            let now = crate::intake::logs::now_ms();
            for p in docs(&st, PAGE).await.unwrap_or_default() {
                if p.body["status"].as_str() != Some("triggered") {
                    continue;
                }
                let Some(policy) = policy_of(&st, p.body["team"].as_str().unwrap_or("")).await
                else {
                    continue;
                };
                let steps = policy["steps"].as_array().cloned().unwrap_or_default();
                let i = p.body["step"].as_u64().unwrap_or(0) as usize;
                let delay = steps
                    .get(i % steps.len().max(1))
                    .and_then(|s| s["delay_minutes"].as_i64())
                    .unwrap_or(5)
                    * 60_000;
                let step_at = p.body["step_at"].as_i64().unwrap_or(now);
                // After the last step, the policy starts over `repeat` times.
                let max = steps.len() * (1 + policy["repeat"].as_u64().unwrap_or(0) as usize);
                if now - step_at < delay || i + 1 >= max {
                    continue;
                }
                let mut b = p.body.clone();
                b["step"] = json!(i + 1);
                run_step(&st, &p.id, &mut b, now).await;
                let _ = st.store.pg.put_doc(PAGE, &p.id, &b).await;
            }
        }
    });
}

// ---------------------------------------------------------------- API

fn kind_of(what: &str) -> Result<&'static str, ApiError> {
    match what {
        "schedules" => Ok(SCHEDULE),
        "escalation-policies" => Ok(POLICY),
        _ => Err(ApiError::NotFound),
    }
}

fn check(kind: &str, v: &Value) -> Result<(), ApiError> {
    let bad = |m: &str| Err(ApiError::BadQuery(m.into()));
    if v["name"].as_str().unwrap_or("").trim().is_empty() {
        return bad("a name is required");
    }
    if kind == SCHEDULE && v["members"].as_array().is_none_or(|m| m.is_empty()) {
        return bad("a schedule needs members");
    }
    if kind == POLICY
        && (v["team"].as_str().unwrap_or("").is_empty()
            || v["steps"].as_array().is_none_or(|s| s.is_empty()))
    {
        return bad("a policy needs a team and steps");
    }
    Ok(())
}

/// `GET /api/v2/on-call/{schedules|escalation-policies}`.
pub async fn list(State(st): State<AppState>, Path(what): Path<String>) -> ApiResult<Value> {
    let kind = kind_of(&what)?;
    let now = crate::intake::logs::now_ms();
    let data: Vec<Value> = docs(&st, kind)
        .await?
        .iter()
        .map(|d| {
            let mut v = with_id(d);
            if kind == SCHEDULE {
                v["on_call"] = json!(on_call(&d.body, now));
            }
            v
        })
        .collect();
    Ok(Json(json!({"data": data})))
}

pub async fn create(
    State(st): State<AppState>,
    Path(what): Path<String>,
    Json(v): Json<Value>,
) -> ApiResult<Value> {
    let kind = kind_of(&what)?;
    let mut v = v
        .get("data")
        .and_then(|d| d.get("attributes"))
        .cloned()
        .unwrap_or(v);
    check(kind, &v)?;
    if kind == SCHEDULE && v["start"].as_i64().is_none() {
        v["start"] = json!(crate::intake::logs::now_ms());
    }
    let d = st.store.pg.put_doc(kind, &new_id(), &v).await?;
    Ok(Json(with_id(&d)))
}

pub async fn update(
    State(st): State<AppState>,
    Path((what, id)): Path<(String, String)>,
    Json(v): Json<Value>,
) -> ApiResult<Value> {
    let kind = kind_of(&what)?;
    st.store
        .pg
        .doc(kind, &id)
        .await?
        .ok_or(ApiError::NotFound)?;
    let v = v
        .get("data")
        .and_then(|d| d.get("attributes"))
        .cloned()
        .unwrap_or(v);
    check(kind, &v)?;
    let d = st.store.pg.put_doc(kind, &id, &v).await?;
    Ok(Json(with_id(&d)))
}

pub async fn delete(
    State(st): State<AppState>,
    Path((what, id)): Path<(String, String)>,
) -> ApiResult<Value> {
    let kind = kind_of(&what)?;
    if !st.store.pg.delete_doc(kind, &id).await? {
        return Err(ApiError::NotFound);
    }
    Ok(Json(json!({})))
}

/// `GET /api/v2/on-call/teams/{team}/on-call`: who the team's policy reaches first.
pub async fn team_on_call(
    State(st): State<AppState>,
    Path(team): Path<String>,
) -> ApiResult<Value> {
    let now = crate::intake::logs::now_ms();
    let policy = policy_of(&st, &team).await.ok_or(ApiError::NotFound)?;
    let mut steps = Vec::new();
    for s in policy["steps"].as_array().into_iter().flatten() {
        steps.push(
            json!({"delay_minutes": s["delay_minutes"], "responders": targets(&st, s, now).await}),
        );
    }
    Ok(Json(
        json!({"team": team, "policy": policy["name"], "steps": steps}),
    ))
}

/// `GET /api/v2/on-call/pages`.
pub async fn pages(State(st): State<AppState>) -> ApiResult<Value> {
    let mut data: Vec<Value> = docs(&st, PAGE).await?.iter().map(with_id).collect();
    data.sort_by_key(|p| std::cmp::Reverse(p["created_at"].as_i64().unwrap_or(0)));
    data.truncate(200);
    Ok(Json(json!({"data": data})))
}

/// `POST /api/v2/on-call/pages`: pages a team by hand.
pub async fn create_page(State(st): State<AppState>, Json(v): Json<Value>) -> ApiResult<Value> {
    let a = v
        .get("data")
        .and_then(|d| d.get("attributes"))
        .cloned()
        .unwrap_or(v);
    let team = a["team"]
        .as_str()
        .or(a["target"]["identifier"].as_str())
        .unwrap_or("");
    let title = a["title"].as_str().unwrap_or("");
    if team.is_empty() || title.is_empty() {
        return Err(ApiError::BadQuery("a page needs a team and a title".into()));
    }
    let now = crate::intake::logs::now_ms();
    let id = open_page(
        &st,
        team,
        "manual",
        title,
        a["description"]
            .as_str()
            .or(a["message"].as_str())
            .unwrap_or(""),
        now,
    )
    .await?;
    let d = st
        .store
        .pg
        .doc(PAGE, &id)
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(json!({"data": with_id(&d)})))
}

/// `POST /api/v2/on-call/pages/{id}/{acknowledge|resolve}` (the UI passes
/// who acknowledged).
pub async fn page_action(
    State(st): State<AppState>,
    Path((id, action)): Path<(String, String)>,
    by: Option<axum::Extension<crate::auth::Identity>>,
) -> ApiResult<Value> {
    let d = st
        .store
        .pg
        .doc(PAGE, &id)
        .await?
        .ok_or(ApiError::NotFound)?;
    let mut b = d.body;
    let who = by
        .map(|axum::Extension(i)| i.email.unwrap_or(i.sub))
        .unwrap_or_else(|| "api".into());
    let now = crate::intake::logs::now_ms();
    match action.as_str() {
        "acknowledge" => {
            b["status"] = json!("acknowledged");
            b["acked_by"] = json!(who);
            push(&mut b, now, format!("Acknowledged by {who}"));
        }
        "resolve" => {
            b["status"] = json!("resolved");
            push(&mut b, now, format!("Resolved by {who}"));
        }
        _ => return Err(ApiError::NotFound),
    }
    let d = st.store.pg.put_doc(PAGE, &id, &b).await?;
    Ok(Json(json!({"data": with_id(&d)})))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotations_and_handles() {
        let s = json!({"members": ["a@x.io", "b@x.io", "c@x.io"], "start": 0, "length_hours": 24});
        assert_eq!(on_call(&s, 0).as_deref(), Some("a@x.io"));
        assert_eq!(on_call(&s, 86_400_000 + 5).as_deref(), Some("b@x.io"));
        assert_eq!(on_call(&s, 3 * 86_400_000).as_deref(), Some("a@x.io"));
        assert_eq!(on_call(&s, -1).as_deref(), Some("c@x.io"));
        assert_eq!(
            teams_in("Down! @oncall-payments @ops@x.io @oncall-payments."),
            vec!["payments"]
        );
    }
}
