//! Scorecards, as Datadog's: rules grouped in scorecards, each passed or
//! failed by every service. Out-of-the-box rules are evaluated by robine-dog
//! from service definitions, monitors, SLOs, spans and logs; custom rules
//! get their outcomes through the API (`/api/v2/scorecard/outcomes`).

use crate::AppState;
use crate::api::{ApiError, ApiResult, time_clause};
use crate::query::sql::Dataset;
use crate::store::greptime::{i64_at, str_at};
use crate::store::pg::new_id;
use axum::Json;
use axum::extract::{Path, Query, State};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, HashMap};

pub const RULE_KIND: &str = "scorecard_rule";
pub const OUTCOME_KIND: &str = "scorecard_outcome";
const DAY: i64 = 86_400_000;

pub const PRODUCTION: &str = "Production Readiness";
pub const OBSERVABILITY: &str = "Observability Best Practices";
pub const OWNERSHIP: &str = "Ownership & Documentation";

/// The out-of-the-box rules: id, scorecard, name, description.
pub const BUILTIN: &[(&str, &str, &str, &str)] = &[
    (
        "ootb-monitors",
        PRODUCTION,
        "Has monitors",
        "A monitor queries or is tagged with the service.",
    ),
    (
        "ootb-slos",
        PRODUCTION,
        "Has SLOs",
        "An SLO queries or is tagged with the service.",
    ),
    (
        "ootb-on-call",
        PRODUCTION,
        "Has on-call contacts",
        "The service definition lists contacts or an on-call integration.",
    ),
    (
        "ootb-deployment",
        OBSERVABILITY,
        "Deployment tracking",
        "Spans of the last day carry a version.",
    ),
    (
        "ootb-env",
        OBSERVABILITY,
        "Unified service tagging",
        "Spans of the last day carry an env.",
    ),
    (
        "ootb-logs",
        OBSERVABILITY,
        "Logs correlated with traces",
        "Logs of the last day carry a trace id.",
    ),
    (
        "ootb-team",
        OWNERSHIP,
        "Has a team",
        "The service definition names a team.",
    ),
    (
        "ootb-repo",
        OWNERSHIP,
        "Has a code repository",
        "The service definition links a repository.",
    ),
    (
        "ootb-docs",
        OWNERSHIP,
        "Has documentation",
        "The service definition links docs or a runbook.",
    ),
    (
        "ootb-tier",
        OWNERSHIP,
        "Has a tier",
        "The service definition sets a tier.",
    ),
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rule {
    pub name: String,
    pub scorecard_name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default)]
    pub owner: Option<String>,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Outcome {
    pub rule_id: String,
    pub service_name: String,
    /// `pass`, `fail` or `skip`.
    pub state: String,
    #[serde(default)]
    pub remarks: String,
    #[serde(default)]
    pub modified_at: i64,
}

/// What built-in rules look at, per service.
#[derive(Default, Debug)]
pub struct Facts {
    pub services: BTreeSet<String>,
    pub versioned: BTreeSet<String>,
    pub with_env: BTreeSet<String>,
    pub correlated_logs: HashMap<String, i64>,
    pub monitored: BTreeSet<String>,
    pub with_slo: BTreeSet<String>,
    pub definitions: HashMap<String, super::definitions::Summary>,
}

/// Does a monitor or an SLO body point at the service?
fn mentions(body: &Value, service: &str) -> bool {
    let tag = format!("service:{service}");
    let in_tags = body["tags"]
        .as_array()
        .is_some_and(|t| t.iter().any(|x| x.as_str() == Some(tag.as_str())));
    if in_tags {
        return true;
    }
    // In a query: `service:shop` not followed by more of a name.
    let text = body["query"].to_string();
    text.match_indices(&tag).any(|(i, _)| {
        text[i + tag.len()..]
            .chars()
            .next()
            .is_none_or(|c| !(c.is_alphanumeric() || "-_.".contains(c)))
    })
}

