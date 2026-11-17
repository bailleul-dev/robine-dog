//! Monitors, as Datadog's: a definition in the API's JSON (`/api/v1/monitor`),
//! evaluated every minute per group (multi-alert), with OK / Warn / Alert /
//! No Data states, recovery thresholds, renotification, mute, an event
//! history, and notifications to `@webhook-<name>` handles.
//!
//! Queries:
//! - metric:  `avg(last_5m):avg:system.cpu.user{env:prod} by {host} > 80`
//! - logs:    `logs("status:error").index("*").rollup("count").by("service").last("5m") > 10`
//! - spans:   `trace-analytics("service:web").rollup("avg", "@duration").by("resource_name").last("10m") > 500000000`

pub mod template;
pub mod webhooks;

use crate::AppState;
use crate::api::analytics::{self, AggregateReq, Compute, Filter, GroupBy, parse_span_ms};
use crate::metrics::query::{self as mq, MetricQuery};
use crate::metrics::{FLUSH_INTERVAL_MS, Kind};
use crate::query::sql::Dataset;
use crate::store::greptime::{f64_at, i64_at, lit, num, str_at};
use crate::store::pg::GroupState;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, HashMap};
use std::sync::LazyLock;

pub const KIND: &str = "monitor";
pub const EVERY_MS: i64 = 60_000;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Thresholds {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub critical: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warning: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub critical_recovery: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warning_recovery: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Options {
    pub thresholds: Thresholds,
    pub notify_no_data: bool,
    /// Minutes without data before No Data.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub no_data_timeframe: Option<i64>,
    /// Minutes between reminders while not OK (0 or none: never).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub renotify_interval: Option<i64>,
    /// Seconds the evaluation window is shifted back by.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evaluation_delay: Option<i64>,
    pub include_tags: bool,
    /// Muted scopes (`*` for all) → end (epoch seconds) or null (until unmuted).
    pub silenced: Map<String, Value>,
    /// Options this implementation doesn't use, kept as given.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            thresholds: Thresholds::default(),
            notify_no_data: false,
            no_data_timeframe: None,
            renotify_interval: None,
            evaluation_delay: None,
            include_tags: true,
            silenced: Map::new(),
            extra: Map::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MonitorDef {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<i64>,
    #[serde(default)]
    pub name: String,
    #[serde(rename = "type", default)]
    pub typ: String,
    pub query: String,
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub priority: Option<i64>,
    #[serde(default)]
    pub options: Options,
}

// --- queries -------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Cmp {
    Gt,
    Ge,
    Lt,
    Le,
    Eq,
    Ne,
}

impl Cmp {
    fn parse(s: &str) -> Option<Cmp> {
        Some(match s {
            ">" => Cmp::Gt,
            ">=" => Cmp::Ge,
            "<" => Cmp::Lt,
            "<=" => Cmp::Le,
            "==" => Cmp::Eq,
            "!=" => Cmp::Ne,
            _ => return None,
        })
    }

    pub fn symbol(self) -> &'static str {
        match self {
            Cmp::Gt => ">",
            Cmp::Ge => ">=",
            Cmp::Lt => "<",
            Cmp::Le => "<=",
            Cmp::Eq => "==",
            Cmp::Ne => "!=",
        }
    }

    pub fn holds(self, v: f64, t: f64) -> bool {
        match self {
            Cmp::Gt => v > t,
            Cmp::Ge => v >= t,
            Cmp::Lt => v < t,
            Cmp::Le => v <= t,
            Cmp::Eq => v == t,
            Cmp::Ne => v != t,
        }
    }

    /// Still beyond a recovery threshold (not yet recovered).
    fn beyond(self, v: f64, t: f64) -> bool {
        match self {
            Cmp::Gt | Cmp::Ge => v > t,
            Cmp::Lt | Cmp::Le => v < t,
            _ => self.holds(v, t),
        }
    }
}

#[derive(Debug, Clone)]
pub enum Source {
    Metric {
        time_agg: String,
        query: MetricQuery,
    },
    Events {
        dataset: Dataset,
        search: String,
        aggregation: String,
        metric: Option<String>,
        by: Vec<String>,
    },
}

#[derive(Debug, Clone)]
pub struct Parsed {
    pub source: Source,
    pub window_ms: i64,
    pub cmp: Cmp,
    pub threshold: f64,
}

