//! Watchdog Insights (what stands out in the errors of an explorer's
//! search), Root Cause Analysis (the anomalies of a story's downstream
//! dependencies, from the service map) and Impact Analysis (the RUM
//! sessions, users and views whose requests went through an anomalous
//! service, by trace id).

use crate::AppState;
use crate::api::analytics::{self, AggregateReq, Compute, Filter, GroupBy};
use crate::api::{ApiError, ApiResult, time_clause};
use crate::query::sql::Dataset;
use crate::store::greptime::{i64_at, lit, str_at};
use axum::Json;
use axum::extract::{Path, State};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};

#[derive(Debug, Deserialize)]
pub struct InsightReq {
    pub dataset: String,
    #[serde(default)]
    pub query: String,
    pub from: i64,
    pub to: i64,
}

/// A facet value over-represented in the focus (errors) against the whole:
/// (value, count in the focus, its share of the focus, its share of all).
pub fn standouts(all: &[(String, f64)], focus: &[(String, f64)]) -> Vec<(String, f64, f64, f64)> {
    let total_all: f64 = all.iter().map(|x| x.1).sum();
    let total_focus: f64 = focus.iter().map(|x| x.1).sum();
    if total_all <= 0.0 || total_focus < 5.0 {
        return vec![];
    }
    let base: HashMap<&str, f64> = all.iter().map(|(v, n)| (v.as_str(), *n)).collect();
    let mut out: Vec<_> = focus
        .iter()
        .filter_map(|(v, n)| {
            let sf = n / total_focus;
            let sa = base.get(v.as_str()).copied().unwrap_or(*n) / total_all;
            (*n >= 5.0 && sf >= 0.25 && sf >= 1.5 * sa).then(|| (v.clone(), *n, sf, sa))
        })
        .collect();
    out.sort_by(|a, b| (b.2 - b.3).total_cmp(&(a.2 - a.3)));
    out
}

async fn counts(
    st: &AppState,
    ds: Dataset,
    table: &str,
    query: &str,
    facet: &str,
    from: i64,
    to: i64,
) -> Result<Vec<(String, f64)>, ApiError> {
    let req = AggregateReq {
        compute: vec![Compute {
            aggregation: "count".into(),
            metric: None,
            typ: None,
            interval: None,
        }],
        filter: Filter {
            query: query.into(),
            from: json!(from),
            to: json!(to),
        },
        group_by: vec![GroupBy {
            facet: facet.into(),
            limit: Some(200),
            sort: None,
            missing: None,
        }],
    };
    let out = analytics::run_in(st, ds, table, &req, to).await?;
    Ok(out
        .buckets
        .iter()
        .map(|b| {
            let v = &b["by"][facet];
            (
                v.as_str()
                    .map(String::from)
                    .unwrap_or_else(|| v.to_string()),
                b["computes"]["c0"].as_f64().unwrap_or(0.0),
            )
        })
        .collect())
}

/// `POST /api/ui/insights`: `{"dataset": "logs|spans|rum", "query", "from", "to"}`.
pub async fn insights(State(st): State<AppState>, Json(r): Json<InsightReq>) -> ApiResult<Value> {
    let (ds, table, focus, facets): (Dataset, &str, &str, &[&str]) = match r.dataset.as_str() {
        "logs" => (
            Dataset::Logs,
            "logs",
            "status:error",
            &[
                "service",
                "host",
                "env",
                "source",
                "version",
                "@http.status_code",
            ],
        ),
        "spans" => (
            Dataset::Spans,
            "spans",
            "status:error",
            &[
                "service",
                "resource_name",
                "env",
                "version",
                "host",
                "@http.status_code",
            ],
        ),
        "rum" => (
            Dataset::Logs,
            "rum_events",
            "@type:error",
            &[
                "service",
                "version",
                "@view.name",
                "@browser.name",
                "@os.name",
                "@geo.country",
            ],
        ),
        _ => return Err(ApiError::BadQuery("dataset: logs, spans or rum".into())),
    };
    let base = r.query.trim();
    let base = if base.is_empty() || base == "*" {
        String::new()
    } else {
        base.to_string()
    };
    let subset = if base.is_empty() {
        focus.to_string()
    } else {
        format!("({base}) {focus}")
    };
    let mut out = Vec::new();
    for f in facets {
        let (all, foc) = tokio::try_join!(
            counts(
                &st,
                ds,
                table,
                if base.is_empty() { "*" } else { &base },
                f,
                r.from,
                r.to
            ),
            counts(&st, ds, table, &subset, f, r.from, r.to)
        )?;
        for (v, n, sf, sa) in standouts(&all, &foc).into_iter().take(2) {
            out.push(json!({"facet": f, "value": v, "count": n, "share": sf, "baseline": sa,
                "text": format!("{f}:{v} is in {:.0}% of the errors, against {:.0}% of everything", sf * 100.0, sa * 100.0),
                "query": format!("{focus} {f}:{}", if v.contains(' ') { format!("\"{v}\"") } else { v.clone() })}));
        }
    }
    out.sort_by(|a, b| {
        let lift =
            |x: &Value| x["share"].as_f64().unwrap_or(0.0) - x["baseline"].as_f64().unwrap_or(0.0);
        lift(b).total_cmp(&lift(a))
    });
    out.truncate(6);
    Ok(Json(json!({"insights": out})))
}