pub async fn facts(st: &AppState, now: i64) -> anyhow::Result<Facts> {
    let gt = &st.store.gt;
    let spans = gt
        .query(&format!(
            "SELECT service,
                    max(CASE WHEN version IS NOT NULL AND version != '' THEN 1 ELSE 0 END),
                    max(CASE WHEN env IS NOT NULL AND env != '' THEN 1 ELSE 0 END)
             FROM spans WHERE top_level AND {} GROUP BY service",
            time_clause(Dataset::Spans, now - DAY, now)
        ))
        .await?;
    let logs = gt
        .query(&format!(
            "SELECT service, count(*) FROM logs
             WHERE trace_id IS NOT NULL AND trace_id != '' AND service IS NOT NULL AND {}
             GROUP BY service",
            time_clause(Dataset::Logs, now - DAY, now)
        ))
        .await?;
    let mut f = Facts::default();
    for r in &spans {
        let Some(s) = str_at(r, 0) else { continue };
        if i64_at(r, 1) > 0 {
            f.versioned.insert(s.clone());
        }
        if i64_at(r, 2) > 0 {
            f.with_env.insert(s.clone());
        }
        f.services.insert(s);
    }
    for r in &logs {
        if let Some(s) = str_at(r, 0) {
            f.correlated_logs.insert(s, i64_at(r, 1));
        }
    }
    for d in super::definitions::summaries(st).await? {
        f.services.insert(d.service.clone());
        f.definitions.insert(d.service.clone(), d);
    }
    let monitors = st.store.pg.docs(crate::monitors::KIND).await?;
    let slos = st.store.pg.docs(crate::slo::KIND).await?;
    for s in &f.services {
        if monitors.iter().any(|m| mentions(&m.body, s)) {
            f.monitored.insert(s.clone());
        }
        if slos.iter().any(|m| mentions(&m.body, s)) {
            f.with_slo.insert(s.clone());
        }
    }
    Ok(f)
}

/// A built-in rule's outcome for a service: state and remarks.
pub fn evaluate(rule: &str, service: &str, f: &Facts) -> (&'static str, String) {
    let def = f.definitions.get(service);
    let pass = |ok: bool, yes: String, no: String| {
        if ok { ("pass", yes) } else { ("fail", no) }
    };
    let link = |kinds: &[&str]| {
        def.and_then(|d| {
            d.links
                .iter()
                .find(|l| kinds.contains(&l["type"].as_str().unwrap_or("")))
                .map(|l| l["url"].as_str().unwrap_or("").to_string())
        })
    };
    match rule {
        "ootb-monitors" => pass(
            f.monitored.contains(service),
            "monitored".into(),
            format!("no monitor mentions service:{service}"),
        ),
        "ootb-slos" => pass(
            f.with_slo.contains(service),
            "has an SLO".into(),
            "no SLO mentions the service".into(),
        ),
        "ootb-on-call" => {
            let n = def.map(|d| d.contacts.len()).unwrap_or(0);
            pass(
                n > 0,
                format!("{n} contact(s)"),
                "no contacts in the service definition".into(),
            )
        }
        "ootb-deployment" => pass(
            f.versioned.contains(service),
            "spans carry version".into(),
            "no version on spans: set DD_VERSION".into(),
        ),
        "ootb-env" => pass(
            f.with_env.contains(service),
            "spans carry env".into(),
            "no env on spans: set DD_ENV".into(),
        ),
        "ootb-logs" => match f.correlated_logs.get(service) {
            Some(n) => (
                "pass",
                format!("{n} logs with a trace id over the last day"),
            ),
            None => (
                "fail",
                "no log with a trace id: enable logs injection".into(),
            ),
        },
        "ootb-team" => match def.and_then(|d| d.team.clone()) {
            Some(t) => ("pass", format!("team {t}")),
            None => ("fail", "no team in the service definition".into()),
        },
        "ootb-repo" => match link(&["repo"]) {
            Some(u) => ("pass", u),
            None => ("fail", "no repo link in the service definition".into()),
        },
        "ootb-docs" => match link(&["doc", "runbook"]) {
            Some(u) => ("pass", u),
            None => (
                "fail",
                "no doc or runbook link in the service definition".into(),
            ),
        },
        "ootb-tier" => match def.and_then(|d| d.tier.clone()) {
            Some(t) => ("pass", format!("tier {t}")),
            None => ("fail", "no tier in the service definition".into()),
        },
        _ => ("skip", String::new()),
    }
}

async fn custom_rules(st: &AppState) -> anyhow::Result<Vec<(String, Rule, i64, i64)>> {
    let mut v: Vec<(String, Rule, i64, i64)> = st
        .store
        .pg
        .docs(RULE_KIND)
        .await?
        .into_iter()
        .filter_map(|d| {
            Some((
                d.id,
                serde_json::from_value(d.body).ok()?,
                d.created_at,
                d.updated_at,
            ))
        })
        .collect();
    v.sort_by_key(|r| r.2);
    Ok(v)
}

