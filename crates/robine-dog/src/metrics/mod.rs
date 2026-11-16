//! Metrics: points, the in-process aggregator (what a Datadog Agent does with
//! DogStatsD packets), and the trace metrics derived from service entry spans.

pub mod dogstatsd;
pub mod query;

use crate::store::Store;
use crate::store::greptime::{lit, num, opt_lit, tags_column};
use crate::store::writer::Insertable;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Agent flush interval: samples are aggregated into 10-second buckets.
pub const FLUSH_INTERVAL_MS: i64 = 10_000;

/// One stored value of one series.
#[derive(Debug, Clone)]
pub struct Point {
    /// Epoch milliseconds.
    pub ts: i64,
    pub metric: String,
    pub host: Option<String>,
    /// Sorted, deduplicated `key:value` tags.
    pub tags: Vec<String>,
    pub value: f64,
}

impl Insertable for Point {
    const TABLE: &'static str = "metric_points";
    const COLUMNS: &'static str = r#"("ts", "metric", "host", "tags", "value")"#;

    fn values(&self, out: &mut String) {
        use std::fmt::Write;
        let _ = write!(
            out,
            "({}, {}, {}, {}, {})",
            self.ts,
            lit(&self.metric),
            opt_lit(self.host.as_deref()),
            lit(&tags_column(&self.tags)),
            num(self.value),
        );
    }
}

/// Datadog's metric types, as kept in the metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    /// A number of occurrences per interval (summed over time).
    Count,
    /// A per-second rate.
    Rate,
    /// A value at a point in time (averaged over time).
    Gauge,
    /// Every sample is kept; queried with percentiles.
    Distribution,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Count => "count",
            Kind::Rate => "rate",
            Kind::Gauge => "gauge",
            Kind::Distribution => "distribution",
        }
    }

    pub fn parse(s: &str) -> Kind {
        match s {
            "count" => Kind::Count,
            "rate" => Kind::Rate,
            "distribution" => Kind::Distribution,
            _ => Kind::Gauge,
        }
    }
}

/// Normalizes tags as the Agent does: trimmed, deduplicated, sorted.
pub fn normalize_tags(tags: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut t: Vec<String> = tags
        .into_iter()
        .map(|t| t.trim().replace(',', "_"))
        .filter(|t| !t.is_empty())
        .collect();
    t.sort();
    t.dedup();
    t
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Key {
    bucket: i64,
    name: String,
    host: Option<String>,
    tags: Vec<String>,
}

#[derive(Debug)]
enum Acc {
    Count(f64),
    Gauge(f64),
    Set(HashSet<String>),
    /// DogStatsD histograms: the Agent turns them into avg/count/median/95percentile/max.
    Histogram(Vec<f64>),
    Distribution(Vec<f64>),
}

/// A sample as received.
#[derive(Debug, Clone, PartialEq)]
pub enum Sample {
    Count(f64),
    Gauge(f64),
    Set(String),
    Histogram(f64),
    Distribution(f64),
}

#[derive(Default)]
pub struct Aggregator {
    buckets: Mutex<HashMap<Key, Acc>>,
}

impl Aggregator {
    pub fn add(&self, ts_ms: i64, name: &str, host: Option<&str>, tags: Vec<String>, s: Sample) {
        let key = Key {
            bucket: ts_ms - ts_ms.rem_euclid(FLUSH_INTERVAL_MS),
            name: name.to_string(),
            host: host.map(str::to_string),
            tags,
        };
        let mut map = self.buckets.lock().unwrap_or_else(|e| e.into_inner());
        match (map.get_mut(&key), s) {
            (Some(Acc::Count(c)), Sample::Count(v)) => *c += v,
            (Some(Acc::Gauge(g)), Sample::Gauge(v)) => *g = v,
            (Some(Acc::Set(set)), Sample::Set(v)) => {
                set.insert(v);
            }
            (Some(Acc::Histogram(h)), Sample::Histogram(v)) => h.push(v),
            (Some(Acc::Distribution(d)), Sample::Distribution(v)) => d.push(v),
            // First sample, or a type change: the latest type wins.
            (_, s) => {
                map.insert(
                    key,
                    match s {
                        Sample::Count(v) => Acc::Count(v),
                        Sample::Gauge(v) => Acc::Gauge(v),
                        Sample::Set(v) => Acc::Set(HashSet::from([v])),
                        Sample::Histogram(v) => Acc::Histogram(vec![v]),
                        Sample::Distribution(v) => Acc::Distribution(vec![v]),
                    },
                );
            }
        }
    }

    /// Takes every bucket out, as points and the (name, type) of each metric.
    fn drain(&self) -> (Vec<Point>, HashMap<String, Kind>) {
        let map = std::mem::take(&mut *self.buckets.lock().unwrap_or_else(|e| e.into_inner()));
        let mut points = Vec::new();
        let mut kinds = HashMap::new();
        for (k, acc) in map {
            let mut emit = |name: String, kind: Kind, value: f64| {
                kinds.insert(name.clone(), kind);
                points.push(Point {
                    ts: k.bucket,
                    metric: name,
                    host: k.host.clone(),
                    tags: k.tags.clone(),
                    value,
                });
            };
            match acc {
                Acc::Count(c) => emit(k.name.clone(), Kind::Count, c),
                Acc::Gauge(g) => emit(k.name.clone(), Kind::Gauge, g),
                Acc::Set(s) => emit(k.name.clone(), Kind::Gauge, s.len() as f64),
                Acc::Histogram(mut h) => {
                    h.sort_by(f64::total_cmp);
                    let n = h.len() as f64;
                    let at = |p: f64| h[(((n - 1.0) * p).round() as usize).min(h.len() - 1)];
                    emit(
                        format!("{}.avg", k.name),
                        Kind::Gauge,
                        h.iter().sum::<f64>() / n,
                    );
                    emit(
                        format!("{}.count", k.name),
                        Kind::Rate,
                        n / (FLUSH_INTERVAL_MS as f64 / 1000.0),
                    );
                    emit(format!("{}.median", k.name), Kind::Gauge, at(0.5));
                    emit(format!("{}.95percentile", k.name), Kind::Gauge, at(0.95));
                    emit(format!("{}.max", k.name), Kind::Gauge, h[h.len() - 1]);
                }
                Acc::Distribution(d) => {
                    for v in d {
                        emit(k.name.clone(), Kind::Distribution, v);
                    }
                }
            }
        }
        (points, kinds)
    }

    /// Flushes the buckets to the store every interval, and once more on shutdown.
    pub fn spawn_flusher(self: &Arc<Self>, store: Arc<Store>) {
        let agg = self.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_millis(FLUSH_INTERVAL_MS as u64));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tick.tick().await;
                agg.flush(&store).await;
            }
        });
    }

    pub async fn flush(&self, store: &Store) {
        let (points, kinds) = self.drain();
        if points.is_empty() {
            return;
        }
        let kinds: Vec<(String, String)> = kinds
            .into_iter()
            .map(|(n, k)| (n, k.as_str().to_string()))
            .collect();
        if let Err(e) = store.pg.touch_metrics(&kinds).await {
            tracing::error!(error = %format!("{e:#}"), "recording metric metadata");
        }
        if let Err(e) = store.points.send(points).await {
            tracing::error!(error = %e, "storing metric points");
        }
    }
}

