//! Data for the dashboard widgets that need more than a timeseries or a
//! scalar query: distributions (histograms of a measure, or of the groups of
//! a metric query) and RUM funnels.

use super::analytics;
use super::query::{FormulaDef, QueryDef, Request, execute};
use super::{ApiError, ApiResult};
use crate::AppState;
use crate::query::sql::Dataset;
use axum::Json;
use axum::extract::State;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;

fn source(ds: &str) -> Result<(Dataset, &'static str), ApiError> {
    Ok(match ds {
        "logs" => (Dataset::Logs, "logs"),
        "spans" => (Dataset::Spans, "spans"),
        "rum" => (Dataset::Logs, "rum_events"),
        "events" => (Dataset::Logs, "events"),
        other => {
            return Err(ApiError::BadQuery(format!(
                "unsupported data_source {other}"
            )));
        }
    })
}

#[derive(Deserialize)]
pub struct DistributionReq {
    from: i64,
    to: i64,
    #[serde(default = "metrics")]
    data_source: String,
    /// Metrics: the query; its groups' averages are distributed.
    #[serde(default)]
    query: String,
    /// Events: the search, and the measure distributed.
    #[serde(default)]
    search: String,
    #[serde(default)]
    metric: Option<String>,
    #[serde(default)]
    buckets: Option<usize>,
}

fn metrics() -> String {
    "metrics".into()
}

/// Buckets of equal width between the lowest and highest value, and the
/// usual percentiles.
pub fn histogram(mut values: Vec<f64>, n: usize) -> Value {
    values.retain(|v| v.is_finite());
    values.sort_by(f64::total_cmp);
    let count = values.len();
    if count == 0 {
        return json!({"buckets": [], "count": 0});
    }
    let (lo, hi) = (values[0], values[count - 1]);
    let n = if hi > lo { n.clamp(1, 200) } else { 1 };
    let width = if hi > lo { (hi - lo) / n as f64 } else { 1.0 };
    let mut counts = vec![0u64; n];
    for v in &values {
        let i = (((v - lo) / width) as usize).min(n - 1);
        counts[i] += 1;
    }
    let pct = |p: f64| values[((count - 1) as f64 * p).round() as usize];
    json!({
        "buckets": counts.iter().enumerate().map(|(i, c)| json!({
            "lower": lo + i as f64 * width,
            "upper": if hi > lo { lo + (i + 1) as f64 * width } else { hi },
            "count": c,
        })).collect::<Vec<_>>(),
        "count": count,
        "min": lo,
        "max": hi,
        "avg": values.iter().sum::<f64>() / count as f64,
        "p50": pct(0.5),
        "p75": pct(0.75),
        "p90": pct(0.9),
        "p95": pct(0.95),
        "p99": pct(0.99),
    })
}

pub async fn distribution(
    State(st): State<AppState>,
    Json(r): Json<DistributionReq>,
) -> ApiResult<Value> {
    let values = if r.data_source == "metrics" {
        let q: QueryDef = serde_json::from_value(
            json!({"data_source": "metrics", "name": "a", "query": r.query, "aggregator": "avg"}),
        )
        .map_err(|e| ApiError::BadQuery(e.to_string()))?;
        let req = Request {
            from: r.from,
            to: r.to,
            interval: None,
            queries: vec![q],
            formulas: vec![FormulaDef {
                formula: "a".into(),
                alias: None,
                limit: None,
            }],
        };
        execute(&st, &req, true)
            .await?
            .series
            .into_iter()
            .filter_map(|(_, _, s)| s.values.first().copied().flatten())
            .collect()
    } else {
        let (ds, table) = source(&r.data_source)?;
        let metric = r.metric.as_deref().unwrap_or("@duration");
        let mut v =
            analytics::values(&st, ds, table, &r.search, metric, r.from, r.to, 100_000).await?;
        // Durations are stored in nanoseconds.
        if ds == Dataset::Spans && metric == "@duration" {
            v.iter_mut().for_each(|x| *x /= 1e9);
        }
        v
    };
    Ok(Json(histogram(values, r.buckets.unwrap_or(30))))
}

