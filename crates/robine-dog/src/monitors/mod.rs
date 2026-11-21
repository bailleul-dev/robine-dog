//! Monitors, as Datadog's: a definition in the API's JSON (`/api/v1/monitor`),
//! evaluated every minute per group (multi-alert), with OK / Warn / Alert /
//! No Data states, recovery thresholds, renotification, mute, an event
//! history, and notifications to `@webhook-<name>` handles.
//!
//! Queries:
//! - metric:  `avg(last_5m):avg:system.cpu.user{env:prod} by {host} > 80`
//! - logs:    `logs("status:error").index("*").rollup("count").by("service").last("5m") > 10`
//! - spans:   `trace-analytics("service:web").rollup("avg", "@duration").by("resource_name").last("10m") > 500000000`

pub mod email;
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
    /// Service checks: consecutive OK runs to recover.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ok: Option<f64>,
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
    /// Seconds a new group is left alone (a host booting, a deploy).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new_group_delay: Option<i64>,
    /// Added to reminders (`renotify_interval`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub escalation_message: Option<String>,
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
            new_group_delay: None,
            escalation_message: None,
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

/// Logs-like event sets an events query can read: their table, and the
/// monitor type Datadog gives it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EventSet {
    Logs,
    Spans,
    Events,
    Rum,
}

impl EventSet {
    fn parse(f: &str) -> Option<Self> {
        Some(match f {
            "logs" => Self::Logs,
            "trace-analytics" | "spans" => Self::Spans,
            "events" => Self::Events,
            "rum" => Self::Rum,
            _ => return None,
        })
    }

    pub fn dataset(self) -> Dataset {
        match self {
            Self::Spans => Dataset::Spans,
            _ => Dataset::Logs,
        }
    }

    pub fn table(self) -> &'static str {
        match self {
            Self::Logs => "logs",
            Self::Spans => "spans",
            Self::Events => "events",
            Self::Rum => "rum_events",
        }
    }

    pub fn monitor_type(self) -> &'static str {
        match self {
            Self::Logs => "log alert",
            Self::Spans => "trace-analytics alert",
            Self::Events => "event-v2 alert",
            Self::Rum => "rum alert",
        }
    }
}

/// A composite monitor's expression over other monitors (`12 && !(34 || 56)`).
#[derive(Debug, Clone, PartialEq)]
pub enum Composite {
    Id(i64),
    Not(Box<Composite>),
    And(Box<Composite>, Box<Composite>),
    Or(Box<Composite>, Box<Composite>),
}

impl Composite {
    pub fn parse(s: &str) -> Result<Self, String> {
        let mut toks = Vec::new();
        let c: Vec<char> = s.chars().collect();
        let mut i = 0;
        while i < c.len() {
            match c[i] {
                w if w.is_whitespace() => i += 1,
                '(' | ')' | '!' => {
                    toks.push(c[i].to_string());
                    i += 1;
                }
                '&' | '|' if c.get(i + 1) == Some(&c[i]) => {
                    toks.push(format!("{}{}", c[i], c[i]));
                    i += 2;
                }
                d if d.is_ascii_digit() => {
                    let st = i;
                    while i < c.len() && c[i].is_ascii_digit() {
                        i += 1;
                    }
                    toks.push(c[st..i].iter().collect());
                }
                other => return Err(format!("unexpected `{other}` in a composite query")),
            }
        }
        let mut pos = 0;
        let e = Self::or(&toks, &mut pos)?;
        if pos != toks.len() {
            return Err(format!("unexpected `{}` in a composite query", toks[pos]));
        }
        Ok(e)
    }

    fn or(t: &[String], i: &mut usize) -> Result<Self, String> {
        let mut l = Self::and(t, i)?;
        while t.get(*i).map(String::as_str) == Some("||") {
            *i += 1;
            l = Self::Or(Box::new(l), Box::new(Self::and(t, i)?));
        }
        Ok(l)
    }

    fn and(t: &[String], i: &mut usize) -> Result<Self, String> {
        let mut l = Self::not(t, i)?;
        while t.get(*i).map(String::as_str) == Some("&&") {
            *i += 1;
            l = Self::And(Box::new(l), Box::new(Self::not(t, i)?));
        }
        Ok(l)
    }

    fn not(t: &[String], i: &mut usize) -> Result<Self, String> {
        let tok = t.get(*i).ok_or("a composite query ends too early")?.clone();
        *i += 1;
        match tok.as_str() {
            "!" => Ok(Self::Not(Box::new(Self::not(t, i)?))),
            "(" => {
                let e = Self::or(t, i)?;
                if t.get(*i).map(String::as_str) != Some(")") {
                    return Err("missing `)` in a composite query".into());
                }
                *i += 1;
                Ok(e)
            }
            n => n
                .parse()
                .map(Self::Id)
                .map_err(|_| format!("expected a monitor id, not `{n}`")),
        }
    }

    pub fn ids(&self, out: &mut Vec<i64>) {
        match self {
            Self::Id(i) => out.push(*i),
            Self::Not(x) => x.ids(out),
            Self::And(a, b) | Self::Or(a, b) => {
                a.ids(out);
                b.ids(out);
            }
        }
    }

