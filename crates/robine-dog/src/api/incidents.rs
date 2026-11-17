//! Incidents, and the status page.
//!
//! An incident is declared (by hand, or from a Watchdog story or a
//! monitor), has a severity and a state (`active`, `stable`, `resolved`),
//! and keeps a timeline: its notes and every change of state or severity.
//! Declaring and resolving one posts an event (`source:incident`).
//!
//! The status page (`/status`, public once enabled) shows components —
//! each the state of a synthetic test or of a monitor — and the incidents
//! marked public.

use super::{ApiError, ApiResult};
use crate::AppState;
use crate::auth::Identity;
use crate::intake::logs::{LogRow, now_ms};
use crate::store::pg::new_id;
use axum::extract::{Path, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

const KIND: &str = "incident";
const PAGE_KIND: &str = "status_page";
const PAGE_ID: &str = "default";
const SEVERITIES: [&str; 6] = ["SEV-1", "SEV-2", "SEV-3", "SEV-4", "SEV-5", "UNKNOWN"];
const STATES: [&str; 3] = ["active", "stable", "resolved"];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    /// Epoch milliseconds.
    pub ts: i64,
    /// `declared`, `note`, `state` or `severity`.
    pub kind: String,
    pub text: String,
    #[serde(default)]
    pub by: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Incident {
    pub id: String,
    /// The number people say: IR-12.
    pub public_id: i64,
    pub title: String,
    pub severity: String,
    pub state: String,
    pub created: i64,
    pub modified: i64,
    #[serde(default)]
    pub resolved: Option<i64>,
    #[serde(default)]
    pub commander: Option<String>,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub services: Vec<String>,
    /// Shown on the status page.
    #[serde(default)]
    pub public: bool,
    #[serde(default)]
    pub timeline: Vec<Entry>,
}

/// What can be given when declaring, and changed afterwards.
#[derive(Debug, Default, Deserialize)]
pub struct Patch {
    title: Option<String>,
    severity: Option<String>,
    state: Option<String>,
    commander: Option<String>,
    summary: Option<String>,
    services: Option<Vec<String>>,
    public: Option<bool>,
}

type Who = Option<Extension<Identity>>;

fn who(id: &Who) -> Option<String> {
    let Extension(i) = id.as_ref()?;
    i.name.clone().or_else(|| i.email.clone())
}

fn one_of(value: &str, allowed: &[&str], what: &str) -> Result<(), ApiError> {
    if allowed.contains(&value) {
        return Ok(());
    }
    Err(ApiError::BadQuery(format!(
        "{what} is one of {}",
        allowed.join(", ")
    )))
}

/// Applies a change, recording in the timeline what people follow.
fn apply(inc: &mut Incident, p: Patch, by: Option<String>, now: i64) -> Result<(), ApiError> {
    if let Some(t) = p.title {
        let t = t.trim().to_string();
        if t.is_empty() {
            return Err(ApiError::BadQuery("an incident needs a title".into()));
        }
        inc.title = t;
    }
    if let Some(s) = p.severity
        && s != inc.severity
    {
        one_of(&s, &SEVERITIES, "the severity")?;
        inc.timeline.push(Entry {
            ts: now,
            kind: "severity".into(),
            text: format!("{} → {s}", inc.severity),
            by: by.clone(),
        });
        inc.severity = s;
    }
    if let Some(s) = p.state
        && s != inc.state
    {
        one_of(&s, &STATES, "the state")?;
        inc.timeline.push(Entry {
            ts: now,
            kind: "state".into(),
            text: format!("{} → {s}", inc.state),
            by: by.clone(),
        });
        inc.resolved = (s == "resolved").then_some(now);
        inc.state = s;
    }
    if let Some(c) = p.commander {
        inc.commander = Some(c.trim().to_string()).filter(|c| !c.is_empty());
    }
    if let Some(s) = p.summary {
        inc.summary = s;
    }
    if let Some(s) = p.services {
        inc.services = s
            .into_iter()
            .map(|x| x.trim().to_string())
            .filter(|x| !x.is_empty())
            .collect();
    }
    if let Some(public) = p.public {
        inc.public = public;
    }
    inc.modified = now;
    Ok(())
}

async fn stored(st: &AppState) -> anyhow::Result<Vec<Incident>> {
    Ok(st
        .store
        .pg
        .docs(KIND)
        .await?
        .into_iter()
        .filter_map(|d| serde_json::from_value(d.body).ok())
        .collect())
}

async fn load(st: &AppState, id: &str) -> Result<Incident, ApiError> {
    let d = st.store.pg.doc(KIND, id).await?.ok_or(ApiError::NotFound)?;
    serde_json::from_value(d.body).map_err(|e| ApiError::Internal(e.into()))
}

async fn save(st: &AppState, inc: &Incident) -> Result<(), ApiError> {
    let body = serde_json::to_value(inc).map_err(anyhow::Error::from)?;
    st.store.pg.put_doc(KIND, &inc.id, &body).await?;
    Ok(())
}

async fn announce(st: &AppState, inc: &Incident, what: &str) {
    let title = format!("IR-{} {what}: {}", inc.public_id, inc.title);
    let row = LogRow {
        ts: now_ms(),
        status: if inc.state == "resolved" {
            "ok"
        } else {
            "error"
        }
        .into(),
        service: inc.services.first().cloned(),
        host: None,
        source: Some("incident".into()),
        message: title.clone(),
        tags: vec![
            format!("severity:{}", inc.severity.to_lowercase()),
            format!("incident_state:{}", inc.state),
        ],
        attributes: Map::from_iter([
            ("evt".to_string(), json!({"title": title})),
            (
                "incident".to_string(),
                json!({"id": inc.id, "public_id": inc.public_id}),
            ),
        ]),
        trace_id: None,
        span_id: None,
    };
    crate::events::emit(st, vec![row]).await;
}

pub async fn list(State(st): State<AppState>) -> ApiResult<Value> {
    let mut all = stored(&st).await?;
    all.sort_by_key(|i| (i.state == "resolved", std::cmp::Reverse(i.created)));
    Ok(Json(json!({"incidents": all})))
}

pub async fn get(State(st): State<AppState>, Path(id): Path<String>) -> ApiResult<Incident> {
    Ok(Json(load(&st, &id).await?))
}

pub async fn create(
    State(st): State<AppState>,
    id: Who,
    Json(p): Json<Patch>,
) -> ApiResult<Incident> {
    let now = now_ms();
    let by = who(&id);
    let number = stored(&st)
        .await?
        .iter()
        .map(|i| i.public_id)
        .max()
        .unwrap_or(0)
        + 1;
    let mut inc = Incident {
        id: new_id(),
        public_id: number,
        title: String::new(),
        severity: "UNKNOWN".into(),
        state: "active".into(),
        created: now,
        modified: now,
        resolved: None,
        commander: by.clone(),
        summary: String::new(),
        services: vec![],
        public: false,
        timeline: vec![Entry {
            ts: now,
            kind: "declared".into(),
            text: "Incident declared".into(),
            by: by.clone(),
        }],
    };
    if p.title.as_deref().is_none_or(|t| t.trim().is_empty()) {
        return Err(ApiError::BadQuery("an incident needs a title".into()));
    }
    // Declared as it is: its first severity and state are not changes.
    if let Some(s) = &p.severity {
        one_of(s, &SEVERITIES, "the severity")?;
        inc.severity = s.clone();
    }
    apply(&mut inc, Patch { state: None, ..p }, by, now)?;
    save(&st, &inc).await?;
    announce(&st, &inc, "declared").await;
    Ok(Json(inc))
}

pub async fn update(
    State(st): State<AppState>,
    Path(id): Path<String>,
    by: Who,
    Json(p): Json<Patch>,
) -> ApiResult<Incident> {
    let mut inc = load(&st, &id).await?;
    let before = inc.state.clone();
    apply(&mut inc, p, who(&by), now_ms())?;
    save(&st, &inc).await?;
    if inc.state != before {
        let what = if inc.state == "resolved" {
            "resolved"
        } else {
            "is"
        };
        let what = if what == "is" {
            format!("is {}", inc.state)
        } else {
            what.to_string()
        };
        announce(&st, &inc, &what).await;
    }
    Ok(Json(inc))
}

#[derive(Deserialize)]
pub struct Note {
    text: String,
}

pub async fn note(
    State(st): State<AppState>,
    Path(id): Path<String>,
    by: Who,
    Json(n): Json<Note>,
) -> ApiResult<Incident> {
    let text = n.text.trim().to_string();
    if text.is_empty() {
        return Err(ApiError::BadQuery("the note is empty".into()));
    }
    let mut inc = load(&st, &id).await?;
    let now = now_ms();
    inc.timeline.push(Entry {
        ts: now,
        kind: "note".into(),
        text,
        by: who(&by),
    });
    inc.modified = now;
    save(&st, &inc).await?;
    Ok(Json(inc))
}

pub async fn delete(State(st): State<AppState>, Path(id): Path<String>) -> ApiResult<Value> {
    if !st.store.pg.delete_doc(KIND, &id).await? {
        return Err(ApiError::NotFound);
    }
    Ok(Json(json!({})))
}

// --- status page -----------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Component {
    pub name: String,
    /// `synthetic` (a test's public id) or `monitor` (its id).
    pub kind: String,
    #[serde(rename = "ref")]
    pub reference: String,
    /// `operational`, `degraded`, `outage` or `unknown`; computed.
    #[serde(default, skip_deserializing)]
    pub status: String,
    /// Per day over the past 30, oldest first: the runs that passed and
    /// those that failed (synthetic tests only); computed.
    #[serde(default, skip_deserializing, skip_serializing_if = "Vec::is_empty")]
    pub history: Vec<[f64; 2]>,
    /// Percent of the runs that passed over those days; computed.
    #[serde(default, skip_deserializing, skip_serializing_if = "Option::is_none")]
    pub uptime: Option<f64>,
}