#[derive(Deserialize)]
pub struct FunnelStep {
    /// What a step is: `@type:view @view.name:/cart`.
    search: String,
    #[serde(default)]
    label: Option<String>,
}

#[derive(Deserialize)]
pub struct FunnelReq {
    from: i64,
    to: i64,
    /// Applied to every step (`@application.id:… env:prod`).
    #[serde(default)]
    query: String,
    steps: Vec<FunnelStep>,
}

/// Sessions that went through the steps in order; the times of each
/// session's events per step, oldest first.
pub fn funnel_counts(steps: &[HashMap<String, Vec<i64>>]) -> Vec<usize> {
    let mut out = vec![0; steps.len()];
    let Some(first) = steps.first() else {
        return out;
    };
    for (session, times) in first {
        let Some(mut at) = times.first().copied() else {
            continue;
        };
        out[0] += 1;
        for (i, step) in steps.iter().enumerate().skip(1) {
            match step
                .get(session)
                .and_then(|ts| ts.iter().find(|t| **t >= at))
            {
                Some(t) => {
                    at = *t;
                    out[i] += 1;
                }
                None => break,
            }
        }
    }
    out
}

pub async fn funnel(State(st): State<AppState>, Json(r): Json<FunnelReq>) -> ApiResult<Value> {
    if r.steps.is_empty() || r.steps.len() > 10 {
        return Err(ApiError::BadQuery("1 to 10 steps".into()));
    }
    let mut per_step = Vec::new();
    for s in &r.steps {
        let q = match (r.query.trim(), s.search.trim()) {
            ("", s) => s.to_string(),
            (q, "") => q.to_string(),
            (q, s) => format!("({q}) ({s})"),
        };
        let pairs = analytics::facet_times(
            &st,
            Dataset::Logs,
            "rum_events",
            &q,
            "@session.id",
            r.from,
            r.to,
            200_000,
        )
        .await?;
        let mut m: HashMap<String, Vec<i64>> = HashMap::new();
        for (session, t) in pairs {
            m.entry(session).or_default().push(t);
        }
        per_step.push(m);
    }
    let counts = funnel_counts(&per_step);
    let first = counts.first().copied().unwrap_or(0).max(1) as f64;
    Ok(Json(json!({
        "steps": r.steps.iter().zip(&counts).enumerate().map(|(i, (s, c))| json!({
            "label": s.label.clone().unwrap_or_else(|| s.search.clone()),
            "search": s.search,
            "sessions": c,
            "conversion": *c as f64 / first * 100.0,
            "dropoff": if i == 0 { 0 } else { counts[i - 1] - c },
        })).collect::<Vec<_>>(),
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn histograms() {
        let h = histogram(vec![1.0, 2.0, 3.0, 4.0, f64::NAN], 3);
        assert_eq!(h["count"], 4);
        let counts: Vec<u64> = h["buckets"]
            .as_array()
            .unwrap()
            .iter()
            .map(|b| b["count"].as_u64().unwrap())
            .collect();
        assert_eq!(counts, vec![1, 1, 2]);
        assert_eq!(h["p50"], 3.0);
        let one = histogram(vec![5.0, 5.0], 10);
        assert_eq!(one["buckets"].as_array().unwrap().len(), 1);
        assert_eq!(histogram(vec![], 10)["count"], 0);
    }

    #[test]
    fn funnels() {
        let step = |pairs: &[(&str, i64)]| {
            let mut m: HashMap<String, Vec<i64>> = HashMap::new();
            for (s, t) in pairs {
                m.entry(s.to_string()).or_default().push(*t);
            }
            m
        };
        let steps = vec![
            step(&[("a", 1), ("b", 5), ("c", 1)]),
            // b reached the cart before the home page: not counted.
            step(&[("a", 2), ("b", 3), ("c", 4)]),
            step(&[("a", 3), ("c", 2)]),
        ];
        assert_eq!(funnel_counts(&steps), vec![3, 2, 1]);
    }
}