static TAIL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?s)(.*?)\s*(>=|<=|==|!=|>|<)\s*(-?[0-9]+(?:\.[0-9]+)?(?:[eE][-+]?[0-9]+)?)\s*$")
        .unwrap()
});
static METRIC_HEAD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(avg|min|max|sum)\(last_(\d+[smhdw])\):(.+)$").unwrap());
static EVENTS_HEAD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"^(logs|trace-analytics|spans)\("((?:[^"\\]|\\.)*)"\)((?:\.\w+\((?:\s*"(?:[^"\\]|\\.)*"\s*,?)*\))*)$"#)
        .unwrap()
});
static CALL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"\.(\w+)\(((?:\s*"(?:[^"\\]|\\.)*"\s*,?)*)\)"#).unwrap());
static ARG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#""((?:[^"\\]|\\.)*)""#).unwrap());

fn unescape(s: &str) -> String {
    s.replace("\\\"", "\"").replace("\\\\", "\\")
}

pub fn parse(query: &str) -> Result<Parsed, String> {
    let caps = TAIL
        .captures(query.trim())
        .ok_or("the query must end with a comparison, e.g. `> 80`")?;
    let head = caps[1].trim();
    let cmp = Cmp::parse(&caps[2]).ok_or("bad comparator")?;
    let threshold: f64 = caps[3].parse().map_err(|_| "bad threshold")?;
    if let Some(m) = METRIC_HEAD.captures(head) {
        let window_ms = parse_span_ms(&m[2]).ok_or("bad evaluation window")?;
        let query = mq::parse(m[3].trim())?;
        return Ok(Parsed {
            source: Source::Metric {
                time_agg: m[1].to_string(),
                query,
            },
            window_ms,
            cmp,
            threshold,
        });
    }
    if let Some(m) = EVENTS_HEAD.captures(head) {
        let dataset = if &m[1] == "logs" {
            Dataset::Logs
        } else {
            Dataset::Spans
        };
        let search = unescape(&m[2]);
        crate::query::parser::parse(&search).map_err(|e| format!("search: {}", e.0))?;
        let (mut aggregation, mut metric, mut by, mut window) =
            ("count".to_string(), None, Vec::new(), None);
        for c in CALL.captures_iter(&m[3]) {
            let args: Vec<String> = ARG.captures_iter(&c[2]).map(|a| unescape(&a[1])).collect();
            match &c[1] {
                "index" => {}
                "rollup" => {
                    aggregation = args.first().cloned().unwrap_or_else(|| "count".into());
                    metric = args.get(1).cloned().filter(|s| !s.is_empty());
                }
                "by" => {
                    by = args
                        .iter()
                        .flat_map(|a| a.split(','))
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect()
                }
                "last" => window = args.first().and_then(|a| parse_span_ms(a)),
                other => return Err(format!("unknown method .{other}()")),
            }
        }
        if aggregation != "count" && metric.is_none() {
            return Err(format!("rollup(\"{aggregation}\") needs a measure"));
        }
        return Ok(Parsed {
            source: Source::Events {
                dataset,
                search,
                aggregation,
                metric,
                by,
            },
            window_ms: window.ok_or("missing .last(\"5m\")")?,
            cmp,
            threshold,
        });
    }
    Err("unsupported monitor query: use `avg(last_5m):<metric query> > N`, `logs(\"…\")…` or `trace-analytics(\"…\")…`".into())
}

/// Datadog's monitor type for a query.
pub fn type_of(p: &Parsed) -> &'static str {
    match &p.source {
        Source::Metric { .. } => "metric alert",
        Source::Events {
            dataset: Dataset::Logs,
            ..
        } => "log alert",
        Source::Events { .. } => "trace-analytics alert",
    }
}

// --- evaluation ------------------------------------------------------------------

/// One group's value: its name (`host:a,env:prod`, `*` without groups),
/// its tags, and the value (none: no data).
#[derive(Debug, Clone)]
pub struct Sample {
    pub group: String,
    pub tags: BTreeMap<String, String>,
    pub value: Option<f64>,
}

fn group_name(tags: &BTreeMap<String, String>, order: &[String]) -> String {
    if order.is_empty() {
        return "*".into();
    }
    order
        .iter()
        .map(|k| format!("{k}:{}", tags.get(k).map(String::as_str).unwrap_or("")))
        .collect::<Vec<_>>()
        .join(",")
}