const DAY_MS: i64 = 86_400_000;
const DAYS: i64 = 30;

/// The runs of a synthetic test per day: `[passed, failed]`.
async fn history(st: &AppState, check_id: &str, now: i64) -> Vec<[f64; 2]> {
    let to = (now / DAY_MS + 1) * DAY_MS;
    let from = to - DAYS * DAY_MS;
    let query = |status: &str| {
        format!(
            "sum:synthetics.test_runs{{check_id:{check_id},status:{status}}}.as_count().rollup(sum, 86400)"
        )
    };
    let body = json!({"queries": [query("passed"), query("failed")], "from": from, "to": to});
    let mut days = vec![[0.0; 2]; DAYS as usize];
    let Ok(v) = crate::mcp::ui(st, axum::http::Method::POST, "/metrics/query", Some(body)).await
    else {
        return days;
    };
    for s in v["series"].as_array().into_iter().flatten() {
        let which = s["query_index"].as_u64().unwrap_or(0).min(1) as usize;
        for p in s["points"].as_array().into_iter().flatten() {
            let day = (p[0].as_f64().unwrap_or(0.0) as i64 - from) / DAY_MS;
            if let (true, Some(n)) = ((0..DAYS).contains(&day), p[1].as_f64()) {
                days[day as usize][which] += n;
            }
        }
    }
    days
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusPage {
    /// Public at `/status` when true.
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub components: Vec<Component>,
}

async fn page(st: &AppState) -> Result<StatusPage, ApiError> {
    let mut page = match st.store.pg.doc(PAGE_KIND, PAGE_ID).await? {
        Some(d) => serde_json::from_value(d.body).map_err(|e| ApiError::Internal(e.into()))?,
        None => StatusPage {
            enabled: false,
            title: "Status".into(),
            description: String::new(),
            components: vec![],
        },
    };
    for c in &mut page.components {
        c.status = match c.kind.as_str() {
            "synthetic" => match crate::synthetics::last_result(&c.reference) {
                Some(r) if r.passed => "operational",
                Some(_) => "outage",
                None => "unknown",
            },
            "monitor" => {
                let groups = match c.reference.parse::<i64>() {
                    Ok(id) => st.store.pg.monitor_groups(id).await?,
                    Err(_) => vec![],
                };
                match crate::monitors::overall(groups.iter().map(|g| g.status.clone())).as_str() {
                    _ if groups.is_empty() => "unknown",
                    "Alert" => "outage",
                    "Warn" => "degraded",
                    "OK" => "operational",
                    _ => "unknown",
                }
            }
            _ => "unknown",
        }
        .to_string();
        if c.kind == "synthetic" {
            c.history = history(st, &c.reference, now_ms()).await;
            let (passed, failed) = c
                .history
                .iter()
                .fold((0.0, 0.0), |(p, f), d| (p + d[0], f + d[1]));
            c.uptime = (passed + failed > 0.0).then(|| 100.0 * passed / (passed + failed));
        }
    }
    Ok(page)
}

pub async fn status_page(State(st): State<AppState>) -> ApiResult<StatusPage> {
    Ok(Json(page(&st).await?))
}

pub async fn put_status_page(
    State(st): State<AppState>,
    Json(mut p): Json<StatusPage>,
) -> ApiResult<StatusPage> {
    p.title = p.title.trim().to_string();
    if p.title.is_empty() {
        p.title = "Status".into();
    }
    for c in &mut p.components {
        c.name = c.name.trim().to_string();
        one_of(&c.kind, &["synthetic", "monitor"], "a component's kind")?;
        if c.name.is_empty() || c.reference.is_empty() {
            return Err(ApiError::BadQuery(
                "a component needs a name and a test or monitor".into(),
            ));
        }
    }
    let body = serde_json::to_value(&p).map_err(anyhow::Error::from)?;
    st.store.pg.put_doc(PAGE_KIND, PAGE_ID, &body).await?;
    Ok(Json(page(&st).await?))
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn when(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|d| d.format("%b %-d, %H:%M UTC").to_string())
        .unwrap_or_default()
}

/// The public page: plain HTML, nothing to load, readable when all else is down.
pub fn render(page: &StatusPage, incidents: &[Incident], now: i64) -> String {
    let label = |s: &str| match s {
        "operational" => "Operational",
        "degraded" => "Degraded",
        "outage" => "Outage",
        _ => "Unknown",
    };
    let active: Vec<&Incident> = incidents.iter().filter(|i| i.state != "resolved").collect();
    let worst = page
        .components
        .iter()
        .map(|c| c.status.as_str())
        .min_by_key(|s| match *s {
            "outage" => 0,
            "degraded" => 1,
            "unknown" => 2,
            _ => 3,
        })
        .unwrap_or("operational");
    let (banner, class) = match (worst, active.is_empty()) {
        ("outage", _) => ("Some systems are down", "outage"),
        ("degraded", _) => ("Some systems are degraded", "degraded"),
        (_, false) => ("An incident is in progress", "degraded"),
        _ => ("All systems operational", "operational"),
    };
    let mut h = format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
         <meta http-equiv=\"refresh\" content=\"60\"><title>{t}</title><style>\
         :root{{color-scheme:light dark;--bg:#f6f7f9;--card:#fff;--text:#1c2030;--muted:#6b7280;--line:#e4e6eb;--ok:#2a9d5c;--warn:#d9902f;--bad:#d64545}}\
         @media(prefers-color-scheme:dark){{:root{{--bg:#14161c;--card:#1d2029;--text:#e8eaf0;--muted:#9aa1ad;--line:#2c303b}}}}\
         body{{margin:0;background:var(--bg);color:var(--text);font:15px/1.5 system-ui,sans-serif}}\
         main{{max-width:760px;margin:0 auto;padding:32px 16px 64px}}h1{{font-size:24px;margin:0 0 4px}}h2{{font-size:16px;margin:32px 0 10px}}\
         .muted{{color:var(--muted)}}.banner{{margin:20px 0;padding:14px 16px;border-radius:8px;color:#fff;font-weight:600}}\
         .operational{{background:var(--ok)}}.degraded{{background:var(--warn)}}.outage{{background:var(--bad)}}.unknown{{background:var(--muted)}}\
         .card{{background:var(--card);border:1px solid var(--line);border-radius:8px}}\
         .row{{display:flex;justify-content:space-between;gap:12px;padding:12px 16px;border-top:1px solid var(--line)}}.row:first-child{{border-top:0}}\
         .days{{display:flex;gap:2px;padding:0 16px}}.days i{{flex:1;height:26px;border-radius:2px}}.nodata{{background:var(--line)}}\
         .under{{display:flex;justify-content:space-between;padding:4px 16px 12px;font-size:12px}}\
         .dot{{display:inline-block;width:9px;height:9px;border-radius:50%;margin-right:7px}}\
         .inc{{padding:14px 16px;margin-bottom:10px}}.inc h3{{margin:0 0 2px;font-size:15px}}.inc ul{{margin:8px 0 0;padding-left:18px}}.inc li{{margin:3px 0}}\
         </style></head><body><main><h1>{t}</h1><div class=\"muted\">{d}</div><div class=\"banner {class}\">{banner}</div>",
        t = esc(&page.title),
        d = esc(&page.description),
    );
    let incident = |h: &mut String, i: &Incident| {
        h.push_str(&format!(
            "<div class=\"card inc\"><h3>{}</h3><div class=\"muted\">{} · {}{}</div>",
            esc(&i.title),
            esc(&i.state),
            when(i.created),
            i.resolved
                .map(|r| format!(" → {}", when(r)))
                .unwrap_or_default()
        ));
        if !i.summary.trim().is_empty() {
            h.push_str(&format!("<p>{}</p>", esc(&i.summary)));
        }
        h.push_str("<ul>");
        for e in i.timeline.iter().rev().filter(|e| e.kind != "severity") {
            h.push_str(&format!(
                "<li><span class=\"muted\">{}</span> {}</li>",
                when(e.ts),
                esc(&e.text)
            ));
        }
        h.push_str("</ul></div>");
    };
    if !active.is_empty() {
        h.push_str("<h2>Ongoing incidents</h2>");
        for i in &active {
            incident(&mut h, i);
        }
    }
    if !page.components.is_empty() {
        h.push_str("<h2>Components</h2><div class=\"card\">");
        for c in &page.components {
            h.push_str(&format!(
                "<div class=\"row\"><span>{}</span><span><span class=\"dot {}\"></span>{}</span></div>",
                esc(&c.name),
                c.status,
                label(&c.status)
            ));
            if !c.history.is_empty() {
                h.push_str("<div class=\"days\">");
                for d in &c.history {
                    let (total, failed) = (d[0] + d[1], d[1]);
                    let class = if total == 0.0 {
                        "nodata"
                    } else if failed == 0.0 {
                        "operational"
                    } else if failed / total < 0.05 {
                        "degraded"
                    } else {
                        "outage"
                    };
                    h.push_str(&format!("<i class=\"{class}\"></i>"));
                }
                h.push_str(&format!(
                    "</div><div class=\"muted under\"><span>{DAYS} days ago</span><span>{}</span><span>Today</span></div>",
                    c.uptime.map(|u| format!("{u:.2}% uptime")).unwrap_or_default()
                ));
            }
        }
        h.push_str("</div>");
    }
    let past: Vec<&Incident> = incidents
        .iter()
        .filter(|i| i.resolved.is_some_and(|r| now - r < 30 * 86_400_000))
        .collect();
    h.push_str("<h2>Past 30 days</h2>");
    if past.is_empty() {
        h.push_str("<div class=\"muted\">No incident.</div>");
    }
    for i in past {
        incident(&mut h, i);
    }
    h.push_str(&format!(
        "<p class=\"muted\" style=\"margin-top:32px\">Updated {}</p></main></body></html>",
        when(now)
    ));
    h
}