    /// Three-valued: true (with a severity, 2 alert / 1 warn), false, or
    /// unknown (a monitor without data).
    pub fn eval(&self, status: &dyn Fn(i64) -> Option<String>) -> Option<(bool, u8)> {
        match self {
            Self::Id(i) => match status(*i).as_deref() {
                Some(ALERT) => Some((true, 2)),
                Some(WARN) => Some((true, 1)),
                Some(OK) => Some((false, 0)),
                _ => None,
            },
            Self::Not(x) => x.eval(status).map(|(b, _)| (!b, if b { 0 } else { 2 })),
            Self::And(a, b) => match (a.eval(status), b.eval(status)) {
                (Some((false, _)), _) | (_, Some((false, _))) => Some((false, 0)),
                (Some((true, x)), Some((true, y))) => Some((true, x.min(y))),
                _ => None,
            },
            Self::Or(a, b) => match (a.eval(status), b.eval(status)) {
                (Some((true, x)), Some((true, y))) => Some((true, x.max(y))),
                (Some((true, x)), _) | (_, Some((true, x))) => Some((true, x)),
                (Some((false, _)), Some((false, _))) => Some((false, 0)),
                _ => None,
            },
        }
    }
}

#[derive(Debug, Clone)]
pub enum Source {
    Metric {
        time_agg: String,
        query: MetricQuery,
    },
    /// `change(avg(last_5m),last_1h):<query>` / `pct_change(…)`: the
    /// aggregate now against the same aggregate `shift_ms` earlier.
    Change {
        time_agg: String,
        query: MetricQuery,
        shift_ms: i64,
        pct: bool,
    },
    /// Functions around the query (`anomalies(…)`, `outliers(…)`,
    /// `forecast(…)`, `ewma_10(…)`…), run as a formula over `q`.
    Function {
        time_agg: String,
        query: MetricQuery,
        /// The bare metric query.
        base: String,
        /// The functions, reading the query as `q`.
        formula: String,
        /// The outermost function.
        func: String,
    },
    Events {
        set: EventSet,
        dataset: Dataset,
        search: String,
        aggregation: String,
        metric: Option<String>,
        by: Vec<String>,
    },
    /// `error-tracking-traces("…")`, `error-tracking-logs("…")`,
    /// `error-tracking("…")`: errors grouped into issues.
    ErrorTracking {
        sources: Vec<&'static str>,
        search: String,
        by: Vec<String>,
    },
    Composite(Composite),
    /// `"datadog.agent.up".over("env:prod").by("host").last(3).count_by_status()`.
    ServiceCheck {
        check: String,
        filter: String,
        by: Vec<String>,
        last: usize,
    },
    /// `burn_rate("id").over("30d").long_window("1h").short_window("5m")`,
    /// `error_budget("id").over("30d")`.
    Slo {
        slo_id: String,
        timeframe: String,
        alert: SloAlert,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum SloAlert {
    /// How many times faster than allowed the budget burns, over both
    /// windows (the lower of the two).
    BurnRate { long_ms: i64, short_ms: i64 },
    /// The share of the time frame's budget spent, in percent.
    ErrorBudget,
}

impl Source {
    /// The tags groups are made of (multi alert when not empty).
    pub fn group_keys(&self) -> Vec<String> {
        match self {
            Source::Metric { query, .. }
            | Source::Change { query, .. }
            | Source::Function { query, .. } => query.by.clone(),
            Source::Events { by, .. }
            | Source::ErrorTracking { by, .. }
            | Source::ServiceCheck { by, .. } => by.clone(),
            Source::Composite(_) | Source::Slo { .. } => Vec::new(),
        }
    }

    /// The metric a monitor reads, if any.
    pub fn metric(&self) -> Option<&str> {
        match self {
            Source::Metric { query, .. }
            | Source::Change { query, .. }
            | Source::Function { query, .. } => Some(&query.metric),
            _ => None,
        }
    }
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
    LazyLock::new(|| Regex::new(r"^(avg|min|max|sum)\((last|next)_(\d+[smhdw])\):(.+)$").unwrap());
static SLO_HEAD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"^(burn_rate|error_budget)\(\s*"([^"]+)"\s*\)((?:\.\w+\(\s*"[^"]*"\s*\))*)$"#)
        .unwrap()
});
static SLO_CALL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"\.(\w+)\(\s*"([^"]*)"\s*\)"#).unwrap());

static CHANGE_HEAD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^(change|pct_change)\(\s*(avg|min|max|sum)\(last_(\d+[smhdw])\)\s*,\s*last_(\d+[smhdw])\s*\):(.+)$",
    )
    .unwrap()
});
static EVENTS_HEAD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"^(logs|trace-analytics|spans|events|rum|error-tracking|error-tracking-traces|error-tracking-logs|error-tracking-rum)\("((?:[^"\\]|\\.)*)"\)((?:\.\w+\((?:\s*"(?:[^"\\]|\\.)*"\s*,?)*\))*)$"#)
        .unwrap()
});
static CALL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"\.(\w+)\(((?:\s*"(?:[^"\\]|\\.)*"\s*,?)*)\)"#).unwrap());
static ARG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#""((?:[^"\\]|\\.)*)""#).unwrap());
static CHECK_HEAD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"^"([^"]+)"\s*((?:\.\w+\([^()]*\))*)\s*$"#).unwrap());
static CHECK_CALL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"\.(\w+)\(([^()]*)\)"#).unwrap());

fn unescape(s: &str) -> String {
    s.replace("\\\"", "\"").replace("\\\\", "\\")
}

/// The functions a metric monitor accepts around its query.
const FUNCTIONS: &[&str] = &[
    "anomalies",
    "outliers",
    "forecast",
    "abs",
    "log2",
    "log10",
    "cumsum",
    "integral",
    "diff",
    "derivative",
    "monotonic_diff",
    "per_second",
    "per_minute",
    "per_hour",
    "moving_rollup",
    "fill",
    "trend_line",
    "robust_trend",
    "piecewise_constant",
    "count_nonzero",
    "count_not_null",
    "exclude_null",
    "clamp_min",
    "clamp_max",
    "cutoff_min",
    "cutoff_max",
    "default_zero",
    "timeshift",
    "hour_before",
    "day_before",
    "week_before",
    "month_before",
    "top",
    "bottom",
];

fn known_function(f: &str) -> bool {
    FUNCTIONS.contains(&f)
        || f.strip_prefix("ewma_")
            .is_some_and(|n| n.parse::<u32>().is_ok())
        || f.strip_prefix("median_")
            .is_some_and(|n| n.parse::<u32>().is_ok())
}

/// `"env:prod","host:a"` → `env:prod,host:a`.
fn quoted_list(args: &str) -> Vec<String> {
    ARG.captures_iter(args)
        .map(|a| unescape(&a[1]))
        .flat_map(|a| {
            a.split(',')
                .map(|s| s.trim().to_string())
                .collect::<Vec<_>>()
        })
        .filter(|s| !s.is_empty())
        .collect()
}

fn parse_service_check(q: &str) -> Option<Result<Parsed, String>> {
    let m = CHECK_HEAD.captures(q.trim())?;
    let check = m[1].to_string();
    let (mut filter, mut by, mut last) = (Vec::new(), Vec::new(), 1usize);
    for c in CHECK_CALL.captures_iter(&m[2]) {
        match &c[1] {
            "over" => filter = quoted_list(&c[2]),
            "by" => by = quoted_list(&c[2]),
            "last" => last = c[2].trim().parse().unwrap_or(1),
            "count_by_status" | "exclude" => {}
            other => return Some(Err(format!("unknown method .{other}()"))),
        }
    }
    filter.retain(|f| f != "*");
    Some(Ok(Parsed {
        source: Source::ServiceCheck {
            check,
            filter: filter.join(","),
            by,
            last: last.clamp(1, 100),
        },
        window_ms: EVERY_MS,
        cmp: Cmp::Ge,
        threshold: 1.0,
    }))
}

pub fn parse(query: &str) -> Result<Parsed, String> {
    let trimmed = query.trim();
    // Composite: monitor ids and boolean operators only.
    if !trimmed.is_empty()
        && trimmed.chars().any(|c| c.is_ascii_digit())
        && trimmed
            .chars()
            .all(|c| c.is_ascii_digit() || c.is_whitespace() || "()&|!".contains(c))
    {
        return Ok(Parsed {
            source: Source::Composite(Composite::parse(trimmed)?),
            window_ms: EVERY_MS,
            cmp: Cmp::Ge,
            threshold: 1.0,
        });
    }
    if let Some(p) = parse_service_check(trimmed) {
        return p;
    }
    let caps = TAIL
        .captures(trimmed)
        .ok_or("the query must end with a comparison, e.g. `> 80`")?;
    let head = caps[1].trim();
    let cmp = Cmp::parse(&caps[2]).ok_or("bad comparator")?;
    let threshold: f64 = caps[3].parse().map_err(|_| "bad threshold")?;
    if let Some(m) = SLO_HEAD.captures(head) {
        let span = crate::api::analytics::parse_span_ms;
        let (mut timeframe, mut long, mut short) = ("30d".to_string(), None, None);
        for c in SLO_CALL.captures_iter(&m[3]) {
            match &c[1] {
                "over" => timeframe = c[2].to_string(),
                "long_window" => long = Some(span(&c[2]).ok_or("bad long window")?),
                "short_window" => short = Some(span(&c[2]).ok_or("bad short window")?),
                other => return Err(format!("unknown method .{other}()")),
            }
        }
        if crate::slo::timeframe_ms(&timeframe).is_none() {
            return Err("the time frame is 7d, 30d or 90d".into());
        }
        let (alert, window_ms) = if &m[1] == "burn_rate" {
            let long_ms = long.unwrap_or(3_600_000);
            let short_ms = short.unwrap_or(long_ms / 12).max(60_000);
            if !(60_000..=48 * 3_600_000).contains(&long_ms) || short_ms >= long_ms {
                return Err("the long window is 1m to 48h, the short one shorter".into());
            }
            (SloAlert::BurnRate { long_ms, short_ms }, long_ms)
        } else {
            if long.is_some() || short.is_some() {
                return Err("error budget alerts have no windows".into());
            }
            let w = crate::slo::timeframe_ms(&timeframe).unwrap_or(0);
            (SloAlert::ErrorBudget, w)
        };
        return Ok(Parsed {
            source: Source::Slo {
                slo_id: m[2].to_string(),
                timeframe,
                alert,
            },
            window_ms,
            cmp,
            threshold,
        });
    }
    if let Some(m) = CHANGE_HEAD.captures(head) {
        let window_ms = parse_span_ms(&m[3]).ok_or("bad evaluation window")?;
        let shift_ms = parse_span_ms(&m[4]).ok_or("bad comparison window")?;
        return Ok(Parsed {
            source: Source::Change {
                time_agg: m[2].to_string(),
                query: mq::parse(m[5].trim())?,
                shift_ms,
                pct: &m[1] == "pct_change",
            },
            window_ms,
            cmp,
            threshold,
        });
    }
    if let Some(m) = METRIC_HEAD.captures(head) {
        let window_ms = parse_span_ms(&m[3]).ok_or("bad evaluation window")?;
        let rest = m[4].trim();
        if let Some((base, formula)) = mq::split_functions(rest) {
            let func = formula[..formula.find('(').unwrap_or(0)].to_string();
            if !known_function(&func) {
                return Err(format!("unknown function {func}()"));
            }
            let parsed_formula = crate::api::query::parse_formula(&formula)?;
            if &m[2] == "next" && func != "forecast" {
                return Err("`next_…` windows are for forecast() only".into());
            }
            if func == "forecast" && &m[2] != "next" {
                return Err("a forecast monitor looks ahead: `max(next_1w):forecast(…)`".into());
            }
            let _ = parsed_formula;
            return Ok(Parsed {
                source: Source::Function {
                    time_agg: m[1].to_string(),
                    query: mq::parse(&base)?,
                    base,
                    formula,
                    func,
                },
                window_ms,
                cmp,
                threshold,
            });
        }
        if &m[2] == "next" {
            return Err("`next_…` windows are for forecast() only".into());
        }
        let query = mq::parse(rest)?;
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
        let window_ms = window.ok_or("missing .last(\"5m\")")?;
        let source = if let Some(kind) = m[1].strip_prefix("error-tracking") {
            if aggregation != "count" {
                return Err("Error Tracking monitors count errors: .rollup(\"count\")".into());
            }
            let sources = match kind {
                "-traces" => vec!["spans"],
                "-logs" => vec!["logs"],
                "-rum" => vec!["logs"],
                _ => vec!["spans", "logs"],
            };
            let by = by
                .into_iter()
                .map(|b| b.trim_start_matches('@').to_string())
                .collect::<Vec<_>>();
            if let Some(b) = by
                .iter()
                .find(|b| !matches!(b.as_str(), "issue.id" | "service" | "error.type"))
            {
                return Err(format!(
                    "Error Tracking monitors group by @issue.id, service or error.type, not {b}"
                ));
            }
            Source::ErrorTracking {
                sources,
                search,
                by,
            }
        } else {
            let set = EventSet::parse(&m[1]).ok_or("unknown events source")?;
            Source::Events {
                set,
                dataset: set.dataset(),
                search,
                aggregation,
                metric,
                by,
            }
        };
        return Ok(Parsed {
            source,
            window_ms,
            cmp,
            threshold,
        });
    }
    Err("unsupported monitor query: use `avg(last_5m):<metric query> > N`, `change(…)`, `anomalies(…)`, `logs(\"…\")…`, `trace-analytics(\"…\")…`, `rum(\"…\")…`, `events(\"…\")…`, `error-tracking(\"…\")…`, a service check `\"check\".over(\"*\")…` or a composite `12 && 34`".into())
}

/// Datadog's monitor type for a query.
pub fn type_of(p: &Parsed) -> &'static str {
    match &p.source {
        Source::Metric { .. } => "metric alert",
        Source::Change { .. } | Source::Function { .. } => "query alert",
        Source::Events { set, .. } => set.monitor_type(),
        Source::ErrorTracking { .. } => "error-tracking alert",
        Source::Composite(_) => "composite",
        Source::ServiceCheck { .. } => "service check",
        Source::Slo { .. } => "slo alert",
    }
}

// --- evaluation ------------------------------------------------------------------

/// One group's value: its name (`host:a,env:prod`, `*` without groups),
/// its tags, and the value (none: no data). Some sources decide the state
/// themselves (`status`): composites and service checks.
#[derive(Debug, Clone)]
pub struct Sample {
    pub group: String,
    pub tags: BTreeMap<String, String>,
    pub value: Option<f64>,
    pub status: Option<&'static str>,
}

impl Sample {
    fn new(group: String, tags: BTreeMap<String, String>, value: Option<f64>) -> Self {
        Self {
            group,
            tags,
            value,
            status: None,
        }
    }
}

/// A state a source can't decide yet: the group keeps its state.
pub const KEEP: &str = "keep";

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

fn aggregate(how: &str, vs: &[f64]) -> Option<f64> {
    if vs.is_empty() {
        return None;
    }
    Some(match how {
        "min" => vs.iter().copied().fold(f64::INFINITY, f64::min),
        "max" => vs.iter().copied().fold(f64::NEG_INFINITY, f64::max),
        "sum" => vs.iter().sum(),
        _ => vs.iter().sum::<f64>() / vs.len() as f64,
    })
}

/// A metric query aggregated over `[from, to]`, per group.
async fn metric_samples(
    st: &AppState,
    time_agg: &str,
    query: &MetricQuery,
    from: i64,
    to: i64,
    full_window: bool,
) -> anyhow::Result<Vec<Sample>> {
    let kind = st
        .store
        .pg
        .metric(&query.metric)
        .await?
        .map(|m| Kind::parse(&m.typ))
        .unwrap_or(Kind::Gauge);
    let interval = ((to - from) / 60).max(FLUSH_INTERVAL_MS);
    let rows = st
        .store
        .gt
        .query(&mq::build_sql(query, kind, from, to, interval))
        .await?;
    let n = query.by.len();
    let mut groups: BTreeMap<Vec<String>, Vec<(i64, f64)>> = BTreeMap::new();
    for r in &rows {
        let key: Vec<String> = (0..n)
            .map(|g| str_at(r, 1 + g).unwrap_or_else(|| "N/A".into()))
            .collect();
        if let Some(v) = f64_at(r, 1 + n) {
            groups.entry(key).or_default().push((i64_at(r, 0), v));
        }
    }
    Ok(groups
        .into_iter()
        // `require_full_window`: only groups with data from the start.
        .filter(|(_, pts)| !full_window || pts.first().is_some_and(|p| p.0 <= from + interval))
        .map(|(key, pts)| {
            let vs: Vec<f64> = pts.iter().map(|p| p.1).collect();
            let tags: BTreeMap<String, String> = query.by.iter().cloned().zip(key).collect();
            Sample::new(group_name(&tags, &query.by), tags, aggregate(time_agg, &vs))
        })
        .collect())
}

fn named_option(e: &crate::api::query::Expr, key: &str) -> Option<crate::api::query::Expr> {
    use crate::api::query::Expr;
    let Expr::Call(_, args) = e else { return None };
    args.iter().find_map(|a| match a {
        Expr::Named(n, v) if n == key => Some(v.as_ref().clone()),
        _ => None,
    })
}

fn option_text(e: &crate::api::query::Expr, key: &str) -> Option<String> {
    use crate::api::query::Expr;
    match named_option(e, key)? {
        Expr::Str(s) | Expr::Ref(s) => Some(s),
        _ => None,
    }
}

fn option_num(e: &crate::api::query::Expr, key: &str) -> Option<f64> {
    use crate::api::query::Expr;
    match named_option(e, key)? {
        Expr::Num(n) => Some(n),
        Expr::Str(s) => s.parse().ok(),
        _ => None,
    }
}

/// `last_15m` / `15m` → milliseconds.
fn option_span(s: &str) -> Option<i64> {
    parse_span_ms(s.strip_prefix("last_").unwrap_or(s))
}

/// Function monitors: the formula over the bare query, through the same
/// engine as graphs, then reduced to one value per group.
async fn function_samples(st: &AppState, p: &Parsed, to: i64) -> anyhow::Result<Vec<Sample>> {
    let Source::Function {
        time_agg,
        query,
        base,
        formula,
        func,
    } = &p.source
    else {
        unreachable!()
    };
    use crate::api::query::{FormulaDef, QueryDef, Request, execute, parse_formula};
    let expr = parse_formula(formula).map_err(|e| anyhow::anyhow!(e))?;
    let interval = option_num(&expr, "interval").map(|s| (s * 1000.0) as i64);
    let forecast = func == "forecast";
    // A forecast learns from `history` (four times its horizon by default):
    // the graph engine extends it by a quarter, i.e. the horizon.
    let from = if forecast {
        let history = option_text(&expr, "history")
            .and_then(|h| option_span(&h))
            .unwrap_or(p.window_ms * 4)
            .max(p.window_ms * 4);
        to - history
    } else {
        to - p.window_ms
    };
    let req = Request {
        from,
        to,
        interval,
        queries: vec![QueryDef {
            data_source: "metrics".into(),
            name: "q".into(),
            query: base.clone(),
            aggregator: None,
            search: None,
            compute: None,
            group_by: Vec::new(),
        }],
        formulas: vec![FormulaDef {
            formula: formula.clone(),
            alias: None,
            limit: None,
        }],
    };
    let out = execute(st, &req, false).await.map_err(|e| match e {
        crate::api::ApiError::BadQuery(m) => anyhow::anyhow!(m),
        crate::api::ApiError::NotFound => anyhow::anyhow!("not found"),
        crate::api::ApiError::Internal(e) => e,
    })?;
    let alert_from = to
        - option_text(&expr, "alert_window")
            .and_then(|w| option_span(&w))
            .unwrap_or(15 * 60_000)
            .min(p.window_ms);
    let direction = option_text(&expr, "direction").unwrap_or_else(|| "both".into());
    let rising = matches!(p.cmp, Cmp::Gt | Cmp::Ge);
    Ok(out
        .series
        .into_iter()
        .map(|(_, _, s)| {
            let in_range = |lo: i64, hi: i64| -> Vec<usize> {
                out.times
                    .iter()
                    .enumerate()
                    .filter(|(_, t)| **t >= lo && **t <= hi)
                    .map(|(i, _)| i)
                    .collect()
            };
            let value = match func.as_str() {
                "anomalies" => s.band.as_ref().and_then(|b| {
                    let idx = in_range(alert_from, to);
                    let pick = |v: &[Option<f64>]| idx.iter().map(|i| v[*i]).collect::<Vec<_>>();
                    let band = crate::metrics::functions::Band {
                        lower: pick(&b.lower),
                        upper: pick(&b.upper),
                    };
                    crate::metrics::functions::anomalous_share(&pick(&s.values), &band, &direction)
                }),
                "outliers" => s.outlier.map(|o| if o { 1.0 } else { 0.0 }),
                "forecast" => s.band.as_ref().and_then(|b| {
                    // Any part of the forecast range crossing the threshold.
                    let idx = in_range(to + 1, to + p.window_ms);
                    let side = if rising { &b.upper } else { &b.lower };
                    let vs: Vec<f64> = idx.iter().filter_map(|i| side[*i]).collect();
                    aggregate(if rising { "max" } else { "min" }, &vs)
                }),
                _ => {
                    let vs: Vec<f64> = in_range(to - p.window_ms, to)
                        .into_iter()
                        .filter_map(|i| s.values[i])
                        .collect();
                    aggregate(time_agg, &vs)
                }
            };
            Sample::new(group_name(&s.tags, &query.by), s.tags, value)
        })
        .collect())
}

pub async fn evaluate(
    st: &AppState,
    p: &Parsed,
    to: i64,
    options: &Options,
) -> anyhow::Result<Vec<Sample>> {
    let from = to - p.window_ms;
    let full_window = options
        .extra
        .get("require_full_window")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    match &p.source {
        Source::Metric { time_agg, query } => {
            metric_samples(st, time_agg, query, from, to, full_window).await
        }
        Source::Change {
            time_agg,
            query,
            shift_ms,
            pct,
        } => {
            let (now, then) = tokio::try_join!(
                metric_samples(st, time_agg, query, from, to, false),
                metric_samples(st, time_agg, query, from - shift_ms, to - shift_ms, false),
            )?;
            let before: HashMap<String, Option<f64>> =
                then.into_iter().map(|s| (s.group, s.value)).collect();
            Ok(now
                .into_iter()
                .map(|s| {
                    let old = before.get(&s.group).copied().flatten();
                    let value = match (s.value, old) {
                        (Some(v), Some(o)) if *pct => (o != 0.0).then(|| (v - o) / o.abs() * 100.0),
                        (Some(v), Some(o)) => Some(v - o),
                        _ => None,
                    };
                    Sample::new(s.group, s.tags, value)
                })
                .collect())
        }
        Source::Function { .. } => function_samples(st, p, to).await,
        Source::Events {
            set,
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
            let out = analytics::run_in(st, *dataset, set.table(), &req, to)
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
                    Sample::new(group_name(&tags, by), tags, b["computes"]["c0"].as_f64())
                })
                .collect();
            // Counting nothing is zero, not missing data.
            if by.is_empty() && samples.is_empty() && aggregation == "count" {
                samples.push(Sample::new("*".into(), BTreeMap::new(), Some(0.0)));
            }
            Ok(samples)
        }
        Source::ErrorTracking {
            sources,
            search,
            by,
        } => {
            let issues = crate::api::errors::issue_counts(st, sources, search, from, to)
                .await
                .map_err(|e| anyhow::anyhow!("{e:?}"))?;
            let mut groups: BTreeMap<String, (BTreeMap<String, String>, f64)> = BTreeMap::new();
            for i in issues {
                let all = BTreeMap::from([
                    ("issue.id".to_string(), i.id),
                    ("service".to_string(), i.service),
                    ("error.type".to_string(), i.typ),
                ]);
                let tags: BTreeMap<String, String> = by
                    .iter()
                    .filter_map(|k| Some((k.clone(), all.get(k)?.clone())))
                    .collect();
                let g = groups
                    .entry(group_name(&tags, by))
                    .or_insert_with(|| (tags, 0.0));
                g.1 += i.count as f64;
            }
            if by.is_empty() && groups.is_empty() {
                groups.insert("*".into(), (BTreeMap::new(), 0.0));
            }
            Ok(groups
                .into_iter()
                .map(|(g, (tags, v))| Sample::new(g, tags, Some(v)))
                .collect())
        }
        Source::Slo {
            slo_id,
            timeframe,
            alert,
        } => {
            let Some(slo) = crate::slo::get(st, slo_id).await? else {
                anyhow::bail!("no SLO {slo_id}");
            };
            let target = slo
                .threshold(Some(timeframe))
                .map(|t| t.target)
                .ok_or_else(|| anyhow::anyhow!("the SLO has no target"))?;
            let api = |e: crate::api::ApiError| match e {
                crate::api::ApiError::BadQuery(m) => anyhow::anyhow!(m),
                crate::api::ApiError::NotFound => anyhow::anyhow!("not found"),
                crate::api::ApiError::Internal(e) => e,
            };
            let value = match alert {
                SloAlert::ErrorBudget => crate::slo::status(st, &slo, timeframe, to)
                    .await
                    .map_err(api)?
                    .error_budget_remaining
                    .map(|left| 100.0 - left),
                SloAlert::BurnRate { long_ms, short_ms } => {
                    let b = crate::slo::compute(st, &slo, to - long_ms, to, 60_000)
                        .await
                        .map_err(api)?;
                    let long = b.sums().burn_rate(target);
                    let short = b.sums_since(to - short_ms).burn_rate(target);
                    long.zip(short).map(|(l, s)| l.min(s))
                }
            };
            Ok(vec![Sample::new("*".into(), BTreeMap::new(), value)])
        }
        Source::ServiceCheck {
            check,
            filter,
            by,
            last,
        } => {
            let th = &options.thresholds;
            let count = |v: Option<f64>| v.map_or(1, |v| v.max(1.0) as usize);
            let (ok, warning, critical) = (count(th.ok), count(th.warning), count(th.critical));
            let n = ok.max(warning).max(critical).max(*last);
            // Runs come every 15 seconds or so: a day back is plenty.
            let runs = crate::checks::latest(st, check, filter, by, n, to - 86_400_000, to).await?;
            Ok(runs
                .into_iter()
                .map(|(tags, recent)| {
                    let status = match crate::checks::judge(&recent, ok, warning, critical) {
                        Some(crate::checks::CRITICAL) => ALERT,
                        Some(crate::checks::WARNING) => WARN,
                        Some(_) => OK,
                        None => KEEP,
                    };
                    Sample {
                        group: group_name(&tags, by),
                        tags,
                        value: recent.last().map(|s| *s as f64),
                        status: Some(status),
                    }
                })
                .collect())
        }
        Source::Composite(expr) => {
            let mut ids = Vec::new();
            expr.ids(&mut ids);
            ids.sort();
            ids.dedup();
            let mut states: HashMap<i64, HashMap<String, String>> = HashMap::new();
            for id in &ids {
                let groups = st.store.pg.monitor_groups(*id).await?;
                states.insert(
                    *id,
                    groups.into_iter().map(|g| (g.group, g.status)).collect(),
                );
            }
            // Multi-alert monitors are matched group by group; a simple
            // monitor applies to every group.
            let mut names: Vec<String> = states
                .values()
                .flat_map(|g| g.keys().filter(|k| *k != "*").cloned())
                .collect();
            names.sort();
            names.dedup();
            if names.is_empty() {
                names.push("*".into());
            }
            Ok(names
                .into_iter()
                .map(|g| {
                    let status_of = |id: i64| {
                        let s = states.get(&id)?;
                        s.get(&g).or_else(|| s.get("*")).cloned()
                    };
                    let (status, value) = match expr.eval(&status_of) {
                        Some((true, 2)) => (ALERT, Some(1.0)),
                        Some((true, _)) => (WARN, Some(1.0)),
                        Some((false, _)) => (OK, Some(0.0)),
                        None => (KEEP, None),
                    };
                    Sample {
                        tags: group_tags(&g),
                        group: g,
                        value,
                        status: Some(status),
                    }
                })
                .collect())
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
    let mut samples = evaluate(st, &p, to, &def.options).await?;
    let pg = &st.store.pg;
    let known = pg.monitor_groups(id).await?;
    let counting = matches!(&p.source, Source::Events { aggregation, .. } if aggregation == "count")
        || matches!(&p.source, Source::ErrorTracking { .. });
    for g in &known {
        if !samples.iter().any(|s| s.group == g.group) {
            samples.push(Sample {
                group: g.group.clone(),
                tags: group_tags(&g.group),
                // A group counting nothing now counts zero.
                value: counting.then_some(0.0),
                // Self-judging sources keep a group they no longer report.
                status: matches!(p.source, Source::ServiceCheck { .. } | Source::Composite(_))
                    .then_some(KEEP),
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
        let kept = || match prev_status {
            ALERT => ALERT,
            WARN => WARN,
            NO_DATA => NO_DATA,
            _ => OK,
        };
        let first_seen = prev.map(|g| g.first_seen).filter(|t| *t > 0).unwrap_or(now);
        let status = match (s.status, s.value) {
            (Some(KEEP), _) => kept(),
            (Some(decided), _) => decided,
            (None, Some(v)) => next_status(&p, &def.options.thresholds, prev_status, v),
            (None, None) => {
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
                    kept()
                }
            }
        };
        // A new group is left alone for `new_group_delay` seconds.
        let status = match def.options.new_group_delay {
            Some(d) if d > 0 && now - first_seen < d * 1000 => kept(),
            _ => status,
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
            first_seen,
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
            let mut message = template::render(&def.message, &ctx);
            if renotify_due
                && let Some(esc) = def.options.escalation_message.as_deref()
                && !esc.trim().is_empty()
            {
                message = format!("{message}\n\n{}", template::render(esc, &ctx));
            }
            let silenced = is_muted(def, &s.group, now)
                || downtimes.iter().any(|d| d.applies(def, &s.group, now));
            let notified = if silenced {
                vec![]
            } else {
                let mut sent = webhooks::notify(st, &ctx, &title, &message).await;
                sent.extend(email::notify(st, &title, &message).await);
                sent
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
            let mut tags = def.tags.clone();
            tags.extend(s.tags.iter().map(|(k, v)| format!("{k}:{v}")));
            tags.push(format!("monitor_id:{id}"));
            crate::events::emit(
                st,
                vec![crate::intake::logs::LogRow {
                    ts: now,
                    status: match status {
                        ALERT => "error",
                        WARN => "warn",
                        OK => "ok",
                        _ => "info",
                    }
                    .into(),
                    service: s.tags.get("service").cloned(),
                    host: s.tags.get("host").cloned(),
                    source: Some("alert".into()),
                    message: format!("{title}\n{message}"),
                    tags,
                    attributes: serde_json::Map::from_iter([
                        ("evt".to_string(), json!({"title": title, "text": message})),
                        (
                            "monitor".to_string(),
                            json!({"id": id, "name": def.name, "group": s.group, "from": prev_status, "to": status, "value": s.value}),
                        ),
                    ]),
                    trace_id: None,
                    span_id: None,
                }],
            )
            .await;
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
            ..
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
    fn new_query_kinds() {
        let p = parse("pct_change(avg(last_5m),last_1h):avg:cpu{*} by {host} > 50").unwrap();
        assert!(matches!(
            p.source,
            Source::Change {
                pct: true,
                shift_ms: 3_600_000,
                ..
            }
        ));
        assert_eq!(p.window_ms, 300_000);
        assert_eq!(type_of(&p), "query alert");
        assert!(parse("change(sum(last_1h),last_1d):sum:hits{*}.as_count() < -100").is_ok());

        let p = parse("avg(last_4h):anomalies(avg:cpu{*} by {host}, 'agile', 2, direction='above', alert_window='last_15m') >= 1").unwrap();
        let Source::Function {
            func,
            base,
            formula,
            query,
            ..
        } = &p.source
        else {
            panic!()
        };
        assert_eq!(func, "anomalies");
        assert_eq!(base, "avg:cpu{*} by {host}");
        assert!(formula.starts_with("anomalies(q, 'agile'"));
        assert_eq!(query.by, vec!["host"]);
        assert!(parse("max(next_1w):forecast(avg:disk{*}, 'linear', 1) >= 90").is_ok());
        assert!(parse("max(last_1w):forecast(avg:disk{*}, 'linear', 1) >= 90").is_err());
        assert!(parse("avg(next_1h):avg:disk{*} > 1").is_err());
        assert!(parse("avg(last_5m):nope(avg:x{*}) > 1").is_err());
        assert!(parse("avg(last_5m):ewma_10(avg:x{*}) > 1").is_ok());

        let p = parse(r#"rum("@type:error").rollup("count").by("@view.name").last("5m") > 10"#)
            .unwrap();
        assert_eq!(type_of(&p), "rum alert");
        let p = parse(r#"events("source:alert").rollup("count").last("1h") > 0"#).unwrap();
        assert_eq!(type_of(&p), "event-v2 alert");
        let p = parse(r#"error-tracking-traces("service:web").rollup("count").by("@issue.id").last("1h") > 5"#).unwrap();
        let Source::ErrorTracking { sources, by, .. } = &p.source else {
            panic!()
        };
        assert_eq!(sources, &vec!["spans"]);
        assert_eq!(by, &vec!["issue.id".to_string()]);
        assert!(parse(r#"error-tracking("*").rollup("count").by("host").last("1h") > 5"#).is_err());

        let p = parse(
            r#""datadog.agent.up".over("env:prod","*").by("host").last(3).count_by_status()"#,
        )
        .unwrap();
        let Source::ServiceCheck {
            check,
            filter,
            by,
            last,
        } = &p.source
        else {
            panic!()
        };
        assert_eq!(
            (check.as_str(), filter.as_str(), last),
            ("datadog.agent.up", "env:prod", &3)
        );
        assert_eq!(by, &vec!["host".to_string()]);
        assert_eq!(type_of(&p), "service check");

        let p = parse("12 && !(34 || 56)").unwrap();
        assert_eq!(type_of(&p), "composite");
        assert!(parse("12 &&").is_err());
        assert!(parse("12 & 3").is_err());
    }

    #[test]
    fn slo_alerts() {
        let p = parse(r#"burn_rate("abc").over("7d").long_window("1h").short_window("5m") > 14.4"#)
            .unwrap();
        assert_eq!(type_of(&p), "slo alert");
        let Source::Slo {
            slo_id,
            timeframe,
            alert,
        } = &p.source
        else {
            panic!()
        };
        assert_eq!((slo_id.as_str(), timeframe.as_str()), ("abc", "7d"));
        assert_eq!(
            alert,
            &SloAlert::BurnRate {
                long_ms: 3_600_000,
                short_ms: 300_000
            }
        );
        assert_eq!(p.threshold, 14.4);
        let p = parse(r#"error_budget("abc").over("30d") > 75"#).unwrap();
        assert!(matches!(
            p.source,
            Source::Slo {
                alert: SloAlert::ErrorBudget,
                ..
            }
        ));
        assert!(parse(r#"error_budget("abc").over("1y") > 75"#).is_err());
        assert!(
            parse(r#"burn_rate("abc").over("30d").long_window("1h").short_window("2h") > 1"#)
                .is_err()
        );
        assert!(parse(r#"burn_rate("abc").over("30d").nope("1h") > 1"#).is_err());
    }

    #[test]
    fn composite_logic() {
        let c = Composite::parse("1 && (2 || !3)").unwrap();
        let mut ids = Vec::new();
        c.ids(&mut ids);
        assert_eq!(ids, vec![1, 2, 3]);
        let states = |a: &str, b: &str, d: &str| {
            let (a, b, d) = (a.to_string(), b.to_string(), d.to_string());
            move |id: i64| {
                Some(match id {
                    1 => a.clone(),
                    2 => b.clone(),
                    _ => d.clone(),
                })
            }
        };
        assert_eq!(c.eval(&states(ALERT, OK, OK)), Some((true, 2)));
        assert_eq!(c.eval(&states(WARN, ALERT, ALERT)), Some((true, 1)));
        assert_eq!(c.eval(&states(OK, ALERT, OK)), Some((false, 0)));
        assert_eq!(c.eval(&states(ALERT, OK, ALERT)), Some((false, 0)));
        assert_eq!(c.eval(&states(ALERT, NO_DATA, ALERT)), None);
        assert_eq!(c.eval(&states(OK, NO_DATA, ALERT)), Some((false, 0)));
    }

    #[test]
    fn states_and_recovery() {
        let p = parse("avg(last_5m):avg:cpu{*} > 80").unwrap();
        let th = Thresholds {
            critical: Some(80.0),
            warning: Some(60.0),
            critical_recovery: Some(70.0),
            warning_recovery: None,
            ok: None,
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