pub fn group_tags(group: &str) -> BTreeMap<String, String> {
    if group == "*" {
        return BTreeMap::new();
    }
    group
        .split(',')
        .filter_map(|t| t.split_once(':'))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

pub async fn evaluate(st: &AppState, p: &Parsed, to: i64) -> anyhow::Result<Vec<Sample>> {
    let from = to - p.window_ms;
    match &p.source {
        Source::Metric { time_agg, query } => {
            let kind = st
                .store
                .pg
                .metric(&query.metric)
                .await?
                .map(|m| Kind::parse(&m.typ))
                .unwrap_or(Kind::Gauge);
            let interval = (p.window_ms / 60).max(FLUSH_INTERVAL_MS);
            let rows = st
                .store
                .gt
                .query(&mq::build_sql(query, kind, from, to, interval))
                .await?;
            let n = query.by.len();
            let mut groups: BTreeMap<Vec<String>, Vec<f64>> = BTreeMap::new();
            for r in &rows {
                let key: Vec<String> = (0..n)
                    .map(|g| str_at(r, 1 + g).unwrap_or_else(|| "N/A".into()))
                    .collect();
                if let Some(v) = f64_at(r, 1 + n) {
                    groups.entry(key).or_default().push(v);
                }
            }
            Ok(groups
                .into_iter()
                .map(|(key, vs)| {
                    let value = match time_agg.as_str() {
                        "min" => vs.iter().copied().fold(f64::INFINITY, f64::min),
                        "max" => vs.iter().copied().fold(f64::NEG_INFINITY, f64::max),
                        "sum" => vs.iter().sum(),
                        _ => vs.iter().sum::<f64>() / vs.len() as f64,
                    };
                    let tags: BTreeMap<String, String> =
                        query.by.iter().cloned().zip(key).collect();
                    Sample {
                        group: group_name(&tags, &query.by),
                        tags,
                        value: Some(value),
                    }
                })
                .collect())
        }
        Source::Events {
            dataset,
            search,
            aggregation,
            metric,
            by,
        } => {
            let req = AggregateReq {
                compute: vec![Compute {
                    aggregation: aggregation.clone(),
                    metric: metric.clone(),
                    typ: None,
                    interval: None,
                }],
                filter: Filter {
                    query: search.clone(),
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
            let out = analytics::run(st, *dataset, &req, to)
                .await
                .map_err(|e| anyhow::anyhow!("{e:?}"))?;
            let mut samples: Vec<Sample> = out
                .buckets
                .iter()
                .map(|b| {
                    let tags: BTreeMap<String, String> = by
                        .iter()
                        .map(|f| {
                            let v = b["by"][f].as_str().unwrap_or("").to_string();
                            (f.clone(), v)
                        })
                        .collect();
                    Sample {
                        group: group_name(&tags, by),
                        tags,
                        value: b["computes"]["c0"].as_f64(),
                    }
                })
                .collect();
            // Counting nothing is zero, not missing data.
            if by.is_empty() && samples.is_empty() && aggregation == "count" {
                samples.push(Sample {
                    group: "*".into(),
                    tags: BTreeMap::new(),
                    value: Some(0.0),
                });
            }
            Ok(samples)
        }
    }
}

// --- states ------------------------------------------------------------------------

pub const OK: &str = "OK";
pub const WARN: &str = "Warn";
pub const ALERT: &str = "Alert";
pub const NO_DATA: &str = "No Data";

/// The worst state first: what the monitor as a whole shows.
pub fn overall(statuses: impl IntoIterator<Item = String>) -> String {
    let rank = |s: &str| match s {
        ALERT => 0,
        WARN => 1,
        NO_DATA => 2,
        OK => 3,
        _ => 4,
    };
    statuses
        .into_iter()
        .min_by_key(|s| rank(s))
        .unwrap_or_else(|| OK.to_string())
}

/// The next state of a group from its value.
pub fn next_status(p: &Parsed, th: &Thresholds, prev: &str, value: f64) -> &'static str {
    if p.cmp.holds(value, p.threshold) {
        return ALERT;
    }
    if prev == ALERT
        && let Some(r) = th.critical_recovery
        && p.cmp.beyond(value, r)
    {
        return ALERT;
    }
    if let Some(w) = th.warning
        && p.cmp.holds(value, w)
    {
        return WARN;
    }
    if prev == WARN
        && let Some(r) = th.warning_recovery
        && p.cmp.beyond(value, r)
    {
        return WARN;
    }
    OK
}

pub fn is_muted(def: &MonitorDef, group: &str, now: i64) -> bool {
    let tags = group_tags(group);
    def.options.silenced.iter().any(|(scope, end)| {
        let active = match end.as_i64() {
            Some(end) => end * 1000 > now,
            None => true,
        };
        let applies = scope == "*"
            || scope.split(',').all(|t| match t.split_once(':') {
                Some((k, v)) => tags.get(k).is_some_and(|x| x == v),
                None => false,
            });
        active && applies
    })
}

fn transition(from: &str, to: &str, renotify: bool) -> &'static str {
    match (from, to) {
        (_, ALERT) if renotify => "Re-Triggered",
        (_, WARN) if renotify => "Re-Warn",
        (_, NO_DATA) if renotify => "Re-No Data",
        (_, ALERT) => "Triggered",
        (_, WARN) => "Warn",
        (_, NO_DATA) => "No Data",
        (ALERT, OK) | (WARN, OK) | (NO_DATA, OK) => "Recovered",
        _ => "OK",
    }
}

fn fmt_value(v: Option<f64>) -> String {
    match v {
        Some(v) if v.fract() == 0.0 && v.abs() < 1e15 => format!("{}", v as i64),
        Some(v) => format!("{v:.4}")
            .trim_end_matches('0')
            .trim_end_matches('.')
            .to_string(),
        None => "N/A".into(),
    }
}

/// Evaluates one monitor now: states, history, value series, notifications.
pub async fn run_monitor(st: &AppState, def: &MonitorDef, now: i64) -> anyhow::Result<()> {
    let downtimes = crate::api::downtimes::stored(st).await.unwrap_or_default();
    let id = def.id.unwrap_or_default();
    let p = parse(&def.query).map_err(|e| anyhow::anyhow!(e))?;
    let to = now - def.options.evaluation_delay.unwrap_or(0) * 1000;
    let mut samples = evaluate(st, &p, to).await?;
    let pg = &st.store.pg;
    let known = pg.monitor_groups(id).await?;
    let counting =
        matches!(&p.source, Source::Events { aggregation, .. } if aggregation == "count");
    for g in &known {
        if !samples.iter().any(|s| s.group == g.group) {
            samples.push(Sample {
                group: g.group.clone(),
                tags: group_tags(&g.group),
                // A group counting nothing now counts zero.
                value: counting.then_some(0.0),
            });
        }
    }
    let previous: HashMap<&str, &GroupState> =
        known.iter().map(|g| (g.group.as_str(), g)).collect();
    let mut values = Vec::new();
    for s in &samples {
        let prev = previous.get(s.group.as_str()).copied();
        let prev_status = prev.map(|g| g.status.as_str()).unwrap_or(OK);
        let last_data = if s.value.is_some() {
            Some(now)
        } else {
            prev.and_then(|g| g.last_data)
        };
        let status = match s.value {
            Some(v) => next_status(&p, &def.options.thresholds, prev_status, v),
            None => {
                let timeframe = def
                    .options
                    .no_data_timeframe
                    .unwrap_or(p.window_ms / 60_000 * 2)
                    .max(1)
                    * 60_000;
                let silent_since = last_data.or(prev.map(|g| g.since)).unwrap_or(now);
                if def.options.notify_no_data && now - silent_since >= timeframe {
                    NO_DATA
                } else if prev.is_none() {
                    OK
                } else {
                    // Without data, a group keeps its state.
                    match prev_status {
                        ALERT => ALERT,
                        WARN => WARN,
                        NO_DATA => NO_DATA,
                        _ => OK,
                    }
                }
            }
        };
        let changed = status != prev_status;
        let renotify_due = !changed
            && status != OK
            && def.options.renotify_interval.unwrap_or(0) > 0
            && prev
                .and_then(|g| g.last_notified)
                .is_some_and(|t| now - t >= def.options.renotify_interval.unwrap_or(0) * 60_000);
        let mut state = GroupState {
            group: s.group.clone(),
            status: status.to_string(),
            value: s.value.or(prev.and_then(|g| g.value)),
            last_data,
            last_triggered: prev.and_then(|g| g.last_triggered),
            last_notified: prev.and_then(|g| g.last_notified),
            last_resolved: prev.and_then(|g| g.last_resolved),
            since: if changed {
                now
            } else {
                prev.map(|g| g.since).unwrap_or(now)
            },
        };
        // A new group starting OK is not news.
        let news = (changed && !(prev.is_none() && status == OK)) || renotify_due;
        if news {
            if status == ALERT && changed {
                state.last_triggered = Some(now);
            }
            if status == OK {
                state.last_resolved = Some(now);
            }
            let event = transition(prev_status, status, renotify_due);
            let ctx = template::Context {
                def,
                parsed: &p,
                group: &s.group,
                tags: &s.tags,
                status,
                previous: prev_status,
                transition: event,
                value: s.value,
                renotify: renotify_due,
                last_triggered: state.last_triggered,
            };
            let title = ctx.title();
            let message = template::render(&def.message, &ctx);
            let silenced = is_muted(def, &s.group, now)
                || downtimes.iter().any(|d| d.applies(def, &s.group, now));
            let notified = if silenced {
                vec![]
            } else {
                webhooks::notify(st, &ctx, &title, &message).await
            };
            if !notified.is_empty() || status != OK {
                state.last_notified = Some(now);
            }
            pg.add_monitor_event(
                id,
                &s.group,
                prev_status,
                status,
                s.value,
                &title,
                &message,
                &notified,
            )
            .await?;
            tracing::info!(monitor = id, group = %s.group, from = prev_status, to = status, value = %fmt_value(s.value), "monitor state");
        }
        pg.put_monitor_group(id, &state).await?;
        values.push(format!(
            "({now}, {id}, {}, {}, {})",
            lit(&s.group),
            s.value.map(num).unwrap_or_else(|| "NULL".into()),
            lit(status)
        ));
    }
    // Groups gone quiet and OK for a day are dropped (they come back if seen).
    let keep: Vec<String> = samples
        .iter()
        .map(|s| s.group.clone())
        .chain(
            known
                .iter()
                .filter(|g| g.status != OK || now - g.last_data.unwrap_or(g.since) < 86_400_000)
                .map(|g| g.group.clone()),
        )
        .collect();
    pg.delete_monitor_groups(id, &keep).await?;
    if !values.is_empty() {
        st.store
            .gt
            .exec(&format!(
                "INSERT INTO monitor_values (ts, monitor_id, grp, value, status) VALUES {}",
                values.join(", ")
            ))
            .await?;
    }
    Ok(())
}

/// The value history of a monitor, per group: `(group, [(ms, value)])`.
pub type History = Vec<(String, Vec<(i64, Option<f64>, String)>)>;

pub async fn history(st: &AppState, id: i64, from: i64, to: i64) -> anyhow::Result<History> {
    let rows = st
        .store
        .gt
        .query(&format!(
            "SELECT grp, CAST(ts AS BIGINT) AS t, value, status FROM monitor_values
             WHERE monitor_id = {id} AND ts >= {from} AND ts <= {to} ORDER BY grp, ts"
        ))
        .await?;
    let mut out: History = Vec::new();
    for r in &rows {
        let g = str_at(r, 0).unwrap_or_default();
        let point = (i64_at(r, 1), f64_at(r, 2), str_at(r, 3).unwrap_or_default());
        match out.last_mut() {
            Some((last, pts)) if *last == g => pts.push(point),
            _ => out.push((g, vec![point])),
        }
    }
    Ok(out)
}

pub async fn stored(st: &AppState) -> anyhow::Result<Vec<MonitorDef>> {
    Ok(st
        .store
        .pg
        .docs(KIND)
        .await?
        .into_iter()
        .filter_map(|d| {
            let mut m: MonitorDef = serde_json::from_value(d.body).ok()?;
            m.id = d.id.parse().ok();
            Some(m)
        })
        .collect())
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Evaluates every monitor once a minute.
pub fn spawn(st: AppState) {
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(10)).await;
        let mut tick = tokio::time::interval(std::time::Duration::from_millis(EVERY_MS as u64));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tick.tick().await;
            let monitors = match stored(&st).await {
                Ok(m) => m,
                Err(e) => {
                    tracing::warn!(error = %e, "loading monitors");
                    continue;
                }
            };
            let now = now_ms();
            futures::future::join_all(monitors.iter().map(|m| {
                let st = &st;
                async move {
                    if let Err(e) = run_monitor(st, m, now).await {
                        tracing::warn!(monitor = ?m.id, error = %e, "monitor evaluation failed");
                    }
                }
            }))
            .await;
        }
    });
}