/// `GET /status`: public, once enabled.
pub async fn public_page(State(st): State<AppState>) -> Response {
    let (page, incidents) = match (page(&st).await, stored(&st).await) {
        (Ok(p), Ok(i)) => (p, i),
        _ => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
    };
    if !page.enabled {
        return StatusCode::NOT_FOUND.into_response();
    }
    let mut public: Vec<Incident> = incidents.into_iter().filter(|i| i.public).collect();
    public.sort_by_key(|i| std::cmp::Reverse(i.created));
    (
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        render(&page, &public, now_ms()),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn incident() -> Incident {
        Incident {
            id: "a".into(),
            public_id: 3,
            title: "API <down>".into(),
            severity: "SEV-2".into(),
            state: "active".into(),
            created: 1_000,
            modified: 1_000,
            resolved: None,
            commander: None,
            summary: String::new(),
            services: vec![],
            public: true,
            timeline: vec![],
        }
    }

    #[test]
    fn records_what_changes() {
        let mut i = incident();
        let p = Patch {
            state: Some("resolved".into()),
            severity: Some("SEV-1".into()),
            ..Patch::default()
        };
        apply(&mut i, p, Some("ada".into()), 5_000).unwrap();
        assert_eq!((i.state.as_str(), i.resolved), ("resolved", Some(5_000)));
        let kinds: Vec<&str> = i.timeline.iter().map(|e| e.kind.as_str()).collect();
        assert_eq!(kinds, ["severity", "state"]);
        assert_eq!(i.timeline[1].text, "active → resolved");
        // Reopened: no longer resolved.
        let p = Patch {
            state: Some("active".into()),
            ..Patch::default()
        };
        apply(&mut i, p, None, 6_000).unwrap();
        assert_eq!(i.resolved, None);
        let bad = Patch {
            state: Some("gone".into()),
            ..Patch::default()
        };
        assert!(apply(&mut i, bad, None, 7_000).is_err());
    }

    #[test]
    fn renders_the_public_page() {
        let page = StatusPage {
            enabled: true,
            title: "Shop".into(),
            description: String::new(),
            components: vec![Component {
                name: "API".into(),
                kind: "synthetic".into(),
                reference: "x".into(),
                status: "outage".into(),
                history: vec![[10.0, 0.0], [0.0, 0.0], [5.0, 5.0]],
                uptime: Some(75.0),
            }],
        };
        let html = render(&page, &[incident()], 2_000);
        assert!(html.contains("Some systems are down") && html.contains("API &lt;down&gt;"));
        assert!(html.contains("Ongoing incidents") && html.contains("No incident."));
        assert!(html.contains(
            r#"<i class="operational"></i><i class="nodata"></i><i class="outage"></i>"#
        ));
        assert!(html.contains("75.00% uptime"));
        let calm = StatusPage {
            components: vec![],
            ..page
        };
        assert!(render(&calm, &[], 2_000).contains("All systems operational"));
    }
}
