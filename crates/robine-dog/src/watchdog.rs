//! Watchdog: anomalies found without anyone configuring a monitor. Every
//! five minutes the last quarter of an hour of each service is compared to
//! its own previous day:
//!
//! - error logs: far more than usual;
//! - APM errors: far more failed requests than usual;
//! - APM latency: a 95th percentile several times its usual value.
//!
//! An anomaly opens a story (and posts an event, `source:watchdog`); the
//! story stays ongoing while the anomaly lasts and is resolved after a
//! quiet quarter of an hour.

use crate::AppState;
use crate::api::ApiResult;
use crate::intake::logs::LogRow;
use crate::store::greptime::{f64_at, i64_at, str_at};
use axum::Json;
use axum::extract::State;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, HashMap};
use std::time::Duration;

const KIND: &str = "watchdog_story";
const BUCKET_MS: i64 = 900_000;
/// Buckets of history: a day.
const HISTORY: i64 = 96;
const EVERY: Duration = Duration::from_secs(300);
const KEEP_MS: i64 = 30 * 86_400_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Story {
    pub id: String,
    /// `error_logs`, `apm_errors` or `apm_latency`.
    pub kind: String,
    pub service: String,
    pub title: String,
    /// Epoch milliseconds.
    pub start: i64,
    pub last_seen: i64,
    /// When it stopped; none while ongoing.
    pub end: Option<i64>,
    /// The worst value seen, and what is usual.
    pub value: f64,
    pub baseline: f64,
    pub unit: String,
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Whether the latest value stands out of the history: well above its mean
/// (by deviations and by ratio) and above `floor`, so quiet services with a
/// handful of events do not alert.
pub fn anomaly(history: &[f64], current: f64, floor: f64, ratio: f64) -> Option<f64> {
    if history.len() < 8 {
        return None;
    }
    let n = history.len() as f64;
    let mean = history.iter().sum::<f64>() / n;
    let var = history.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / n;
    let limit = (mean + 4.0 * var.sqrt()).max(mean * ratio).max(floor);
    (current > limit).then_some(mean)
}

/// Per service, the value of each bucket from the oldest to the current one
/// (missing buckets are `empty`).
fn series(
    rows: &[(String, i64, f64)],
    first: i64,
    empty: Option<f64>,
) -> HashMap<String, Vec<Option<f64>>> {
    let mut out: HashMap<String, Vec<Option<f64>>> = HashMap::new();
    for (service, bucket, v) in rows {
        let i = (bucket - first) / BUCKET_MS;
        if (0..=HISTORY).contains(&i) {
            out.entry(service.clone())
                .or_insert_with(|| vec![empty; HISTORY as usize + 1])[i as usize] = Some(*v);
        }
    }
    out
}

struct Finding {
    kind: &'static str,
    service: String,
    title: String,
    value: f64,
    baseline: f64,
    unit: &'static str,
}

async fn findings(st: &AppState, now: i64) -> anyhow::Result<Vec<Finding>> {
    // The current bucket ends now; the history is the day before it.
    let first = now - (HISTORY + 1) * BUCKET_MS;
    let bucket = |ts: &str, unit: i64| {
        format!(
            "{first} + CAST(floor((CAST({ts} AS BIGINT) / {unit} - {first}) / {BUCKET_MS}) AS BIGINT) * {BUCKET_MS}"
        )
    };
    let gt = &st.store.gt;
    let logs_sql = format!(
        "SELECT service, {} AS b, CAST(count(*) AS DOUBLE) FROM logs \
         WHERE ts > {first} AND ts <= {now} AND status IN ('error', 'critical', 'alert', 'emergency') \
           AND service IS NOT NULL AND source <> 'synthetics' GROUP BY service, b",
        bucket("ts", 1)
    );
    let spans_sql = format!(
        "SELECT service, {} AS b, CAST(count(*) AS DOUBLE), CAST(sum(CASE WHEN error THEN 1 ELSE 0 END) AS DOUBLE), \
           approx_percentile_cont(duration, 0.95) FROM spans \
         WHERE ts > {} AND ts <= {} AND top_level = true GROUP BY service, b",
        bucket("ts", 1_000_000),
        first.saturating_mul(1_000_000),
        now.saturating_mul(1_000_000)
    );
    let (logs, spans) = tokio::try_join!(gt.query(&logs_sql), gt.query(&spans_sql))?;
    let row = |r: &crate::store::greptime::Row, col: usize| {
        Some((str_at(r, 0)?, i64_at(r, 1), f64_at(r, col)?))
    };
    let mut out = Vec::new();
    let split = |s: &[Option<f64>]| -> (Vec<f64>, Option<f64>) {
        let (cur, hist) = s.split_last().expect("not empty");
        (hist.iter().flatten().copied().collect(), *cur)
    };

    let log_rows: Vec<_> = logs.iter().filter_map(|r| row(r, 2)).collect();
    for (service, s) in series(&log_rows, first, Some(0.0)) {
        let (hist, cur) = split(&s);
        if let (Some(cur), Some(mean)) = (cur, cur.and_then(|c| anomaly(&hist, c, 20.0, 3.0))) {
            out.push(Finding {
                kind: "error_logs",
                title: format!("Error logs of {service} are well above usual"),
                service,
                value: cur,
                baseline: mean,
                unit: "error logs per 15 min",
            });
        }
    }
    let err_rows: Vec<_> = spans.iter().filter_map(|r| row(r, 3)).collect();
    for (service, s) in series(&err_rows, first, Some(0.0)) {
        let (hist, cur) = split(&s);
        if let (Some(cur), Some(mean)) = (cur, cur.and_then(|c| anomaly(&hist, c, 10.0, 3.0))) {
            out.push(Finding {
                kind: "apm_errors",
                title: format!("Requests of {service} fail far more than usual"),
                service,
                value: cur,
                baseline: mean,
                unit: "failed requests per 15 min",
            });
        }
    }
    // Latency only means something with enough requests in the bucket.
    let hits: HashMap<(String, i64), f64> = spans
        .iter()
        .filter_map(|r| row(r, 2))
        .map(|(s, b, n)| ((s, b), n))
        .collect();
    let lat_rows: Vec<_> = spans
        .iter()
        .filter_map(|r| row(r, 4))
        .filter(|(s, b, _)| hits.get(&(s.clone(), *b)).is_some_and(|n| *n >= 20.0))
        .collect();
    for (service, s) in series(&lat_rows, first, None) {
        let (hist, cur) = split(&s);
        if let (Some(cur), Some(mean)) = (cur, cur.and_then(|c| anomaly(&hist, c, 5e7, 3.0))) {
            out.push(Finding {
                kind: "apm_latency",
                title: format!("{service} is much slower than usual"),
                service,
                value: cur / 1e6,
                baseline: mean / 1e6,
                unit: "ms at p95",
            });
        }
    }
    Ok(out)
}

async fn stored(st: &AppState) -> anyhow::Result<Vec<Story>> {
    Ok(st
        .store
        .pg
        .docs(KIND)
        .await?
        .into_iter()
        .filter_map(|d| serde_json::from_value(d.body).ok())
        .collect())
}

async fn save(st: &AppState, s: &Story) -> anyhow::Result<()> {
    st.store
        .pg
        .put_doc(KIND, &s.id, &serde_json::to_value(s)?)
        .await?;
    Ok(())
}

async fn evaluate(st: &AppState) -> anyhow::Result<()> {
    let now = now_ms();
    let found = findings(st, now).await?;
    let mut ongoing: BTreeMap<(String, String), Story> = BTreeMap::new();
    for s in stored(st).await? {
        if s.end.is_none() {
            ongoing.insert((s.kind.clone(), s.service.clone()), s);
        } else if now - s.start > KEEP_MS {
            st.store.pg.delete_doc(KIND, &s.id).await?;
        }
    }
    for f in found {
        let key = (f.kind.to_string(), f.service.clone());
        if let Some(mut s) = ongoing.remove(&key) {
            s.last_seen = now;
            s.value = s.value.max(f.value);
            save(st, &s).await?;
            continue;
        }
        let story = Story {
            id: crate::store::pg::new_id(),
            kind: f.kind.into(),
            service: f.service.clone(),
            title: f.title.clone(),
            start: now - BUCKET_MS,
            last_seen: now,
            end: None,
            value: f.value,
            baseline: f.baseline,
            unit: f.unit.into(),
        };
        save(st, &story).await?;
        tracing::info!(kind = f.kind, service = %f.service, value = f.value, baseline = f.baseline, "watchdog story");
        let text = format!(
            "{:.0} {} against {:.1} usually.",
            f.value, f.unit, f.baseline
        );
        crate::events::emit(
            st,
            vec![LogRow {
                ts: now,
                status: "warn".into(),
                service: Some(f.service.clone()),
                host: None,
                source: Some("watchdog".into()),
                message: format!("{}\n{text}", f.title),
                tags: vec![format!("watchdog_kind:{}", f.kind)],
                attributes: Map::from_iter([(
                    "evt".to_string(),
                    json!({"title": f.title, "text": text}),
                )]),
                trace_id: None,
                span_id: None,
            }],
        )
        .await;
    }
    // What was not found again: resolved once quiet for a bucket.
    for (_, mut s) in ongoing {
        if now - s.last_seen >= BUCKET_MS {
            s.end = Some(s.last_seen);
            save(st, &s).await?;
        }
    }
    Ok(())
}

pub fn spawn(st: AppState) {
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(60)).await;
        let mut tick = tokio::time::interval(EVERY);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tick.tick().await;
            if let Err(e) = evaluate(&st).await {
                tracing::warn!(error = %format!("{e:#}"), "watchdog evaluation failed");
            }
        }
    });
}