/// Evaluates one monitor right away (after it is created or changed).
pub fn evaluate_soon(st: &AppState, def: MonitorDef) {
    let st = st.clone();
    tokio::spawn(async move {
        if let Err(e) = run_monitor(&st, &def, now_ms()).await {
            tracing::warn!(monitor = ?def.id, error = %e, "monitor evaluation failed");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metric_queries() {
        let p = parse("avg(last_5m):avg:system.cpu.user{env:prod} by {host} > 80").unwrap();
        assert_eq!(p.window_ms, 300_000);
        assert_eq!(p.cmp, Cmp::Gt);
        assert_eq!(p.threshold, 80.0);
        let Source::Metric { time_agg, query } = &p.source else {
            panic!()
        };
        assert_eq!(time_agg, "avg");
        assert_eq!(query.metric, "system.cpu.user");
        assert_eq!(query.by, vec!["host"]);
        assert_eq!(type_of(&p), "metric alert");
        assert!(parse("max(last_1h):sum:shop.orders{*}.as_count() <= 0.5").is_ok());
        assert!(parse("avg(last_5m):avg:x{*}").is_err());
    }

    #[test]
    fn event_queries() {
        let p = parse(r#"logs("status:error service:\"web store\"").index("*").rollup("count").by("service,host").last("15m") >= 10"#).unwrap();
        let Source::Events {
            dataset,
            search,
            aggregation,
            metric,
            by,
        } = &p.source
        else {
            panic!()
        };
        assert_eq!(*dataset, Dataset::Logs);
        assert_eq!(search, r#"status:error service:"web store""#);
        assert_eq!(aggregation, "count");
        assert!(metric.is_none());
        assert_eq!(by, &vec!["service".to_string(), "host".to_string()]);
        assert_eq!(p.window_ms, 900_000);
        assert_eq!(p.cmp, Cmp::Ge);

        let p = parse(r#"trace-analytics("env:prod").rollup("avg", "@duration").by("resource_name").last("10m") > 500000000"#).unwrap();
        assert_eq!(type_of(&p), "trace-analytics alert");
        assert!(parse(r#"logs("*").rollup("avg").last("5m") > 1"#).is_err());
        assert!(parse(r#"logs("*").rollup("count") > 1"#).is_err());
    }

    #[test]
    fn states_and_recovery() {
        let p = parse("avg(last_5m):avg:cpu{*} > 80").unwrap();
        let th = Thresholds {
            critical: Some(80.0),
            warning: Some(60.0),
            critical_recovery: Some(70.0),
            warning_recovery: None,
        };
        assert_eq!(next_status(&p, &th, OK, 90.0), ALERT);
        assert_eq!(next_status(&p, &th, OK, 75.0), WARN);
        assert_eq!(next_status(&p, &th, ALERT, 75.0), ALERT);
        assert_eq!(next_status(&p, &th, ALERT, 70.0), WARN);
        assert_eq!(next_status(&p, &th, WARN, 10.0), OK);
        assert_eq!(
            overall([OK.to_string(), WARN.to_string(), NO_DATA.to_string()]),
            WARN
        );
    }

    #[test]
    fn muting() {
        let mut def: MonitorDef = serde_json::from_value(json!({"name": "m", "type": "metric alert", "query": "avg(last_5m):avg:cpu{*} by {host} > 1"})).unwrap();
        assert!(!is_muted(&def, "host:a", 0));
        def.options.silenced.insert("host:a".into(), Value::Null);
        assert!(is_muted(&def, "host:a", 0));
        assert!(!is_muted(&def, "host:b", 0));
        def.options.silenced = Map::from_iter([("*".to_string(), json!(10))]);
        assert!(is_muted(&def, "host:b", 9_000));
        assert!(!is_muted(&def, "host:b", 10_000));
    }
}