fn rule_out(id: &str, r: &Rule, custom: bool, created: i64, updated: i64) -> Value {
    json!({
        "id": id,
        "type": "rule",
        "attributes": {
            "name": r.name, "scorecard_name": r.scorecard_name, "description": r.description,
            "enabled": r.enabled, "owner": r.owner, "custom": custom,
            "created_at": crate::logconfig::iso(created), "modified_at": crate::logconfig::iso(updated),
        },
        "relationships": {"scorecard": {"data": {"id": r.scorecard_name, "type": "scorecard"}}},
    })
}

fn builtin_rules() -> Vec<(String, Rule)> {
    BUILTIN
        .iter()
        .map(|(id, card, name, desc)| {
            (
                id.to_string(),
                Rule {
                    name: name.to_string(),
                    scorecard_name: card.to_string(),
                    description: desc.to_string(),
                    enabled: true,
                    owner: Some("robine-dog".into()),
                },
            )
        })
        .collect()
}

/// `GET /api/v2/scorecard/rules`.
pub async fn list_rules(State(st): State<AppState>) -> ApiResult<Value> {
    let mut data: Vec<Value> = builtin_rules()
        .iter()
        .map(|(id, r)| rule_out(id, r, false, 0, 0))
        .collect();
    data.extend(
        custom_rules(&st)
            .await?
            .iter()
            .map(|(id, r, c, u)| rule_out(id, r, true, *c, *u)),
    );
    Ok(Json(json!({"data": data})))
}

fn rule_body(v: &Value) -> Result<Rule, ApiError> {
    let a = v
        .pointer("/data/attributes")
        .cloned()
        .unwrap_or_else(|| v.clone());
    let r: Rule = serde_json::from_value(a).map_err(|e| ApiError::BadQuery(e.to_string()))?;
    if r.name.trim().is_empty() || r.scorecard_name.trim().is_empty() {
        return Err(ApiError::BadQuery(
            "a rule has a name and a scorecard_name".into(),
        ));
    }
    Ok(r)
}

/// `POST /api/v2/scorecard/rules`.
pub async fn create_rule(State(st): State<AppState>, Json(v): Json<Value>) -> ApiResult<Value> {
    let r = rule_body(&v)?;
    let id = new_id();
    let d = st
        .store
        .pg
        .put_doc(
            RULE_KIND,
            &id,
            &serde_json::to_value(&r).map_err(anyhow::Error::from)?,
        )
        .await?;
    Ok(Json(
        json!({"data": rule_out(&id, &r, true, d.created_at, d.updated_at)}),
    ))
}

/// `PUT /api/v2/scorecard/rules/{rule_id}`.
pub async fn update_rule(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Json(v): Json<Value>,
) -> ApiResult<Value> {
    if BUILTIN.iter().any(|b| b.0 == id) {
        return Err(ApiError::BadQuery(
            "out-of-the-box rules cannot change".into(),
        ));
    }
    st.store
        .pg
        .doc(RULE_KIND, &id)
        .await?
        .ok_or(ApiError::NotFound)?;
    let r = rule_body(&v)?;
    let d = st
        .store
        .pg
        .put_doc(
            RULE_KIND,
            &id,
            &serde_json::to_value(&r).map_err(anyhow::Error::from)?,
        )
        .await?;
    Ok(Json(
        json!({"data": rule_out(&id, &r, true, d.created_at, d.updated_at)}),
    ))
}

/// `DELETE /api/v2/scorecard/rules/{rule_id}`: with its outcomes.
pub async fn delete_rule(State(st): State<AppState>, Path(id): Path<String>) -> ApiResult<Value> {
    if !st.store.pg.delete_doc(RULE_KIND, &id).await? {
        return Err(ApiError::NotFound);
    }
    for d in st.store.pg.docs(OUTCOME_KIND).await? {
        if d.body["rule_id"] == id.as_str() {
            st.store.pg.delete_doc(OUTCOME_KIND, &d.id).await?;
        }
    }
    Ok(Json(json!({})))
}

