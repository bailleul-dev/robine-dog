//! RUM analyses: frustration signals (the rage, dead and error clicks the
//! SDK flags in `@action.frustration.type`), and retention (of the users who
//! did something, how many came back each following day or week).

use super::TABLE;
use crate::AppState;
use crate::api::{ApiError, ApiResult, Scope, compile, time_clause};
use crate::query::sql::Dataset;
use crate::store::greptime::{i64_at, str_at};
use axum::Json;
use axum::extract::State;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap, HashSet};

pub const FRUSTRATIONS: &[&str] = &["rage_click", "dead_click", "error_click"];

/// `["rage_click","error_click"]` → its types.
pub fn frustration_types(raw: &str) -> Vec<String> {
    serde_json::from_str::<Vec<String>>(raw)
        .unwrap_or_default()
        .into_iter()
        .filter(|t| FRUSTRATIONS.contains(&t.as_str()))
        .collect()
}

#[derive(Default)]
struct Tally {
    total: i64,
    by_type: BTreeMap<String, i64>,
    sessions: HashSet<String>,
}

impl Tally {
    fn add(&mut self, types: &[String], session: Option<&str>) {
        self.total += 1;
        for t in types {
            *self.by_type.entry(t.clone()).or_default() += 1;
        }
        if let Some(s) = session {
            self.sessions.insert(s.to_string());
        }
    }

    fn json(&self) -> Value {
        json!({"count": self.total, "by_type": self.by_type, "sessions": self.sessions.len()})
    }
}

