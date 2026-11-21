//! Read-only public links to dashboards (`/api/v1/dashboard/public`). The
//! page `/p/{token}` is served without a session; it reads the dashboard and
//! runs its widgets' queries through `/api/public/dashboard/{token}`, which
//! only runs the requests stored in the dashboard, with the template
//! variables the share allows.

use super::{ApiError, ApiResult, dashboards, query};
use crate::AppState;
use crate::auth::Identity;
use crate::store::pg::Doc;
use axum::extract::{Path, State};
use axum::{Extension, Json};
use rand::Rng;
use serde::Deserialize;
use serde_json::{Map, Value, json};
use std::collections::HashMap;

pub const KIND: &str = "shared_dashboard";

/// The longest time range a public viewer may ask for.
const MAX_RANGE_MS: i64 = 30 * 86_400_000;

fn iso(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|d| d.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
        .unwrap_or_default()
}

fn token() -> String {
    let mut rng = rand::rng();
    (0..32)
        .map(|_| format!("{:x}", rng.random_range(0..16u8)))
        .collect()
}

fn present(st: &AppState, d: &Doc) -> Value {
    let mut b = d.body.clone();
    if let Some(o) = b.as_object_mut() {
        o.insert("token".into(), json!(d.id));
        o.insert("created".into(), json!(iso(d.created_at)));
        o.insert(
            "public_url".into(),
            json!(format!(
                "{}/p/{}",
                st.cfg.base_url.trim_end_matches('/'),
                d.id
            )),
        );
        o.insert("status".into(), json!("active"));
    }
    b
}

/// The settings a share keeps, from Datadog's body.
fn settings(body: &Value, base: &mut Map<String, Value>) -> Result<(), ApiError> {
    if let Some(t) = body.get("share_type").and_then(Value::as_str)
        && t != "open"
    {
        return Err(ApiError::BadQuery(
            "only open shares are supported (no invitations)".into(),
        ));
    }
    base.insert("share_type".into(), json!("open"));
    let span = body
        .pointer("/global_time/live_span")
        .and_then(Value::as_str)
        .unwrap_or("1h");
    if span_ms(span).is_none() {
        return Err(ApiError::BadQuery(format!("unknown live_span {span}")));
    }
    base.insert("global_time".into(), json!({"live_span": span}));
    for k in ["global_time_selectable_enabled"] {
        if let Some(v) = body.get(k) {
            base.insert(k.into(), json!(v.as_bool().unwrap_or(false)));
        }
    }
    base.entry("global_time_selectable_enabled")
        .or_insert(json!(false));
    if let Some(v) = body.get("selectable_template_vars") {
        if !v.is_array() && !v.is_null() {
            return Err(ApiError::BadQuery(
                "selectable_template_vars is an array".into(),
            ));
        }
        base.insert("selectable_template_vars".into(), v.clone());
    }
    base.entry("selectable_template_vars").or_insert(json!([]));
    Ok(())
}

/// `5m`, `1h`, `1d`, `1w`, `1mo`.
fn span_ms(s: &str) -> Option<i64> {
    let digits: String = s.chars().take_while(char::is_ascii_digit).collect();
    let n: i64 = digits.parse().ok()?;
    let unit = match &s[digits.len()..] {
        "m" => 60_000,
        "h" => 3_600_000,
        "d" => 86_400_000,
        "w" => 7 * 86_400_000,
        "mo" => 30 * 86_400_000,
        _ => return None,
    };
    (n > 0).then_some(n * unit)
}

pub async fn create(
    State(st): State<AppState>,
    who: Option<Extension<Identity>>,
    Json(body): Json<Value>,
) -> ApiResult<Value> {
    let dash = body["dashboard_id"]
        .as_str()
        .ok_or_else(|| ApiError::BadQuery("dashboard_id is required".into()))?;
    let d = st
        .store
        .pg
        .doc(dashboards::KIND, dash)
        .await?
        .ok_or_else(|| ApiError::BadQuery(format!("no dashboard {dash}")))?;
    let mut base = Map::new();
    base.insert("dashboard_id".into(), json!(d.id));
    base.insert(
        "dashboard_type".into(),
        json!(if d.body["layout_type"] == "free" {
            "custom_screenboard"
        } else {
            "custom_timeboard"
        }),
    );
    let author = who.map(
        |Extension(i)| json!({"handle": i.email.clone().unwrap_or(i.sub.clone()), "name": i.name}),
    );
    base.insert(
        "author".into(),
        author.unwrap_or(json!({"handle": "robine-dog", "name": "robine-dog"})),
    );
    settings(&body, &mut base)?;
    let doc = st
        .store
        .pg
        .put_doc(KIND, &token(), &Value::Object(base))
        .await?;
    Ok(Json(present(&st, &doc)))
}

async fn load(st: &AppState, token: &str) -> Result<Doc, ApiError> {
    st.store
        .pg
        .doc(KIND, token)
        .await?
        .ok_or(ApiError::NotFound)
}

