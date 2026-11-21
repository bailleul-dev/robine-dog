//! Cloud SIEM: detection rules on logs (threshold on a count by group, or a
//! new value of an attribute) evaluated every minute, which open security
//! signals to triage. Rules use Datadog's `/api/v2/security_monitoring/rules`
//! shape: `queries`, `cases` (`a > 10` → severity) and `options`.

use crate::AppState;
use crate::api::analytics::{self, AggregateReq, Compute, Filter, GroupBy};
use crate::api::{ApiError, ApiResult};
use crate::query::sql::Dataset;
use axum::Json;
use axum::extract::{Path, Query, State};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashSet};

const KIND: &str = "security_rule";

/// `a > 10`, `a >= 1`… → (operator, value).
pub fn condition(c: &str) -> Option<(&'static str, f64)> {
    let c = c.trim();
    for op in [">=", "<=", "==", ">", "<"] {
        if let Some((_, v)) = c.split_once(op) {
            return Some((
                match op {
                    ">=" => ">=",
                    "<=" => "<=",
                    "==" => "==",
                    ">" => ">",
                    _ => "<",
                },
                v.trim().parse().ok()?,
            ));
        }
    }
    None
}

fn holds(op: &str, v: f64, t: f64) -> bool {
    match op {
        ">=" => v >= t,
        "<=" => v <= t,
        "==" => v == t,
        ">" => v > t,
        _ => v < t,
    }
}

/// The severity of the first case a value meets.
pub fn severity(rule: &Value, v: f64) -> Option<String> {
    rule["cases"].as_array()?.iter().find_map(|c| {
        let status = c["status"].as_str().unwrap_or("info").to_string();
        match c["condition"].as_str().filter(|x| !x.trim().is_empty()) {
            None => Some(status),
            Some(cond) => condition(cond)
                .filter(|(op, t)| holds(op, v, *t))
                .map(|_| status),
        }
    })
}

fn validate(rule: &Value) -> Result<(), String> {
    if rule["name"].as_str().unwrap_or("").trim().is_empty() {
        return Err("a rule needs a name".into());
    }
    let q = rule["queries"].get(0).ok_or("a rule needs a query")?;
    crate::query::parser::parse(q["query"].as_str().unwrap_or(""))
        .map_err(|e| format!("query: {e}"))?;
    match q["aggregation"].as_str().unwrap_or("count") {
        "count" => {}
        "new_value" if q["metrics"].get(0).and_then(|m| m.as_str()).is_some() => {}
        "new_value" => return Err("a new_value query needs metrics: [the attribute]".into()),
        a => {
            return Err(format!(
                "aggregation {a} is not supported: count, new_value"
            ));
        }
    }
    let cases = rule["cases"]
        .as_array()
        .filter(|c| !c.is_empty())
        .ok_or("a rule needs a case")?;
    for c in cases {
        if let Some(cond) = c["condition"].as_str().filter(|x| !x.trim().is_empty())
            && condition(cond).is_none()
        {
            return Err(format!("condition {cond}: expected a > N"));
        }
    }
    Ok(())
}

fn defaults() -> Vec<(&'static str, Value)> {
    vec![
        (
            "ootb-ssh-bruteforce",
            json!({"name": "SSH brute force", "isEnabled": true, "type": "log_detection",
                "message": "Many failed SSH logins from one address: someone may be guessing passwords.",
                "tags": ["source:ssh", "tactic:TA0006-credential-access"],
                "queries": [{"name": "a", "query": "\"Failed password\" OR \"authentication failure\"",
                    "groupByFields": ["@network.client.ip"], "aggregation": "count"}],
                "cases": [{"name": "many", "status": "high", "condition": "a > 20"},
                    {"name": "some", "status": "medium", "condition": "a > 5"}],
                "options": {"evaluationWindow": 300, "keepAlive": 3600, "maxSignalDuration": 86400}}),
        ),
        (
            "ootb-401-burst",
            json!({"name": "Burst of unauthorized requests", "isEnabled": true, "type": "log_detection",
                "message": "Many 401 or 403 answers to one client: credential stuffing or a broken client.",
                "tags": ["tactic:TA0006-credential-access"],
                "queries": [{"name": "a", "query": "@http.status_code:(401 OR 403)",
                    "groupByFields": ["@network.client.ip"], "aggregation": "count"}],
                "cases": [{"name": "burst", "status": "medium", "condition": "a > 50"}],
                "options": {"evaluationWindow": 300, "keepAlive": 3600, "maxSignalDuration": 86400}}),
        ),
        (
            "ootb-new-admin-user",
            json!({"name": "New user logging in as admin", "isEnabled": true, "type": "log_detection",
                "message": "A user never seen before used an admin route.",
                "tags": ["tactic:TA0004-privilege-escalation"],
                "queries": [{"name": "a", "query": "@http.url_details.path:/admin*",
                    "groupByFields": [], "aggregation": "new_value", "metrics": ["@usr.id"]}],
                "cases": [{"name": "new", "status": "low", "condition": ""}],
                "options": {"evaluationWindow": 300, "keepAlive": 3600, "maxSignalDuration": 86400,
                    "newValueOptions": {"learningDuration": 7}}}),
        ),
    ]
}

