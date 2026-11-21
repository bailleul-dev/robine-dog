//! Metrics generated from logs, spans and RUM events at ingestion, as
//! Datadog's "Generate Metrics": a filter query, a count or a distribution of
//! a measure, and attributes turned into tags. Like Datadog, they count what
//! is ingested after their creation.
//!
//! RUM-based metrics are on one event type; those on sessions and views
//! count each session or view once (`uniqueness`): when it first matches
//! (`match`), or when it ends (`end`, with its final values: a view that
//! says it is no longer active, or one not updated for 15 minutes).

use super::{Aggregator, Sample, normalize_tags};
use crate::intake::logs::LogRow;
use crate::intake::traces::SpanRow;
use crate::query::matcher::{Record, matches};
use crate::query::parser::{self, Expr};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};

pub const LOGS_KIND: &str = "logs_metric";
pub const SPANS_KIND: &str = "spans_metric";
pub const RUM_KIND: &str = "rum_metric";

pub const RUM_EVENT_TYPES: &[&str] = &[
    "session",
    "view",
    "action",
    "error",
    "resource",
    "long_task",
    "vital",
];

/// A session or view not updated for this long has ended.
const IDLE_END_MS: i64 = 15 * 60_000;
/// How long a counted session or view is remembered.
const SEEN_FOR_MS: i64 = 6 * 3_600_000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Compute {
    /// `count` or `distribution`.
    pub aggregation_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_percentiles: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Filter {
    #[serde(default = "everything")]
    pub query: String,
}

fn everything() -> String {
    "*".into()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GroupBy {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Uniqueness {
    /// `match` or `end`.
    pub when: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MetricDef {
    pub compute: Compute,
    #[serde(default = "default_filter")]
    pub filter: Filter,
    #[serde(default)]
    pub group_by: Vec<GroupBy>,
    /// RUM-based metrics: `session`, `view`, `action`…
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_type: Option<String>,
    /// RUM-based metrics on sessions and views.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uniqueness: Option<Uniqueness>,
}

fn default_filter() -> Filter {
    Filter {
        query: everything(),
    }
}

fn parse(q: &str) -> Result<Expr, String> {
    let q = q.trim();
    if q.is_empty() || q == "*" {
        return Ok(Expr::All);
    }
    parser::parse(q).map_err(|e| format!("filter `{q}`: {}", e.0))
}

/// The tag name of a group-by path: `@http.status_code` → `http.status_code`.
fn tag_name(g: &GroupBy) -> String {
    g.tag_name
        .clone()
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| g.path.trim_start_matches('@').to_string())
}

pub fn validate(name: &str, d: &MetricDef) -> Result<(), String> {
    if !crate::intake::series::valid_name(name) {
        return Err(format!("`{name}` is not a valid metric name"));
    }
    match d.compute.aggregation_type.as_str() {
        "count" => {}
        "distribution" => {
            if d.compute
                .path
                .as_deref()
                .is_none_or(|p| p.trim().is_empty())
            {
                return Err("a distribution needs a path (a measure, like @duration)".into());
            }
        }
        other => {
            return Err(format!(
                "aggregation_type is count or distribution, not `{other}`"
            ));
        }
    }
    parse(&d.filter.query)?;
    for g in &d.group_by {
        if g.path.trim().is_empty() {
            return Err("a group by needs a path".into());
        }
    }
    Ok(())
}

/// What a RUM-based metric needs more: its event type, and for sessions
/// and views, what identifies one and whether it counts at its end.
pub fn validate_rum(name: &str, d: &mut MetricDef) -> Result<(), String> {
    validate(name, d)?;
    let t = d.event_type.clone().unwrap_or_default();
    if !RUM_EVENT_TYPES.contains(&t.as_str()) {
        return Err(format!(
            "event_type is one of {}",
            RUM_EVENT_TYPES.join(", ")
        ));
    }
    match (&d.uniqueness, t.as_str()) {
        (None, "session" | "view") => {
            d.uniqueness = Some(Uniqueness {
                when: "match".into(),
            })
        }
        (Some(u), "session" | "view") if u.when != "match" && u.when != "end" => {
            return Err("uniqueness.when is match or end".into());
        }
        (Some(_), "session" | "view") => {}
        (Some(_), _) => d.uniqueness = None,
        (None, _) => {}
    }
    Ok(())
}

struct Compiled {
    name: String,
    filter: Expr,
    path: Option<String>,
    group_by: Vec<(String, String)>,
    event_type: Option<String>,
    /// The attribute naming the session or view, and whether it counts at
    /// its end.
    unique: Option<(&'static str, bool)>,
}

/// A session or view counted at its end: its latest values.
struct Pending {
    updated: i64,
    host: Option<String>,
    tags: Vec<String>,
    sample: Sample,
}

#[derive(Default)]
pub struct Generated {
    logs: RwLock<Arc<Vec<Compiled>>>,
    spans: RwLock<Arc<Vec<Compiled>>>,
    rum: RwLock<Arc<Vec<Compiled>>>,
    /// (metric, session or view) already counted, and when.
    seen: Mutex<HashMap<(String, String), i64>>,
    pending: Mutex<HashMap<(String, String), Pending>>,
}

fn compile(defs: &[(String, MetricDef)]) -> Vec<Compiled> {
    defs.iter()
        .filter_map(|(name, d)| match parse(&d.filter.query) {
            Ok(filter) => Some(Compiled {
                name: name.clone(),
                filter,
                path: (d.compute.aggregation_type == "distribution")
                    .then(|| d.compute.path.clone())
                    .flatten(),
                group_by: d
                    .group_by
                    .iter()
                    .map(|g| (g.path.clone(), tag_name(g)))
                    .collect(),
                event_type: d.event_type.clone(),
                unique: match (d.event_type.as_deref(), &d.uniqueness) {
                    (Some("session"), Some(u)) => Some(("@session.id", u.when == "end")),
                    (Some("view"), Some(u)) => Some(("@view.id", u.when == "end")),
                    _ => None,
                },
            }),
            Err(e) => {
                tracing::warn!(metric = %name, error = %e, "generated metric skipped");
                None
            }
        })
        .collect()
}

impl Generated {
    pub fn load_logs(&self, defs: &[(String, MetricDef)]) {
        *self.logs.write().unwrap_or_else(|e| e.into_inner()) = Arc::new(compile(defs));
    }

    pub fn load_spans(&self, defs: &[(String, MetricDef)]) {
        *self.spans.write().unwrap_or_else(|e| e.into_inner()) = Arc::new(compile(defs));
    }

    pub fn load_rum(&self, defs: &[(String, MetricDef)]) {
        *self.rum.write().unwrap_or_else(|e| e.into_inner()) = Arc::new(compile(defs));
    }

    /// A matching record's tags and sample, if it has the measure.
    fn measure<R: Record>(m: &Compiled, r: &R) -> Option<(Vec<String>, Sample)> {
        let tags = normalize_tags(
            m.group_by
                .iter()
                .map(|(path, tag)| {
                    let v = r
                        .values(path)
                        .into_iter()
                        .next()
                        .unwrap_or_else(|| "n/a".into());
                    format!("{tag}:{v}")
                })
                .collect::<Vec<_>>(),
        );
        let sample = match &m.path {
            None => Sample::Count(1.0),
            Some(path) => Sample::Distribution(
                r.values(path)
                    .iter()
                    .find_map(|v| v.trim().parse::<f64>().ok())?,
            ),
        };
        Some((tags, sample))
    }

    /// A RUM event (`source` is its type). `now_ms`: when it is received,
    /// the time of what counts at a session's or view's end.
    pub fn on_rum(&self, agg: &Aggregator, row: &LogRow, now_ms: i64) {
        let list = self.rum.read().unwrap_or_else(|e| e.into_inner()).clone();
        for m in list.iter() {
            let typ = m.event_type.as_deref().unwrap_or_default();
            if typ != "session" && row.source.as_deref() != Some(typ) {
                continue;
            }
            if !matches(&m.filter, row) {
                continue;
            }
            let Some((key_path, at_end)) = m.unique else {
                if let Some((tags, sample)) = Self::measure(m, row) {
                    agg.add(row.ts, &m.name, row.host.as_deref(), tags, sample);
                }
                continue;
            };
            let Some(entity) = row.values(key_path).into_iter().next() else {
                continue;
            };
            let key = (m.name.clone(), entity);
            let mut seen = self.seen.lock().unwrap_or_else(|e| e.into_inner());
            if seen.contains_key(&key) {
                continue;
            }
            if !at_end {
                if let Some((tags, sample)) = Self::measure(m, row) {
                    seen.insert(key, now_ms);
                    agg.add(row.ts, &m.name, row.host.as_deref(), tags, sample);
                }
                continue;
            }
            let active_path = if typ == "view" {
                "@view.is_active"
            } else {
                "@session.is_active"
            };
            let ended = row.values(active_path).iter().any(|v| v == "false");
            let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
            match Self::measure(m, row) {
                Some((tags, sample)) if ended => {
                    pending.remove(&key);
                    seen.insert(key, now_ms);
                    agg.add(now_ms, &m.name, row.host.as_deref(), tags, sample);
                }
                Some((tags, sample)) => {
                    pending.insert(
                        key,
                        Pending {
                            updated: now_ms,
                            host: row.host.clone(),
                            tags,
                            sample,
                        },
                    );
                }
                // No measure yet: still waiting for the final values.
                None => {
                    if let Some(p) = pending.get_mut(&key) {
                        p.updated = now_ms;
                    }
                }
            }
        }
    }

    /// Counts the sessions and views idle for long enough to have ended,
    /// and forgets the old counted ones.
    pub fn flush_rum(&self, agg: &Aggregator, now_ms: i64) {
        let mut seen = self.seen.lock().unwrap_or_else(|e| e.into_inner());
        let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        let done: Vec<(String, String)> = pending
            .iter()
            .filter(|(_, p)| now_ms - p.updated >= IDLE_END_MS)
            .map(|(k, _)| k.clone())
            .collect();
        for k in done {
            if let Some(p) = pending.remove(&k) {
                agg.add(now_ms, &k.0, p.host.as_deref(), p.tags, p.sample);
                seen.insert(k, now_ms);
            }
        }
        seen.retain(|_, t| now_ms - *t < SEEN_FOR_MS);
    }

    fn emit<R: Record>(list: &[Compiled], agg: &Aggregator, r: &R, ts_ms: i64, host: Option<&str>) {
        for m in list {
            if !matches(&m.filter, r) {
                continue;
            }
            let tags = normalize_tags(
                m.group_by
                    .iter()
                    .map(|(path, tag)| {
                        let v = r
                            .values(path)
                            .into_iter()
                            .next()
                            .unwrap_or_else(|| "n/a".into());
                        format!("{tag}:{v}")
                    })
                    .collect::<Vec<_>>(),
            );
            let sample = match &m.path {
                None => Sample::Count(1.0),
                Some(path) => match r
                    .values(path)
                    .iter()
                    .find_map(|v| v.trim().parse::<f64>().ok())
                {
                    Some(v) => Sample::Distribution(v),
                    None => continue,
                },
            };
            agg.add(ts_ms, &m.name, host, tags, sample);
        }
    }

    pub fn on_log(&self, agg: &Aggregator, log: &LogRow) {
        let list = self.logs.read().unwrap_or_else(|e| e.into_inner()).clone();
        if !list.is_empty() {
            Self::emit(&list, agg, log, log.ts, log.host.as_deref());
        }
    }

    pub fn on_span(&self, agg: &Aggregator, span: &SpanRow) {
        let list = self.spans.read().unwrap_or_else(|e| e.into_inner()).clone();
        if !list.is_empty() {
            let host = span.meta.get("_dd.hostname").and_then(|v| v.as_str());
            Self::emit(&list, agg, span, span.start / 1_000_000, host);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn validates() {
        let d = |v: serde_json::Value| -> MetricDef { serde_json::from_value(v).unwrap() };
        assert!(
            validate(
                "logs.errors",
                &d(json!({"compute": {"aggregation_type": "count"}}))
            )
            .is_ok()
        );
        assert!(
            validate(
                "bad name",
                &d(json!({"compute": {"aggregation_type": "count"}}))
            )
            .is_err()
        );
        assert!(
            validate(
                "x",
                &d(json!({"compute": {"aggregation_type": "distribution"}}))
            )
            .is_err()
        );
        assert!(
            validate(
                "x",
                &d(json!({"compute": {"aggregation_type": "count"}, "filter": {"query": "a:("}}))
            )
            .is_err()
        );
        assert_eq!(
            tag_name(&GroupBy {
                path: "@http.status_code".into(),
                tag_name: None
            }),
            "http.status_code"
        );
    }

    fn rum_row(source: &str, attrs: serde_json::Value) -> LogRow {
        LogRow {
            ts: 1_000,
            status: "info".into(),
            service: Some("shop-web".into()),
            host: None,
            source: Some(source.into()),
            message: String::new(),
            tags: vec![],
            attributes: attrs.as_object().cloned().unwrap_or_default(),
            trace_id: None,
            span_id: None,
        }
    }

    #[test]
    fn rum_metrics_count_sessions_and_views_once() {
        let mut views: MetricDef = serde_json::from_value(json!({
            "compute": {"aggregation_type": "count"}, "event_type": "view",
            "group_by": [{"path": "@view.url_path", "tag_name": "page"}]}))
        .unwrap();
        validate_rum("rum.views", &mut views).unwrap();
        assert_eq!(views.uniqueness.as_ref().unwrap().when, "match");
        let mut sessions: MetricDef = serde_json::from_value(json!({
            "compute": {"aggregation_type": "count"}, "event_type": "session"}))
        .unwrap();
        validate_rum("rum.sessions", &mut sessions).unwrap();
        let mut lt: MetricDef = serde_json::from_value(json!({
            "compute": {"aggregation_type": "distribution", "path": "@view.loading_time"},
            "event_type": "view", "uniqueness": {"when": "end"}}))
        .unwrap();
        validate_rum("rum.loading", &mut lt).unwrap();
        let mut bad = sessions.clone();
        bad.event_type = Some("page".into());
        assert!(validate_rum("x", &mut bad).is_err());
        let mut clicks: MetricDef = serde_json::from_value(json!({
            "compute": {"aggregation_type": "count"}, "event_type": "action",
            "uniqueness": {"when": "match"}, "filter": {"query": "@action.type:click"}}))
        .unwrap();
        validate_rum("rum.clicks", &mut clicks).unwrap();
        assert!(clicks.uniqueness.is_none());

        let g = Generated::default();
        g.load_rum(&[
            ("rum.views".into(), views),
            ("rum.sessions".into(), sessions),
            ("rum.loading".into(), lt),
            ("rum.clicks".into(), clicks),
        ]);
        let agg = Aggregator::default();
        let view = |id: &str, extra: serde_json::Value| {
            let mut v = json!({"session": {"id": "s1"}, "view": {"id": id, "url_path": "/cart"}});
            for (k, x) in extra.as_object().unwrap() {
                v["view"][k] = x.clone();
            }
            rum_row("view", v)
        };
        // A view sent three times; a second view; two clicks.
        g.on_rum(&agg, &view("v1", json!({})), 10_000);
        g.on_rum(&agg, &view("v1", json!({"loading_time": 800})), 20_000);
        g.on_rum(&agg, &view("v2", json!({"loading_time": 300})), 30_000);
        g.on_rum(
            &agg,
            &view("v1", json!({"loading_time": 900, "is_active": false})),
            40_000,
        );
        for _ in 0..2 {
            g.on_rum(
                &agg,
                &rum_row(
                    "action",
                    json!({"session": {"id": "s1"}, "action": {"type": "click"}}),
                ),
                40_000,
            );
        }
        let total = |agg: &Aggregator, name: &str| -> (f64, Vec<f64>) {
            let (points, _) = agg.drain();
            let mine: Vec<&super::super::Point> =
                points.iter().filter(|p| p.metric == name).collect();
            (
                mine.iter().map(|p| p.value * p.weight).sum(),
                mine.iter().map(|p| p.value).collect(),
            )
        };
        let (points, _) = agg.drain();
        let sum = |n: &str| {
            points
                .iter()
                .filter(|p| p.metric == n)
                .map(|p| p.value)
                .sum::<f64>()
        };
        assert_eq!(sum("rum.views"), 2.0);
        assert_eq!(sum("rum.sessions"), 1.0);
        assert_eq!(sum("rum.clicks"), 2.0);
        // v1 ended with its final value; v2 is still open.
        let lt: Vec<f64> = points
            .iter()
            .filter(|p| p.metric == "rum.loading")
            .map(|p| p.value)
            .collect();
        assert_eq!(lt, vec![900.0]);
        assert!(
            points
                .iter()
                .any(|p| p.metric == "rum.views" && p.tags == ["page:/cart"])
        );
        // v2 ends after 15 idle minutes; v1 is not counted again.
        g.flush_rum(&agg, 30_000 + IDLE_END_MS - 1);
        assert_eq!(total(&agg, "rum.loading").1, Vec::<f64>::new());
        g.flush_rum(&agg, 30_000 + IDLE_END_MS);
        assert_eq!(total(&agg, "rum.loading").1, vec![300.0]);
        g.on_rum(&agg, &view("v1", json!({"loading_time": 950})), 50_000);
        g.flush_rum(&agg, 50_000 + IDLE_END_MS);
        assert_eq!(total(&agg, "rum.loading").0, 0.0);
    }
}