async fn story(st: &AppState, id: &str) -> Result<crate::watchdog::Story, ApiError> {
    crate::watchdog::stored(st)
        .await?
        .into_iter()
        .find(|s| s.id == id)
        .ok_or(ApiError::NotFound)
}

/// Services reachable downstream of `from`, nearest first, with their depth.
pub fn downstream(edges: &[(String, String)], from: &str) -> Vec<(String, usize)> {
    let mut seen = HashSet::from([from.to_string()]);
    let mut q = VecDeque::from([(from.to_string(), 0usize)]);
    let mut out = Vec::new();
    while let Some((s, d)) = q.pop_front() {
        for (a, b) in edges {
            if *a == s && seen.insert(b.clone()) {
                out.push((b.clone(), d + 1));
                q.push_back((b.clone(), d + 1));
            }
        }
    }
    out
}

async fn edges(st: &AppState, from: i64, to: i64) -> Result<Vec<(String, String)>, ApiError> {
    let range = serde_json::from_value(json!({"from": from, "to": to}))
        .map_err(|e| ApiError::Internal(e.into()))?;
    let Json(e) = crate::api::apm::map(State(st.clone()), axum::extract::Query(range)).await?;
    let v = serde_json::to_value(e).map_err(|e| ApiError::Internal(e.into()))?;
    Ok(v.as_array()
        .into_iter()
        .flatten()
        .filter_map(|e| {
            Some((
                e["source"].as_str()?.to_string(),
                e["target"].as_str()?.to_string(),
            ))
        })
        .collect())
}

/// `GET /api/ui/watchdog/stories/{id}/rca`.
pub async fn rca(State(st): State<AppState>, Path(id): Path<String>) -> ApiResult<Value> {
    let s = story(&st, &id).await?;
    let now = crate::intake::logs::now_ms();
    let (from, to) = (
        s.start - 900_000,
        s.end.unwrap_or(now).max(s.start + 900_000),
    );
    let e = edges(&st, from, to).await?;
    let all = crate::watchdog::stored(&st).await?;
    let overlapping = |svc: &str| -> Vec<&crate::watchdog::Story> {
        all.iter()
            .filter(|o| {
                o.id != s.id && o.service == svc && o.start <= to && o.end.unwrap_or(now) >= from
            })
            .collect()
    };
    let down = downstream(&e, &s.service);
    let mut chain = Vec::new();
    for (svc, depth) in &down {
        let found = overlapping(svc);
        if !found.is_empty() {
            chain.push(json!({"service": svc, "depth": depth, "stories": found}));
        }
    }
    // The deepest anomalous dependency that started first is the likeliest cause.
    let root = chain
        .iter()
        .max_by(|a, b| {
            let start = |x: &Value| {
                x["stories"]
                    .as_array()
                    .and_then(|s| s.iter().filter_map(|y| y["start"].as_i64()).min())
                    .unwrap_or(i64::MAX)
            };
            a["depth"]
                .as_u64()
                .cmp(&b["depth"].as_u64())
                .then(start(b).cmp(&start(a)))
        })
        .map(|c| c["service"].clone());
    let upstream: BTreeSet<String> = e
        .iter()
        .filter(|(_, b)| *b == s.service)
        .map(|(a, _)| a.clone())
        .collect();
    Ok(Json(
        json!({"story": s, "dependencies": down.iter().map(|(s, d)| json!({"service": s, "depth": d})).collect::<Vec<_>>(),
        "anomalous_dependencies": chain, "root_cause": root,
        "impacted_upstream": upstream.iter().map(|u| json!({"service": u, "stories": overlapping(u)})).collect::<Vec<_>>()}),
    ))
}