pub async fn init(st: &AppState) -> anyhow::Result<()> {
    let have: HashSet<String> = st
        .store
        .pg
        .docs(KIND)
        .await?
        .into_iter()
        .map(|d| d.id)
        .collect();
    for (id, r) in defaults() {
        if !have.contains(id) {
            st.store.pg.put_doc(KIND, id, &r).await?;
        }
    }
    Ok(())
}

fn rule_json(d: &crate::store::pg::Doc) -> Value {
    let mut v = d.body.clone();
    v["id"] = json!(d.id);
    v["createdAt"] = json!(d.created_at);
    v["isDefault"] = json!(d.id.starts_with("ootb-"));
    v
}

/// Counts by group over `[from, to]`.
async fn counts(
    st: &AppState,
    query: &str,
    by: &[String],
    from: i64,
    to: i64,
) -> anyhow::Result<Vec<(BTreeMap<String, String>, f64)>> {
    let req = AggregateReq {
        compute: vec![Compute {
            aggregation: "count".into(),
            metric: None,
            typ: None,
            interval: None,
        }],
        filter: Filter {
            query: query.to_string(),
            from: json!(from),
            to: json!(to),
        },
        group_by: by
            .iter()
            .map(|f| GroupBy {
                facet: f.clone(),
                limit: Some(1000),
                sort: None,
                missing: None,
            })
            .collect(),
    };
    let out = analytics::run_in(st, Dataset::Logs, "logs", &req, to)
        .await
        .map_err(|e| anyhow::anyhow!("{e:?}"))?;
    Ok(out
        .buckets
        .iter()
        .map(|b| {
            let tags = by
                .iter()
                .map(|f| {
                    (
                        f.clone(),
                        b["by"][f]
                            .as_str()
                            .map(String::from)
                            .unwrap_or_else(|| b["by"][f].to_string()),
                    )
                })
                .collect();
            (tags, b["computes"]["c0"].as_f64().unwrap_or(0.0))
        })
        .collect())
}

fn group_key(tags: &BTreeMap<String, String>) -> String {
    if tags.is_empty() {
        return "*".into();
    }
    tags.iter()
        .map(|(k, v)| format!("{k}:{v}"))
        .collect::<Vec<_>>()
        .join(",")
}

async fn open_signal(
    st: &AppState,
    rule_id: &str,
    rule: &Value,
    tags: &BTreeMap<String, String>,
    sev: &str,
    value: f64,
    now: i64,
) -> anyhow::Result<()> {
    let key = group_key(tags);
    let keep_alive = rule["options"]["keepAlive"].as_i64().unwrap_or(3600) * 1000;
    let max_duration = rule["options"]["maxSignalDuration"]
        .as_i64()
        .unwrap_or(86400)
        * 1000;
    let c = st.store.pg.conn().await?;
    let updated = c
        .execute(
            "UPDATE security_signals SET last_seen = $3, value = greatest(value, $4), severity = $5
             WHERE id = (SELECT id FROM security_signals WHERE rule_id = $1 AND group_key = $2
                AND last_seen >= $6 AND first_seen >= $7 ORDER BY id DESC LIMIT 1)",
            &[
                &rule_id,
                &key,
                &now,
                &value,
                &sev,
                &(now - keep_alive),
                &(now - max_duration),
            ],
        )
        .await?;
    if updated == 0 {
        let q = rule["queries"][0]["query"].as_str().unwrap_or("");
        c.execute(
            "INSERT INTO security_signals (rule_id, rule_name, severity, group_key, group_tags, query, message, value,
                first_seen, last_seen) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $9)",
            &[
                &rule_id,
                &rule["name"].as_str().unwrap_or(""),
                &sev,
                &key,
                &json!(tags),
                &q,
                &rule["message"].as_str().unwrap_or(""),
                &value,
                &now,
            ],
        )
        .await?;
        tracing::info!(rule = rule_id, group = %key, severity = sev, "security signal");
        let mut t = vec![format!("rule_id:{rule_id}"), format!("severity:{sev}")];
        t.extend(
            tags.iter()
                .map(|(k, v)| format!("{}:{v}", k.trim_start_matches('@'))),
        );
        st.aggregator.add(
            now,
            "datadog.security.signals",
            None,
            t,
            crate::metrics::Sample::Count(1.0),
        );
    }
    Ok(())
}