pub async fn list(State(st): State<AppState>) -> ApiResult<Value> {
    let mut all = stored(&st).await?;
    all.sort_by_key(|s| (s.end.is_some(), std::cmp::Reverse(s.start)));
    Ok(Json(json!({"stories": all})))
}

/// Runs the detection now (the UI's refresh).
pub async fn run(State(st): State<AppState>) -> ApiResult<Value> {
    evaluate(&st).await?;
    list(State(st)).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spots_what_stands_out() {
        let quiet = vec![0.0; 96];
        assert_eq!(anomaly(&quiet, 5.0, 20.0, 3.0), None);
        assert_eq!(anomaly(&quiet, 50.0, 20.0, 3.0), Some(0.0));
        let busy: Vec<f64> = (0..96).map(|i| 100.0 + (i % 5) as f64).collect();
        assert_eq!(anomaly(&busy, 150.0, 20.0, 3.0), None);
        assert!(anomaly(&busy, 400.0, 20.0, 3.0).is_some());
        assert_eq!(anomaly(&[1.0, 2.0], 500.0, 20.0, 3.0), None);

        let rows = vec![("a".to_string(), 1000 + 2 * BUCKET_MS, 7.0)];
        let s = series(&rows, 1000, Some(0.0));
        assert_eq!(s["a"][2], Some(7.0));
        assert_eq!(s["a"].len(), 97);
    }
}