pub async fn get(State(st): State<AppState>, Path(token): Path<String>) -> ApiResult<Value> {
    Ok(Json(present(&st, &load(&st, &token).await?)))
}

pub async fn update(
    State(st): State<AppState>,
    Path(token): Path<String>,
    Json(body): Json<Value>,
) -> ApiResult<Value> {
    let d = load(&st, &token).await?;
    let mut base = d.body.as_object().cloned().unwrap_or_default();
    settings(&body, &mut base)?;
    let doc = st
        .store
        .pg
        .put_doc(KIND, &token, &Value::Object(base))
        .await?;
    Ok(Json(present(&st, &doc)))
}

pub async fn delete(State(st): State<AppState>, Path(token): Path<String>) -> ApiResult<Value> {
    if !st.store.pg.delete_doc(KIND, &token).await? {
        return Err(ApiError::NotFound);
    }
    Ok(Json(json!({"deleted_public_dashboard_token": token})))
}

/// The shares of a dashboard, for the UI.
pub async fn of_dashboard(
    State(st): State<AppState>,
    Path(dash): Path<String>,
) -> ApiResult<Value> {
    let docs = st.store.pg.docs(KIND).await?;
    Ok(Json(json!(
        docs.iter()
            .filter(|d| d.body["dashboard_id"] == dash.as_str())
            .map(|d| present(&st, d))
            .collect::<Vec<_>>()
    )))
}

// --- the public side -------------------------------------------------------------------

/// The template variables a viewer may change, and to which values (empty:
/// any).
fn selectable(share: &Value) -> HashMap<String, Vec<String>> {
    share["selectable_template_vars"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| {
            let name = v["name"].as_str()?.to_string();
            let tags = v["visible_tags"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|t| t.as_str().map(String::from))
                .collect();
            Some((name, tags))
        })
        .collect()
}

/// The dashboard as a public viewer sees it.
pub async fn public_dashboard(
    State(st): State<AppState>,
    Path(token): Path<String>,
) -> ApiResult<Value> {
    let share = load(&st, &token).await?;
    let d = st
        .store
        .pg
        .doc(
            dashboards::KIND,
            share.body["dashboard_id"].as_str().unwrap_or(""),
        )
        .await?
        .ok_or(ApiError::NotFound)?;
    let b = &d.body;
    Ok(Json(json!({
        "title": b["title"],
        "description": b["description"],
        "layout_type": b["layout_type"],
        "widgets": b["widgets"],
        "template_variables": b["template_variables"],
        "global_time": share.body["global_time"],
        "global_time_selectable_enabled": share.body["global_time_selectable_enabled"],
        "selectable_template_vars": share.body["selectable_template_vars"],
    })))
}

#[derive(Deserialize)]
pub struct PublicQuery {
    widget_id: i64,
    #[serde(default)]
    request: usize,
    from: i64,
    to: i64,
    #[serde(default)]
    scalar: bool,
    #[serde(default)]
    vars: HashMap<String, String>,
}

fn find_widget(widgets: &Value, id: i64) -> Option<&Value> {
    for w in widgets.as_array()? {
        if w["id"].as_i64() == Some(id) {
            return Some(&w["definition"]);
        }
        if let Some(found) = find_widget(&w["definition"]["widgets"], id) {
            return Some(found);
        }
    }
    None
}