pub async fn evaluate(
    st: &AppState,
    rule_id: &str,
    rule: &Value,
    now: i64,
) -> anyhow::Result<usize> {
    let q = &rule["queries"][0];
    let query = q["query"].as_str().unwrap_or("");
    let by: Vec<String> = q["groupByFields"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let window = rule["options"]["evaluationWindow"].as_i64().unwrap_or(300) * 1000;
    let mut n = 0;
    if q["aggregation"].as_str() == Some("new_value") {
        let field = q["metrics"][0].as_str().unwrap_or("").to_string();
        let mut with = by.clone();
        with.push(field.clone());
        let learning = rule["options"]["newValueOptions"]["learningDuration"]
            .as_i64()
            .unwrap_or(7)
            * 86_400_000;
        let recent = counts(st, query, &with, now - window, now).await?;
        if recent.is_empty() {
            return Ok(0);
        }
        let known: HashSet<String> = counts(st, query, &with, now - learning, now - window)
            .await?
            .into_iter()
            .map(|(t, _)| group_key(&t))
            .collect();
        // Nothing learned yet: everything would look new.
        if known.is_empty() {
            return Ok(0);
        }
        for (tags, v) in recent {
            if !known.contains(&group_key(&tags))
                && let Some(sev) = severity(rule, v)
            {
                open_signal(st, rule_id, rule, &tags, &sev, v, now).await?;
                n += 1;
            }
        }
    } else {
        for (tags, v) in counts(st, query, &by, now - window, now).await? {
            if let Some(sev) = severity(rule, v) {
                open_signal(st, rule_id, rule, &tags, &sev, v, now).await?;
                n += 1;
            }
        }
    }
    Ok(n)
}

pub fn spawn(st: AppState) {
    tokio::spawn(async move {
        if let Err(e) = init(&st).await {
            tracing::warn!(error = %e, "creating default detection rules");
        }
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(60));
        loop {
            tick.tick().await;
            let now = crate::intake::logs::now_ms();
            let Ok(rules) = st.store.pg.docs(KIND).await else {
                continue;
            };
            for d in rules {
                if d.body["isEnabled"].as_bool() == Some(false) {
                    continue;
                }
                if let Err(e) = evaluate(&st, &d.id, &d.body, now).await {
                    tracing::warn!(rule = %d.id, error = %e, "evaluating a detection rule");
                }
            }
        }
    });
}

// ---------------------------------------------------------------- API

pub async fn list_rules(State(st): State<AppState>) -> ApiResult<Value> {
    let docs = st.store.pg.docs(KIND).await?;
    let data: Vec<Value> = docs.iter().map(rule_json).collect();
    Ok(Json(
        json!({"data": data, "meta": {"page": {"total_count": data.len()}}}),
    ))
}

pub async fn get_rule(State(st): State<AppState>, Path(id): Path<String>) -> ApiResult<Value> {
    let d = st
        .store
        .pg
        .doc(KIND, &id)
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(rule_json(&d)))
}

fn clean(mut v: Value) -> Value {
    if let Some(o) = v.as_object_mut() {
        for k in ["id", "createdAt", "isDefault"] {
            o.remove(k);
        }
        o.entry("isEnabled").or_insert(json!(true));
        o.entry("type").or_insert(json!("log_detection"));
        o.entry("options").or_insert(
            json!({"evaluationWindow": 300, "keepAlive": 3600, "maxSignalDuration": 86400}),
        );
    }
    v
}

pub async fn create_rule(State(st): State<AppState>, Json(v): Json<Value>) -> ApiResult<Value> {
    let v = clean(v);
    validate(&v).map_err(ApiError::BadQuery)?;
    let d = st
        .store
        .pg
        .put_doc(KIND, &crate::store::pg::new_id(), &v)
        .await?;
    Ok(Json(rule_json(&d)))
}

pub async fn update_rule(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Json(v): Json<Value>,
) -> ApiResult<Value> {
    let old = st
        .store
        .pg
        .doc(KIND, &id)
        .await?
        .ok_or(ApiError::NotFound)?;
    let mut body = old.body;
    if let (Some(b), Some(n)) = (body.as_object_mut(), clean(v).as_object()) {
        for (k, x) in n {
            b.insert(k.clone(), x.clone());
        }
    }
    validate(&body).map_err(ApiError::BadQuery)?;
    let d = st.store.pg.put_doc(KIND, &id, &body).await?;
    Ok(Json(rule_json(&d)))
}

pub async fn delete_rule(State(st): State<AppState>, Path(id): Path<String>) -> ApiResult<Value> {
    if !st.store.pg.delete_doc(KIND, &id).await? {
        return Err(ApiError::NotFound);
    }
    Ok(Json(json!({})))
}