/// Trace metrics of a service entry span, as Datadog derives them:
/// `trace.<operation>.hits`, `.errors` and the `trace.<operation>` latency
/// distribution (seconds), tagged by env, service, resource and version.
pub fn trace_metrics(agg: &Aggregator, s: &crate::intake::traces::SpanRow) {
    if !s.top_level {
        return;
    }
    let mut tags = vec![
        format!("service:{}", s.service),
        format!("resource_name:{}", s.resource),
    ];
    if let Some(env) = &s.env {
        tags.push(format!("env:{env}"));
    }
    if let Some(v) = &s.version {
        tags.push(format!("version:{v}"));
    }
    if let Some(code) = s.meta.get("http.status_code").and_then(|v| v.as_str()) {
        tags.push(format!("http.status_code:{code}"));
    }
    let tags = normalize_tags(tags);
    let host = s.meta.get("_dd.hostname").and_then(|v| v.as_str());
    let ts = s.start / 1_000_000;
    let base = format!("trace.{}", s.name);
    agg.add(
        ts,
        &format!("{base}.hits"),
        host,
        tags.clone(),
        Sample::Count(1.0),
    );
    agg.add(
        ts,
        &format!("{base}.errors"),
        host,
        tags.clone(),
        Sample::Count(if s.error { 1.0 } else { 0.0 }),
    );
    agg.add(
        ts,
        &base,
        host,
        tags,
        Sample::Distribution(s.duration as f64 / 1e9),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aggregates_per_bucket_and_type() {
        let agg = Aggregator::default();
        let t = vec!["env:prod".to_string()];
        agg.add(1_000, "hits", None, t.clone(), Sample::Count(1.0));
        agg.add(9_999, "hits", None, t.clone(), Sample::Count(2.0));
        agg.add(10_000, "hits", None, t.clone(), Sample::Count(5.0));
        agg.add(1_000, "temp", Some("h"), t.clone(), Sample::Gauge(1.0));
        agg.add(2_000, "temp", Some("h"), t.clone(), Sample::Gauge(7.0));
        agg.add(2_000, "users", None, t.clone(), Sample::Set("a".into()));
        agg.add(2_000, "users", None, t.clone(), Sample::Set("a".into()));
        agg.add(2_000, "users", None, t.clone(), Sample::Set("b".into()));
        for v in 1..=100 {
            agg.add(3_000, "lat", None, t.clone(), Sample::Histogram(v as f64));
        }
        let (points, kinds) = agg.drain();
        let get = |m: &str, ts: i64| {
            points
                .iter()
                .find(|p| p.metric == m && p.ts == ts)
                .map(|p| p.value)
        };
        assert_eq!(get("hits", 0), Some(3.0));
        assert_eq!(get("hits", 10_000), Some(5.0));
        assert_eq!(get("temp", 0), Some(7.0));
        assert_eq!(get("users", 0), Some(2.0));
        assert_eq!(get("lat.max", 0), Some(100.0));
        assert_eq!(get("lat.median", 0), Some(51.0));
        assert_eq!(get("lat.count", 0), Some(10.0));
        assert_eq!(kinds["hits"], Kind::Count);
        assert_eq!(kinds["lat.count"], Kind::Rate);
        assert!(agg.drain().0.is_empty());
    }
}