/// `GET /api/ui/watchdog/stories/{id}/impact`: RUM sessions and users
/// behind the anomaly (their resources' trace ids hit the service).
pub async fn impact(State(st): State<AppState>, Path(id): Path<String>) -> ApiResult<Value> {
    let s = story(&st, &id).await?;
    let now = crate::intake::logs::now_ms();
    let (from, to) = (s.start - 900_000, s.end.unwrap_or(now));
    let cond = match s.kind.as_str() {
        "apm_latency" => format!(" AND duration >= {}", (s.baseline * 2.0 * 1e6) as i64),
        _ => " AND error = true".into(),
    };
    let sql = format!(
        "SELECT DISTINCT trace_id FROM spans WHERE {} AND service = {}{cond} LIMIT 2000",
        time_clause(Dataset::Spans, from, to),
        lit(&s.service)
    );
    let traces: Vec<String> = st
        .store
        .gt
        .query(&sql)
        .await?
        .iter()
        .filter_map(|r| str_at(r, 0))
        .filter(|t| !t.is_empty())
        .collect();
    if traces.is_empty() {
        return Ok(Json(
            json!({"story": s, "traces": 0, "sessions": 0, "users": 0, "views": []}),
        ));
    }
    let at = |p: &str| format!("json_get_string(attributes, '{p}')");
    let ids = traces.iter().map(|t| lit(t)).collect::<Vec<_>>().join(",");
    let sql = format!(
        "SELECT {sid}, coalesce({un}, {ue}, {ui}), {view}, count(*) FROM rum_events WHERE {time} AND trace_id IN ({ids})
         GROUP BY 1, 2, 3 LIMIT 5000",
        sid = at(r#"$["session"]["id"]"#),
        un = at(r#"$["usr"]["name"]"#),
        ue = at(r#"$["usr"]["email"]"#),
        ui = at(r#"$["usr"]["id"]"#),
        view = at(r#"$["view"]["url_path"]"#),
        time = time_clause(Dataset::Logs, from, to),
    );
    let rows = st.store.gt.query(&sql).await?;
    let mut sessions = HashSet::new();
    let mut users = HashSet::new();
    let mut views: HashMap<String, (i64, HashSet<String>)> = HashMap::new();
    for r in &rows {
        let sid = str_at(r, 0).unwrap_or_default();
        if let Some(u) = str_at(r, 1).filter(|u| !u.is_empty()) {
            users.insert(u);
        }
        let v = views
            .entry(str_at(r, 2).unwrap_or_else(|| "(unknown)".into()))
            .or_default();
        v.0 += i64_at(r, 3);
        v.1.insert(sid.clone());
        sessions.insert(sid);
    }
    let mut views: Vec<Value> = views
        .into_iter()
        .map(|(v, (n, s))| json!({"view": v, "requests": n, "sessions": s.len()}))
        .collect();
    views.sort_by_key(|v| std::cmp::Reverse(v["sessions"].as_u64().unwrap_or(0)));
    views.truncate(20);
    Ok(Json(
        json!({"story": s, "traces": traces.len(), "sessions": sessions.len(), "users": users.len(), "views": views}),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standout_values() {
        let all = vec![("a".to_string(), 900.0), ("b".to_string(), 100.0)];
        let focus = vec![("a".to_string(), 10.0), ("b".to_string(), 40.0)];
        let s = standouts(&all, &focus);
        assert_eq!(s.len(), 1);
        assert_eq!((s[0].0.as_str(), s[0].1), ("b", 40.0));
        assert!(standouts(&all, &[("b".to_string(), 3.0)]).is_empty());
    }

    #[test]
    fn dependencies() {
        let e: Vec<(String, String)> = [
            ("web", "api"),
            ("api", "db"),
            ("api", "cache"),
            ("db", "disk"),
            ("other", "db"),
        ]
        .iter()
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .collect();
        assert_eq!(
            downstream(&e, "web"),
            vec![
                ("api".into(), 1),
                ("db".into(), 2),
                ("cache".into(), 2),
                ("disk".into(), 3)
            ]
        );
    }
}