/// `$service` → `service:web` (or nothing for `*`), `$service.value` → `web`:
/// what the UI does (`web/src/lib/dash.ts`).
pub fn substitute(text: &str, vars: &[(String, Option<String>, String)], metric: bool) -> String {
    let mut sorted: Vec<&(String, Option<String>, String)> = vars.iter().collect();
    sorted.sort_by_key(|(n, _, _)| std::cmp::Reverse(n.len()));
    let mut out = text.to_string();
    for (name, prefix, value) in sorted {
        out = out.replace(&format!("${name}.value"), value);
        let tag = if value == "*" {
            if metric {
                "*".to_string()
            } else {
                String::new()
            }
        } else if let Some(p) = prefix.as_deref().filter(|p| !p.is_empty()) {
            format!("{p}:{value}")
        } else {
            value.clone()
        };
        out = out.replace(&format!("${name}"), &tag);
    }
    if metric {
        static TIDY: std::sync::LazyLock<[(regex::Regex, &str); 3]> =
            std::sync::LazyLock::new(|| {
                [
                    (regex::Regex::new(r"\{\s*\*\s*,\s*").unwrap(), "{"),
                    (regex::Regex::new(r",\s*\*\s*\}").unwrap(), "}"),
                    (regex::Regex::new(r",\s*\*\s*,").unwrap(), ","),
                ]
            });
        for (re, to) in TIDY.iter() {
            out = re.replace_all(&out, *to).into_owned();
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The request as the query API takes it: legacy `q` requests become
/// queries, template variables are applied.
pub fn build(
    r: &Value,
    vars: &[(String, Option<String>, String)],
    from: i64,
    to: i64,
) -> Result<Value, ApiError> {
    let (mut queries, formulas) = match r["queries"].as_array().filter(|q| !q.is_empty()) {
        Some(q) => (
            q.clone(),
            r["formulas"].as_array().cloned().unwrap_or_default(),
        ),
        None => {
            let q = r["q"]
                .as_str()
                .ok_or_else(|| ApiError::BadQuery("the widget has no query".into()))?;
            let mut parts = Vec::new();
            let (mut depth, mut cur) = (0, String::new());
            for c in q.chars() {
                match c {
                    '{' => depth += 1,
                    '}' => depth -= 1,
                    ',' if depth == 0 => {
                        parts.push(std::mem::take(&mut cur));
                        continue;
                    }
                    _ => {}
                }
                cur.push(c);
            }
            parts.push(cur);
            let queries = parts
                .iter()
                .enumerate()
                .map(|(i, p)| json!({"data_source": "metrics", "name": format!("query{}", i + 1), "query": p.trim()}))
                .collect();
            (queries, vec![])
        }
    };
    for q in &mut queries {
        if q["data_source"] == "metrics" {
            let s = substitute(q["query"].as_str().unwrap_or(""), vars, true);
            q["query"] = json!(s);
        } else {
            let s = substitute(
                q.pointer("/search/query")
                    .and_then(Value::as_str)
                    .unwrap_or(""),
                vars,
                false,
            );
            q["search"] = json!({"query": s});
        }
    }
    Ok(json!({"from": from, "to": to, "queries": queries, "formulas": formulas}))
}

pub async fn public_query(
    State(st): State<AppState>,
    Path(token): Path<String>,
    Json(q): Json<PublicQuery>,
) -> Result<Json<Value>, ApiError> {
    let share = load(&st, &token).await?;
    if q.to <= q.from || q.to - q.from > MAX_RANGE_MS {
        return Err(ApiError::BadQuery(
            "the time range is at most 30 days".into(),
        ));
    }
    let d = st
        .store
        .pg
        .doc(
            dashboards::KIND,
            share.body["dashboard_id"].as_str().unwrap_or(""),
        )
        .await?
        .ok_or(ApiError::NotFound)?;
    let def = find_widget(&d.body["widgets"], q.widget_id).ok_or(ApiError::NotFound)?;
    let r = def["requests"]
        .as_array()
        .and_then(|a| a.get(q.request))
        .ok_or(ApiError::NotFound)?;
    let allowed = selectable(&share.body);
    let vars: Vec<(String, Option<String>, String)> = d.body["template_variables"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| {
            let name = v["name"].as_str()?.to_string();
            let default = v["default"]
                .as_str()
                .or_else(|| v["defaults"].get(0).and_then(Value::as_str))
                .unwrap_or("*")
                .to_string();
            let value = match (q.vars.get(&name), allowed.get(&name)) {
                (Some(asked), Some(tags))
                    if tags.is_empty() || tags.contains(asked) || asked == &default =>
                {
                    asked.clone()
                }
                _ => default,
            };
            Some((name, v["prefix"].as_str().map(String::from), value))
        })
        .collect();
    let body = build(r, &vars, q.from, q.to)?;
    if q.scalar {
        query::scalar(State(st), Json(body)).await
    } else {
        query::timeseries(State(st), Json(body)).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn substitution() {
        let vars = vec![
            (
                "env".to_string(),
                Some("env".to_string()),
                "prod".to_string(),
            ),
            (
                "service".to_string(),
                Some("service".to_string()),
                "*".to_string(),
            ),
            ("s".to_string(), None, "x".to_string()),
        ];
        assert_eq!(
            substitute("avg:cpu{$service, $env} by {host}", &vars, true),
            "avg:cpu{env:prod} by {host}"
        );
        assert_eq!(
            substitute("$service status:error $env.value", &vars, false),
            "status:error prod"
        );
        assert_eq!(substitute("$s", &vars, false), "x");
    }

    #[test]
    fn legacy_requests() {
        let r = json!({"q": "avg:a{x,y} by {h}, sum:b{*}"});
        let b = build(&r, &[], 1, 2).unwrap();
        assert_eq!(b["queries"][0]["query"], "avg:a{x,y} by {h}");
        assert_eq!(b["queries"][1]["name"], "query2");
        let w = json!([{"id": 1, "definition": {"type": "group", "widgets": [{"id": 7, "definition": {"type": "note"}}]}}]);
        assert_eq!(find_widget(&w, 7).unwrap()["type"], "note");
        assert!(find_widget(&w, 8).is_none());
        assert_eq!(span_ms("15m"), Some(900_000));
        assert_eq!(span_ms("1mo"), Some(30 * 86_400_000));
        assert_eq!(span_ms("x"), None);
    }
}
