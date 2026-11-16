//! Metrics generated from logs and spans at ingestion, as Datadog's
//! "Generate Metrics": a filter query, a count or a distribution of a
//! measure, and attributes turned into tags. Like Datadog, they count what is
//! ingested after their creation.

use super::{Aggregator, Sample, normalize_tags};
use crate::intake::logs::LogRow;
use crate::intake::traces::SpanRow;
use crate::query::matcher::{Record, matches};
use crate::query::parser::{self, Expr};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, RwLock};

pub const LOGS_KIND: &str = "logs_metric";
pub const SPANS_KIND: &str = "spans_metric";

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
pub struct MetricDef {
    pub compute: Compute,
    #[serde(default = "default_filter")]
    pub filter: Filter,
    #[serde(default)]
    pub group_by: Vec<GroupBy>,
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

struct Compiled {
    name: String,
    filter: Expr,
    path: Option<String>,
    group_by: Vec<(String, String)>,
}

#[derive(Default)]
pub struct Generated {
    logs: RwLock<Arc<Vec<Compiled>>>,
    spans: RwLock<Arc<Vec<Compiled>>>,
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
}