/// `POST /api/v2/scorecard/outcomes/batch` and `/outcomes`: custom rules'
/// outcomes, as `results` or as JSON:API `data`.
pub async fn post_outcomes(State(st): State<AppState>, Json(v): Json<Value>) -> ApiResult<Value> {
    let mut items: Vec<(String, String, String, String)> = Vec::new();
    if let Some(results) = v
        .pointer("/data/attributes/results")
        .and_then(Value::as_array)
    {
        for r in results {
            items.push((
                r["rule_id"].as_str().unwrap_or("").into(),
                r["service_name"].as_str().unwrap_or("").into(),
                r["state"].as_str().unwrap_or("").into(),
                r["remarks"].as_str().unwrap_or("").into(),
            ));
        }
    } else if let Some(data) = v["data"].as_array() {
        for r in data {
            items.push((
                r.pointer("/relationships/rule/data/id")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .into(),
                r["attributes"]["service_name"]
                    .as_str()
                    .unwrap_or("")
                    .into(),
                r["attributes"]["state"].as_str().unwrap_or("").into(),
                r["attributes"]["remarks"].as_str().unwrap_or("").into(),
            ));
        }
    } else {
        return Err(ApiError::BadQuery(
            "expected data.attributes.results".into(),
        ));
    }
    let rules: HashMap<String, Rule> = custom_rules(&st)
        .await?
        .into_iter()
        .map(|(id, r, ..)| (id, r))
        .collect();
    let now = crate::intake::logs::now_ms();
    let mut out = Vec::new();
    for (rule, service, state, remarks) in items {
        if !rules.contains_key(&rule) {
            return Err(ApiError::BadQuery(format!("{rule} is not a custom rule")));
        }
        if service.is_empty() {
            return Err(ApiError::BadQuery("service_name is required".into()));
        }
        if !["pass", "fail", "skip"].contains(&state.as_str()) {
            return Err(ApiError::BadQuery("state is pass, fail or skip".into()));
        }
        let o = Outcome {
            rule_id: rule.clone(),
            service_name: service.clone(),
            state,
            remarks,
            modified_at: now,
        };
        st.store
            .pg
            .put_doc(
                OUTCOME_KIND,
                &format!("{rule}/{service}"),
                &serde_json::to_value(&o).map_err(anyhow::Error::from)?,
            )
            .await?;
        out.push(outcome_out(&o));
    }
    Ok(Json(json!({"data": out})))
}

fn outcome_out(o: &Outcome) -> Value {
    json!({
        "id": format!("{}/{}", o.rule_id, o.service_name),
        "type": "outcome",
        "attributes": {
            "service_name": o.service_name, "state": o.state, "remarks": o.remarks,
            "modified_at": crate::logconfig::iso(o.modified_at),
        },
        "relationships": {"rule": {"data": {"id": o.rule_id, "type": "rule"}}},
    })
}

/// Every outcome: built-in rules evaluated now, custom ones as posted.
async fn outcomes(st: &AppState) -> anyhow::Result<(Facts, Vec<Outcome>)> {
    let now = crate::intake::logs::now_ms();
    let f = facts(st, now).await?;
    let mut out = Vec::new();
    for (id, ..) in BUILTIN {
        for s in &f.services {
            let (state, remarks) = evaluate(id, s, &f);
            out.push(Outcome {
                rule_id: id.to_string(),
                service_name: s.clone(),
                state: state.into(),
                remarks,
                modified_at: now,
            });
        }
    }
    let custom: BTreeSet<String> = custom_rules(st)
        .await?
        .into_iter()
        .filter(|r| r.1.enabled)
        .map(|r| r.0)
        .collect();
    for d in st.store.pg.docs(OUTCOME_KIND).await? {
        if let Ok(o) = serde_json::from_value::<Outcome>(d.body)
            && custom.contains(&o.rule_id)
        {
            out.push(o);
        }
    }
    Ok((f, out))
}

#[derive(Deserialize)]
pub struct OutcomesQuery {
    #[serde(rename = "filter[rule][id]")]
    rule: Option<String>,
    #[serde(rename = "filter[outcome][service_name]")]
    service: Option<String>,
    #[serde(rename = "filter[outcome][state]")]
    state: Option<String>,
}

/// `GET /api/v2/scorecard/outcomes`.
pub async fn list_outcomes(
    State(st): State<AppState>,
    Query(q): Query<OutcomesQuery>,
) -> ApiResult<Value> {
    let (_, all) = outcomes(&st).await?;
    let data: Vec<Value> = all
        .iter()
        .filter(|o| q.rule.as_ref().is_none_or(|r| *r == o.rule_id))
        .filter(|o| q.service.as_ref().is_none_or(|s| *s == o.service_name))
        .filter(|o| q.state.as_ref().is_none_or(|s| *s == o.state))
        .map(outcome_out)
        .collect();
    Ok(Json(json!({"data": data})))
}