/// `POST /api/ui/rum/frustrations`: the frustrated clicks of the range, by
/// type, page and element, and the latest ones.
pub async fn frustrations(State(st): State<AppState>, Json(s): Json<Scope>) -> ApiResult<Value> {
    let filter = compile(&s.query, Dataset::Logs)?;
    let time = time_clause(Dataset::Logs, s.from, s.to);
    let types = r#"json_get(attributes, '$["action"]["frustration"]["type"]')"#;
    let at = |p: &str| format!("json_get_string(attributes, '{p}')");
    let user = format!(
        "coalesce({}, {}, {})",
        at(r#"$["usr"]["name"]"#),
        at(r#"$["usr"]["email"]"#),
        at(r#"$["usr"]["id"]"#)
    );
    let sql = format!(
        r#"SELECT {types}, coalesce({target}, message), {path}, {sid}, service, CAST(ts AS BIGINT) AS t, {user}
           FROM {TABLE} WHERE {time} AND source = 'action' AND {filter} AND {types} LIKE '%_click%'
           ORDER BY t DESC LIMIT 20000"#,
        target = at(r#"$["action"]["target"]["name"]"#),
        path = at(r#"$["view"]["url_path"]"#),
        sid = at(r#"$["session"]["id"]"#),
        user = user,
    );
    let (rows, sessions_total) = tokio::try_join!(st.store.gt.query(&sql), async {
        let q = format!(
            r#"SELECT count(DISTINCT {}) FROM {TABLE} WHERE {time} AND {filter}"#,
            at(r#"$["session"]["id"]"#)
        );
        Ok::<_, anyhow::Error>(
            st.store
                .gt
                .query(&q)
                .await?
                .first()
                .map_or(0, |r| i64_at(r, 0)),
        )
    })?;
    let mut all = Tally::default();
    let mut pages: HashMap<String, Tally> = HashMap::new();
    let mut targets: HashMap<(String, String), Tally> = HashMap::new();
    let mut latest = Vec::new();
    for r in &rows {
        let ft = frustration_types(&str_at(r, 0).unwrap_or_default());
        if ft.is_empty() {
            continue;
        }
        let target = str_at(r, 1).unwrap_or_default();
        let path = str_at(r, 2).unwrap_or_else(|| "(unknown)".into());
        let session = str_at(r, 3);
        all.add(&ft, session.as_deref());
        pages
            .entry(path.clone())
            .or_default()
            .add(&ft, session.as_deref());
        targets
            .entry((path.clone(), target.clone()))
            .or_default()
            .add(&ft, session.as_deref());
        if latest.len() < 50 {
            latest.push(
                json!({"ts": i64_at(r, 5), "types": ft, "target": target, "view": path,
                "session_id": session, "service": str_at(r, 4), "user": str_at(r, 6)}),
            );
        }
    }
    let top = |m: HashMap<String, Tally>| {
        let mut v: Vec<(String, Tally)> = m.into_iter().collect();
        v.sort_by(|a, b| b.1.total.cmp(&a.1.total).then_with(|| a.0.cmp(&b.0)));
        v.truncate(50);
        v
    };
    let mut target_list: Vec<((String, String), Tally)> = targets.into_iter().collect();
    target_list.sort_by_key(|t| std::cmp::Reverse(t.1.total));
    target_list.truncate(50);
    Ok(Json(json!({
        "total": all.json(),
        "sessions": sessions_total,
        "pages": top(pages).into_iter().map(|(p, t)| {
            let mut v = t.json();
            v["view"] = json!(p);
            v
        }).collect::<Vec<_>>(),
        "targets": target_list.into_iter().map(|((p, target), t)| {
            let mut v = t.json();
            v["view"] = json!(p);
            v["target"] = json!(target);
            v
        }).collect::<Vec<_>>(),
        "latest": latest,
    })))
}

#[derive(Debug, Deserialize)]
pub struct RetentionReq {
    /// Who counts: users entering the cohort with an event matching it.
    #[serde(default)]
    pub start_query: String,
    /// And coming back with an event matching this (the same by default).
    #[serde(default)]
    pub return_query: Option<String>,
    /// `day` or `week`.
    #[serde(default = "day")]
    pub period: String,
    /// The user attribute: `@usr.id` by default.
    #[serde(default = "user_id")]
    pub by: String,
    pub from: i64,
    pub to: i64,
}

fn day() -> String {
    "day".into()
}

fn user_id() -> String {
    "@usr.id".into()
}

/// A period's number: days since the epoch, or weeks starting on Mondays.
pub fn period_of(ms: i64, week: bool) -> i64 {
    let d = ms.div_euclid(86_400_000);
    if week {
        // 1970-01-01 was a Thursday.
        (d + 3).div_euclid(7)
    } else {
        d
    }
}

pub fn period_start(p: i64, week: bool) -> i64 {
    if week {
        (p * 7 - 3) * 86_400_000
    } else {
        p * 86_400_000
    }
}

/// Cohorts by first start period: their size, and how many of them were
/// back (with a return event) in each period after.
pub fn cohorts(
    starts: &[(String, i64)],
    returns: &[(String, i64)],
    week: bool,
    last: i64,
) -> Vec<(i64, usize, Vec<usize>)> {
    let mut first: HashMap<&str, i64> = HashMap::new();
    for (u, t) in starts {
        let p = period_of(*t, week);
        first
            .entry(u.as_str())
            .and_modify(|f| *f = (*f).min(p))
            .or_insert(p);
    }
    let mut active: HashMap<&str, HashSet<i64>> = HashMap::new();
    for (u, t) in returns {
        active
            .entry(u.as_str())
            .or_default()
            .insert(period_of(*t, week));
    }
    let mut by_cohort: BTreeMap<i64, Vec<&str>> = BTreeMap::new();
    for (u, p) in &first {
        by_cohort.entry(*p).or_default().push(u);
    }
    by_cohort
        .into_iter()
        .map(|(c, users)| {
            let periods = (last - c).max(0) as usize;
            let back = (1..=periods)
                .map(|k| {
                    users
                        .iter()
                        .filter(|u| active.get(*u).is_some_and(|a| a.contains(&(c + k as i64))))
                        .count()
                })
                .collect();
            (c, users.len(), back)
        })
        .collect()
}

/// `POST /api/ui/rum/retention`.
pub async fn retention(
    State(st): State<AppState>,
    Json(r): Json<RetentionReq>,
) -> ApiResult<Value> {
    let week = match r.period.as_str() {
        "day" => false,
        "week" => true,
        _ => return Err(ApiError::BadQuery("period is day or week".into())),
    };
    if !r.by.starts_with('@') {
        return Err(ApiError::BadQuery(
            "by is an attribute, like @usr.id".into(),
        ));
    }
    let starts = crate::api::analytics::facet_times(
        &st,
        Dataset::Logs,
        TABLE,
        &r.start_query,
        &r.by,
        r.from,
        r.to,
        500_000,
    )
    .await?;
    let returns = match r.return_query.as_deref().map(str::trim) {
        Some(q) if !q.is_empty() && q != r.start_query.trim() => {
            crate::api::analytics::facet_times(
                &st,
                Dataset::Logs,
                TABLE,
                q,
                &r.by,
                r.from,
                r.to,
                500_000,
            )
            .await?
        }
        _ => starts.clone(),
    };
    let last = period_of(r.to, week);
    let rows = cohorts(&starts, &returns, week, last);
    Ok(Json(json!({
        "period": r.period,
        "cohorts": rows.into_iter().map(|(c, size, back)| json!({
            "start": period_start(c, week),
            "users": size,
            "returned": back,
            "rates": back.iter().map(|b| if size == 0 { 0.0 } else { *b as f64 / size as f64 * 100.0 }).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frustration_lists() {
        assert_eq!(
            frustration_types(r#"["rage_click","error_click"]"#),
            vec!["rage_click", "error_click"]
        );
        assert!(frustration_types("null").is_empty());
        assert!(frustration_types(r#"["other"]"#).is_empty());
    }

    #[test]
    fn periods_and_cohorts() {
        // 2026-11-16 was a Monday.
        let monday = 1_794_787_200_000;
        assert_eq!(
            period_of(monday, true),
            period_of(monday + 6 * 86_400_000, true)
        );
        assert_ne!(period_of(monday - 1, true), period_of(monday, true));
        assert_eq!(
            period_start(period_of(monday + 3 * 86_400_000, true), true),
            monday
        );
        let day = 86_400_000;
        let ev = |u: &str, d: i64| (u.to_string(), d * day + 1000);
        let starts = vec![ev("a", 10), ev("b", 10), ev("c", 11), ev("a", 12)];
        let returns = vec![
            ev("a", 10),
            ev("a", 11),
            ev("b", 12),
            ev("c", 12),
            ev("a", 12),
        ];
        let c = cohorts(&starts, &returns, false, 12);
        // Day 10: a and b; a back on day 11, both on day 12. Day 11: c, back on 12.
        assert_eq!(
            c,
            vec![(10, 2, vec![1, 2]), (11, 1, vec![1]), (12, 0, vec![])]
                .into_iter()
                .filter(|x| x.1 > 0)
                .collect::<Vec<_>>()
        );
    }
}