#[derive(Debug, Deserialize)]
pub struct SignalFilter {
    #[serde(alias = "filter[from]")]
    pub from: Option<i64>,
    #[serde(alias = "filter[to]")]
    pub to: Option<i64>,
    pub status: Option<String>,
    pub severity: Option<String>,
    #[serde(alias = "filter[query]")]
    pub query: Option<String>,
}

fn signal_json(r: &tokio_postgres::Row) -> Value {
    json!({"id": r.get::<_, i64>(0).to_string(), "type": "signal", "attributes": {
        "rule_id": r.get::<_, String>(1), "title": r.get::<_, String>(2), "severity": r.get::<_, String>(3),
        "status": r.get::<_, String>(4), "group": r.get::<_, String>(5), "group_tags": r.get::<_, Value>(6),
        "query": r.get::<_, String>(7), "message": r.get::<_, String>(8), "value": r.get::<_, f64>(9),
        "first_seen": r.get::<_, i64>(10), "timestamp": r.get::<_, i64>(11),
        "assignee": r.get::<_, Option<String>>(12), "archive_reason": r.get::<_, Option<String>>(13),
    }})
}

const SIGNAL_COLS: &str =
    "id, rule_id, rule_name, severity, status, group_key, group_tags, query, message, value,
    first_seen, last_seen, assignee, archive_reason";

/// `GET /api/v2/security_monitoring/signals`.
pub async fn signals(
    State(st): State<AppState>,
    Query(f): Query<SignalFilter>,
) -> ApiResult<Value> {
    let now = crate::intake::logs::now_ms();
    let c = st.store.pg.conn().await?;
    let pattern = f
        .query
        .as_deref()
        .filter(|q| !q.trim().is_empty())
        .map(|q| format!("%{}%", q.trim()));
    let rows = c
        .query(
            &format!(
                "SELECT {SIGNAL_COLS} FROM security_signals WHERE last_seen >= $1 AND first_seen <= $2
                   AND ($3::TEXT IS NULL OR status = $3) AND ($4::TEXT IS NULL OR severity = $4)
                   AND ($5::TEXT IS NULL OR rule_name ILIKE $5 OR group_key ILIKE $5)
                 ORDER BY last_seen DESC LIMIT 500"
            ),
            &[&f.from.unwrap_or(now - 7 * 86_400_000), &f.to.unwrap_or(now), &f.status, &f.severity, &pattern],
        )
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    Ok(Json(
        json!({"data": rows.iter().map(signal_json).collect::<Vec<_>>()}),
    ))
}

/// `PATCH /api/v2/security_monitoring/signals/{id}/state` and `/assignee`.
pub async fn triage(
    State(st): State<AppState>,
    Path((id, what)): Path<(i64, String)>,
    Json(v): Json<Value>,
) -> ApiResult<Value> {
    let a = &v["data"]["attributes"];
    let c = st.store.pg.conn().await?;
    let n = match what.as_str() {
        "state" => {
            let state = a["state"].as_str().unwrap_or("");
            if !["open", "under_review", "archived"].contains(&state) {
                return Err(ApiError::BadQuery(
                    "state: open, under_review or archived".into(),
                ));
            }
            c.execute(
                "UPDATE security_signals SET status = $2, archive_reason = $3 WHERE id = $1",
                &[&id, &state, &a["archive_reason"].as_str()],
            )
            .await
        }
        "assignee" => {
            let who = a["assignee"]["uuid"]
                .as_str()
                .or(a["assignee"].as_str())
                .filter(|x| !x.is_empty());
            c.execute(
                "UPDATE security_signals SET assignee = $2 WHERE id = $1",
                &[&id, &who],
            )
            .await
        }
        _ => return Err(ApiError::NotFound),
    }
    .map_err(|e| ApiError::Internal(e.into()))?;
    if n == 0 {
        return Err(ApiError::NotFound);
    }
    let r = c
        .query_one(
            &format!("SELECT {SIGNAL_COLS} FROM security_signals WHERE id = $1"),
            &[&id],
        )
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    Ok(Json(json!({"data": signal_json(&r)})))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cases_and_validation() {
        let (_, r) = &defaults()[0];
        assert_eq!(severity(r, 30.0).as_deref(), Some("high"));
        assert_eq!(severity(r, 6.0).as_deref(), Some("medium"));
        assert_eq!(severity(r, 2.0), None);
        for (_, r) in defaults() {
            validate(&r).unwrap();
        }
        assert!(validate(&json!({"name": "x", "queries": [{"query": "a", "aggregation": "avg"}], "cases": [{}]})).is_err());
        assert_eq!(condition("a >= 3"), Some((">=", 3.0)));
    }
}