/// The share of passed rules, skipped ones aside; `None` without any.
pub fn score<'a>(states: impl Iterator<Item = &'a str>) -> Option<f64> {
    let (mut pass, mut total) = (0, 0);
    for s in states {
        match s {
            "pass" => {
                pass += 1;
                total += 1
            }
            "fail" => total += 1,
            _ => {}
        }
    }
    (total > 0).then(|| pass as f64 / total as f64)
}

/// `GET /api/ui/scorecards`: rules, and per service its outcomes and scores.
pub async fn ui(State(st): State<AppState>) -> ApiResult<Value> {
    let (f, all) = outcomes(&st).await?;
    let mut rules: Vec<Value> = builtin_rules()
        .iter()
        .map(|(id, r)| rule_out(id, r, false, 0, 0))
        .collect();
    let custom = custom_rules(&st).await?;
    rules.extend(
        custom
            .iter()
            .map(|(id, r, c, u)| rule_out(id, r, true, *c, *u)),
    );
    let card_of: HashMap<String, String> = builtin_rules()
        .into_iter()
        .chain(custom.into_iter().map(|(id, r, ..)| (id, r)))
        .map(|(id, r)| (id, r.scorecard_name))
        .collect();
    let mut by_service: BTreeMap<String, Vec<&Outcome>> = BTreeMap::new();
    for o in &all {
        by_service
            .entry(o.service_name.clone())
            .or_default()
            .push(o);
    }
    let services: Vec<Value> = by_service
        .iter()
        .map(|(s, os)| {
            let mut cards: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
            for o in os {
                if let Some(c) = card_of.get(&o.rule_id) {
                    cards.entry(c).or_default().push(&o.state);
                }
            }
            json!({
                "service": s,
                "team": f.definitions.get(s).and_then(|d| d.team.clone()),
                "has_definition": f.definitions.contains_key(s),
                "score": score(os.iter().map(|o| o.state.as_str())),
                "scorecards": cards.iter().map(|(c, v)| (c.to_string(), json!(score(v.iter().copied())))).collect::<serde_json::Map<_, _>>(),
                "outcomes": os.iter().map(|o| (o.rule_id.clone(), json!({"state": o.state, "remarks": o.remarks}))).collect::<serde_json::Map<_, _>>(),
            })
        })
        .collect();
    Ok(Json(json!({"rules": rules, "services": services})))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rules_and_scores() {
        let mut f = Facts::default();
        f.services.insert("shop".into());
        f.versioned.insert("shop".into());
        f.correlated_logs.insert("shop".into(), 12);
        let def = super::super::definitions::parse(
            b"schema-version: v2.2\ndd-service: shop\nteam: storefront\nlinks:\n  - {name: src, type: repo, url: https://x}",
        )
        .unwrap();
        f.definitions
            .insert("shop".into(), super::super::definitions::summary(&def));
        assert_eq!(evaluate("ootb-deployment", "shop", &f).0, "pass");
        assert_eq!(evaluate("ootb-env", "shop", &f).0, "fail");
        assert_eq!(evaluate("ootb-logs", "shop", &f).0, "pass");
        assert_eq!(evaluate("ootb-team", "shop", &f).1, "team storefront");
        assert_eq!(evaluate("ootb-repo", "shop", &f).1, "https://x");
        assert_eq!(evaluate("ootb-docs", "shop", &f).0, "fail");
        assert_eq!(evaluate("ootb-monitors", "web", &f).0, "fail");
        assert_eq!(
            score(["pass", "fail", "skip", "pass"].into_iter()),
            Some(2.0 / 3.0)
        );
        assert_eq!(score(["skip"].into_iter()), None);
    }

    #[test]
    fn monitors_mention_services() {
        let m = json!({"query": "avg(last_5m):avg:trace.http.request.duration{service:shop-api} > 1", "tags": []});
        assert!(mentions(&m, "shop-api"));
        assert!(!mentions(&m, "shop"));
        assert!(mentions(
            &json!({"query": "x", "tags": ["service:web"]}),
            "web"
        ));
    }
}
