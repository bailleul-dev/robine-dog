//! Service level objectives: their definitions, their corrections, and how
//! good they are over a window. Each type gives, per time bucket, a good and
//! a total: events for metric SLOs (numerator and denominator queries),
//! seconds for monitor-based SLOs (time out of `Alert`) and time-slice SLOs
//! (slices whose value meets the condition). Corrected periods are taken
//! out of both.

use crate::AppState;
use crate::api::ApiError;
use crate::api::query::{FormulaDef, QueryDef, Request};
use chrono::{Datelike, Duration, Months, NaiveDate, NaiveDateTime, TimeZone, Weekday};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex};

pub const KIND: &str = "slo";
pub const CORRECTION_KIND: &str = "slo_correction";
const DAY: i64 = 86_400_000;

pub fn timeframe_ms(tf: &str) -> Option<i64> {
    match tf {
        "7d" => Some(7 * DAY),
        "30d" => Some(30 * DAY),
        "90d" => Some(90 * DAY),
        _ => None,
    }
}

// --- definitions -----------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SloQuery {
    /// Good events: a metric query, or `a - b` (all minus bad).
    pub numerator: String,
    pub denominator: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Threshold {
    /// `7d`, `30d` or `90d`.
    pub timeframe: String,
    /// Percent.
    pub target: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warning: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SliceQuery {
    #[serde(default = "metrics")]
    pub data_source: String,
    pub name: String,
    pub query: String,
}

fn metrics() -> String {
    "metrics".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SliceFormula {
    pub formula: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SliceQueries {
    pub formulas: Vec<SliceFormula>,
    pub queries: Vec<SliceQuery>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeSlice {
    pub query: SliceQueries,
    /// `>`, `>=`, `<` or `<=`: what a good slice's value is to the threshold.
    pub comparator: String,
    pub threshold: f64,
    /// 60 or 300.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query_interval_seconds: Option<i64>,
}

impl TimeSlice {
    pub fn interval_ms(&self) -> i64 {
        if self.query_interval_seconds == Some(60) {
            60_000
        } else {
            300_000
        }
    }

    pub fn good(&self, v: f64) -> bool {
        match self.comparator.as_str() {
            ">" => v > self.threshold,
            ">=" => v >= self.threshold,
            "<" => v < self.threshold,
            _ => v <= self.threshold,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SliSpec {
    pub time_slice: TimeSlice,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Slo {
    #[serde(default)]
    pub id: Option<String>,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    /// `metric`, `monitor` or `time_slice`.
    #[serde(rename = "type", default)]
    pub typ: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query: Option<SloQuery>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub monitor_ids: Vec<i64>,
    /// Monitor-based SLOs: only these groups of a multi-alert monitor.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sli_specification: Option<SliSpec>,
    #[serde(default)]
    pub thresholds: Vec<Threshold>,
    /// Datadog's shorthand for the first threshold.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeframe: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_threshold: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warning_threshold: Option<f64>,
    #[serde(default)]
    pub tags: Vec<String>,
    /// Epoch seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modified_at: Option<i64>,
}

impl Slo {
    /// The target of a time frame, else the first one.
    pub fn threshold(&self, tf: Option<&str>) -> Option<&Threshold> {
        tf.and_then(|t| self.thresholds.iter().find(|x| x.timeframe == t))
            .or(self.thresholds.first())
    }

    /// What the good and total count.
    pub fn unit(&self) -> &'static str {
        if self.typ == "metric" {
            "events"
        } else {
            "seconds"
        }
    }
}

pub async fn get(st: &AppState, id: &str) -> anyhow::Result<Option<Slo>> {
    Ok(st
        .store
        .pg
        .doc(KIND, id)
        .await?
        .and_then(|d| serde_json::from_value(d.body).ok()))
}

pub async fn all(st: &AppState) -> anyhow::Result<Vec<Slo>> {
    Ok(st
        .store
        .pg
        .docs(KIND)
        .await?
        .into_iter()
        .filter_map(|d| serde_json::from_value(d.body).ok())
        .collect())
}

// --- corrections -----------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Correction {
    #[serde(default)]
    pub id: Option<String>,
    pub slo_id: String,
    /// Epoch seconds.
    pub start: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end: Option<i64>,
    /// Seconds; with `rrule`, each occurrence's length.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration: Option<i64>,
    /// `FREQ=DAILY|WEEKLY|MONTHLY;INTERVAL=…;BYDAY=…;COUNT=…;UNTIL=…`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rrule: Option<String>,
    #[serde(default = "other")]
    pub category: String,
    #[serde(default)]
    pub description: Option<String>,
    /// For recurring corrections: the zone the occurrences follow.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timezone: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<i64>,
}

fn other() -> String {
    "Other".into()
}

pub const CATEGORIES: [&str; 4] = [
    "Scheduled Maintenance",
    "Outside Business Hours",
    "Deployment",
    "Other",
];

#[derive(Debug, Clone, Copy, PartialEq)]
enum Freq {
    Daily,
    Weekly,
    Monthly,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RRule {
    freq: Freq,
    interval: u32,
    count: Option<u32>,
    /// Epoch ms.
    until: Option<i64>,
    by_day: Vec<Weekday>,
}

pub fn parse_rrule(s: &str) -> Result<RRule, String> {
    let s = s.trim();
    let s = s.strip_prefix("RRULE:").unwrap_or(s);
    let mut rule = RRule {
        freq: Freq::Daily,
        interval: 1,
        count: None,
        until: None,
        by_day: Vec::new(),
    };
    let mut freq = None;
    for part in s.split(';').filter(|p| !p.trim().is_empty()) {
        let (k, v) = part
            .split_once('=')
            .ok_or_else(|| format!("bad rule part `{part}`"))?;
        let v = v.trim();
        match k.trim().to_ascii_uppercase().as_str() {
            "FREQ" => {
                freq = Some(match v.to_ascii_uppercase().as_str() {
                    "DAILY" => Freq::Daily,
                    "WEEKLY" => Freq::Weekly,
                    "MONTHLY" => Freq::Monthly,
                    other => return Err(format!("FREQ={other} is not supported")),
                })
            }
            "INTERVAL" => {
                rule.interval = v.parse().ok().filter(|i| *i > 0).ok_or("bad INTERVAL")?
            }
            "COUNT" => rule.count = Some(v.parse().map_err(|_| "bad COUNT")?),
            "UNTIL" => {
                let v = v.trim_end_matches('Z');
                let t = if v.len() == 8 {
                    NaiveDate::parse_from_str(v, "%Y%m%d")
                        .ok()
                        .and_then(|d| d.and_hms_opt(23, 59, 59))
                } else {
                    NaiveDateTime::parse_from_str(v, "%Y%m%dT%H%M%S").ok()
                };
                rule.until = Some(t.ok_or("bad UNTIL")?.and_utc().timestamp_millis());
            }
            "BYDAY" => {
                for d in v.split(',') {
                    rule.by_day
                        .push(match d.trim().to_ascii_uppercase().as_str() {
                            "MO" => Weekday::Mon,
                            "TU" => Weekday::Tue,
                            "WE" => Weekday::Wed,
                            "TH" => Weekday::Thu,
                            "FR" => Weekday::Fri,
                            "SA" => Weekday::Sat,
                            "SU" => Weekday::Sun,
                            other => return Err(format!("bad BYDAY `{other}`")),
                        });
                }
            }
            "WKST" => {}
            other => return Err(format!("{other} is not supported")),
        }
    }
    rule.freq = freq.ok_or("the rule needs a FREQ")?;
    rule.by_day.sort_by_key(|d| d.num_days_from_monday());
    rule.by_day.dedup();
    Ok(rule)
}

impl Correction {
    /// The corrected periods within `[from, to)`, in epoch ms.
    pub fn windows(&self, from: i64, to: i64) -> Vec<(i64, i64)> {
        let start = self.start * 1000;
        let len = match (self.duration, self.end) {
            (Some(d), _) => d * 1000,
            (None, Some(e)) => (e - self.start) * 1000,
            _ => 0,
        };
        if len <= 0 {
            return Vec::new();
        }
        let clip = |(a, b): (i64, i64)| (b > from && a < to).then(|| (a.max(from), b.min(to)));
        let Some(rule) = self.rrule.as_deref().filter(|r| !r.trim().is_empty()) else {
            return clip((start, start + len)).into_iter().collect();
        };
        let Ok(rule) = parse_rrule(rule) else {
            return Vec::new();
        };
        let tz: chrono_tz::Tz = self
            .timezone
            .as_deref()
            .and_then(|t| t.parse().ok())
            .unwrap_or(chrono_tz::UTC);
        let Some(local) = tz
            .timestamp_millis_opt(start)
            .single()
            .map(|d| d.naive_local())
        else {
            return Vec::new();
        };
        let to_ms = |n: NaiveDateTime| {
            tz.from_local_datetime(&n)
                .earliest()
                .map(|d| d.timestamp_millis())
        };
        let mut out = Vec::new();
        let mut n = 0u32;
        // Whether to go on after the occurrence at `occ`.
        let mut push = |occ: i64| -> bool {
            if occ >= to || rule.until.is_some_and(|u| occ > u) {
                return false;
            }
            n += 1;
            if rule.count.is_some_and(|c| n > c) {
                return false;
            }
            if let Some(w) = clip((occ, occ + len)) {
                out.push(w);
            }
            true
        };
        let step = rule.interval as i64;
        'outer: for k in 0..200_000i64 {
            match rule.freq {
                Freq::Daily => {
                    let Some(t) = to_ms(local + Duration::days(k * step)) else {
                        continue;
                    };
                    if !push(t) {
                        break;
                    }
                }
                Freq::Monthly => {
                    let Some(occ) = local.checked_add_months(Months::new((k * step) as u32)) else {
                        break;
                    };
                    let Some(t) = to_ms(occ) else { continue };
                    if !push(t) {
                        break;
                    }
                }
                Freq::Weekly => {
                    let days = if rule.by_day.is_empty() {
                        vec![local.weekday()]
                    } else {
                        rule.by_day.clone()
                    };
                    let monday = local.date()
                        - Duration::days(local.weekday().num_days_from_monday() as i64);
                    for d in days {
                        let date =
                            monday + Duration::days(7 * k * step + d.num_days_from_monday() as i64);
                        let occ = date.and_time(local.time());
                        if occ < local {
                            continue;
                        }
                        let Some(t) = to_ms(occ) else { continue };
                        if !push(t) {
                            break 'outer;
                        }
                    }
                }
            }
        }
        out
    }
}

/// Bumped on every correction change, so cached statuses are recomputed.
pub static CORRECTIONS_GENERATION: AtomicU64 = AtomicU64::new(0);

pub async fn corrections(st: &AppState, slo_id: Option<&str>) -> anyhow::Result<Vec<Correction>> {
    Ok(st
        .store
        .pg
        .docs(CORRECTION_KIND)
        .await?
        .into_iter()
        .filter_map(|d| serde_json::from_value::<Correction>(d.body).ok())
        .filter(|c| slo_id.is_none_or(|id| c.slo_id == id))
        .collect())
}

// --- intervals -------------------------------------------------------------------

/// Sorted, overlapping or touching intervals joined.
pub fn merge(mut v: Vec<(i64, i64)>) -> Vec<(i64, i64)> {
    v.retain(|(a, b)| b > a);
    v.sort();
    let mut out: Vec<(i64, i64)> = Vec::new();
    for (a, b) in v {
        match out.last_mut() {
            Some(last) if a <= last.1 => last.1 = last.1.max(b),
            _ => out.push((a, b)),
        }
    }
    out
}

/// How much of `[a, b)` merged intervals cover.
pub fn overlap(a: i64, b: i64, set: &[(i64, i64)]) -> i64 {
    set.iter()
        .map(|(x, y)| ((*y).min(b) - (*x).max(a)).max(0))
        .sum()
}

/// Merged intervals minus merged intervals.
pub fn subtract(set: &[(i64, i64)], minus: &[(i64, i64)]) -> Vec<(i64, i64)> {
    let mut out = Vec::new();
    for &(mut a, b) in set {
        for &(x, y) in minus {
            if y <= a || x >= b {
                continue;
            }
            if x > a {
                out.push((a, x));
            }
            a = a.max(y);
            if a >= b {
                break;
            }
        }
        if a < b {
            out.push((a, b));
        }
    }
    out
}

// --- computation -----------------------------------------------------------------

/// Good and total per bucket of `step` ms from `start`.
#[derive(Debug, Clone, Serialize)]
pub struct Buckets {
    pub start: i64,
    pub step: i64,
    pub good: Vec<f64>,
    pub total: Vec<f64>,
}

impl Buckets {
    pub fn new(from: i64, to: i64, step: i64) -> Self {
        let start = from.div_euclid(step) * step;
        let len = ((to - start + step - 1) / step).max(1) as usize;
        Buckets {
            start,
            step,
            good: vec![0.0; len],
            total: vec![0.0; len],
        }
    }

    fn index(&self, t: i64) -> Option<usize> {
        let i = (t - self.start).div_euclid(self.step);
        (0..self.good.len() as i64)
            .contains(&i)
            .then_some(i as usize)
    }

    pub fn times(&self) -> Vec<i64> {
        (0..self.good.len())
            .map(|i| self.start + i as i64 * self.step)
            .collect()
    }

    /// Good and total of the buckets ending after `since`.
    pub fn sums_since(&self, since: i64) -> Sli {
        let mut s = Sli::default();
        for (i, t) in self.times().into_iter().enumerate() {
            if t + self.step > since {
                s.good += self.good[i];
                s.total += self.total[i];
            }
        }
        s
    }

    pub fn sums(&self) -> Sli {
        self.sums_since(i64::MIN)
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct Sli {
    pub good: f64,
    pub total: f64,
}

impl Sli {
    /// Percent good, if anything was counted.
    pub fn value(&self) -> Option<f64> {
        (self.total > 0.0).then(|| (self.good / self.total * 100.0).clamp(0.0, 100.0))
    }

    pub fn bad(&self) -> f64 {
        (self.total - self.good).max(0.0)
    }

    /// What is left of the failures the target allows, in percent of them
    /// (negative once over).
    pub fn budget_remaining(&self, target: f64) -> Option<f64> {
        if self.total <= 0.0 {
            return None;
        }
        let allowed = (1.0 - target / 100.0) * self.total;
        if allowed <= 0.0 {
            return Some(if self.bad() > 0.0 { -100.0 } else { 100.0 });
        }
        Some((allowed - self.bad()) / allowed * 100.0)
    }

    /// How many times faster than allowed the budget is spent.
    pub fn burn_rate(&self, target: f64) -> Option<f64> {
        if self.total <= 0.0 {
            return None;
        }
        let allowed = 1.0 - target / 100.0;
        (allowed > 0.0).then(|| (self.bad() / self.total) / allowed)
    }
}

/// The corrected periods of an SLO within a window, merged.
async fn excluded(st: &AppState, slo: &Slo, from: i64, to: i64) -> anyhow::Result<Vec<(i64, i64)>> {
    let Some(id) = &slo.id else {
        return Ok(Vec::new());
    };
    Ok(merge(
        corrections(st, Some(id))
            .await?
            .iter()
            .flat_map(|c| c.windows(from, to))
            .collect(),
    ))
}

fn query_def(name: &str, query: &str, source: &str) -> Result<QueryDef, ApiError> {
    serde_json::from_value(json!({"data_source": source, "name": name, "query": query}))
        .map_err(|e| ApiError::BadQuery(e.to_string()))
}

fn formula(f: String) -> FormulaDef {
    FormulaDef {
        formula: f,
        alias: None,
        limit: None,
    }
}

/// Good and total per `step` over `[from, to)`.
pub async fn compute(
    st: &AppState,
    slo: &Slo,
    from: i64,
    to: i64,
    step: i64,
) -> Result<Buckets, ApiError> {
    // The query engine's grids have at most 5,000 points.
    let step = step.max(60_000).max((to - from) / 4000);
    let step = (step + 59_999) / 60_000 * 60_000;
    let ex = excluded(st, slo, from, to).await?;
    match slo.typ.as_str() {
        "monitor" => monitor_buckets(st, slo, from, to, step, &ex).await,
        "time_slice" => slice_buckets(st, slo, from, to, step, &ex).await,
        _ => metric_buckets(st, slo, from, to, step, &ex).await,
    }
}

async fn metric_buckets(
    st: &AppState,
    slo: &Slo,
    from: i64,
    to: i64,
    step: i64,
    ex: &[(i64, i64)],
) -> Result<Buckets, ApiError> {
    let q = slo
        .query
        .as_ref()
        .ok_or_else(|| ApiError::BadQuery("a metric SLO needs a query".into()))?;
    let mut queries = Vec::new();
    let mut formulas = Vec::new();
    for (prefix, expr) in [("n", &q.numerator), ("d", &q.denominator)] {
        let mut names = Vec::new();
        for (i, term) in expr.split(" - ").map(str::trim).enumerate() {
            let name = format!("{prefix}{i}");
            queries.push(query_def(&name, term, "metrics")?);
            names.push(name);
        }
        formulas.push(formula(names.join(" - ")));
    }
    let req = Request {
        from,
        to,
        interval: Some(step),
        queries,
        formulas,
    };
    let out = crate::api::query::execute(st, &req, false).await?;
    let mut b = Buckets::new(from, to, step);
    for (idx, _, series) in &out.series {
        for (i, v) in series.values.iter().enumerate() {
            let (Some(v), Some(k)) = (v, out.times.get(i).and_then(|t| b.index(*t))) else {
                continue;
            };
            if *idx == 0 {
                b.good[k] += v;
            } else {
                b.total[k] += v;
            }
        }
    }
    for (k, t) in b.times().into_iter().enumerate() {
        let left = 1.0 - overlap(t, t + step, ex) as f64 / step as f64;
        b.good[k] = (b.good[k] * left).max(0.0);
        b.total[k] *= left;
    }
    Ok(b)
}

/// The periods any of an SLO's monitors (or chosen groups) was in `Alert`.
pub async fn downtime(st: &AppState, slo: &Slo, to: i64) -> anyhow::Result<Vec<(i64, i64)>> {
    let rows = st
        .store
        .pg
        .monitor_transitions(&slo.monitor_ids, to)
        .await?;
    let mut timelines: BTreeMap<(i64, String), Vec<(i64, String)>> = BTreeMap::new();
    for (m, g, ts, status) in rows {
        if slo.groups.is_empty() || slo.groups.contains(&g) {
            timelines.entry((m, g)).or_default().push((ts, status));
        }
    }
    let mut down = Vec::new();
    for steps in timelines.values() {
        let mut since = None;
        for (ts, status) in steps {
            match (status == crate::monitors::ALERT, since) {
                (true, None) => since = Some(*ts),
                (false, Some(s)) => {
                    down.push((s, *ts));
                    since = None;
                }
                _ => {}
            }
        }
        if let Some(s) = since {
            down.push((s, to));
        }
    }
    Ok(merge(down))
}

async fn monitor_buckets(
    st: &AppState,
    slo: &Slo,
    from: i64,
    to: i64,
    step: i64,
    ex: &[(i64, i64)],
) -> Result<Buckets, ApiError> {
    if slo.monitor_ids.is_empty() {
        return Err(ApiError::BadQuery("a monitor SLO needs monitors".into()));
    }
    let down = subtract(&downtime(st, slo, to).await?, ex);
    let mut b = Buckets::new(from, to, step);
    for (k, t) in b.times().into_iter().enumerate() {
        let (a, z) = (t.max(from), (t + step).min(to));
        if z <= a {
            continue;
        }
        let total = (z - a - overlap(a, z, ex)) as f64 / 1000.0;
        let bad = overlap(a, z, &down) as f64 / 1000.0;
        b.total[k] = total;
        b.good[k] = (total - bad).max(0.0);
    }
    Ok(b)
}

async fn slice_buckets(
    st: &AppState,
    slo: &Slo,
    from: i64,
    to: i64,
    step: i64,
    ex: &[(i64, i64)],
) -> Result<Buckets, ApiError> {
    let spec = &slo
        .sli_specification
        .as_ref()
        .ok_or_else(|| ApiError::BadQuery("a time-slice SLO needs a specification".into()))?
        .time_slice;
    let iv = spec.interval_ms();
    let queries: Vec<QueryDef> = spec
        .query
        .queries
        .iter()
        .map(|q| query_def(&q.name, &q.query, &q.data_source))
        .collect::<Result<_, _>>()?;
    let formulas: Vec<FormulaDef> = spec
        .query
        .formulas
        .iter()
        .take(1)
        .map(|f| formula(f.formula.clone()))
        .collect();
    let mut b = Buckets::new(from, to, step);
    let secs = iv as f64 / 1000.0;
    let mut t0 = from.div_euclid(iv) * iv;
    while t0 < to {
        let t1 = (t0 + 4000 * iv).min(to);
        let req = Request {
            from: t0,
            to: t1,
            interval: Some(iv),
            queries: queries.clone(),
            formulas: formulas.clone(),
        };
        let out = crate::api::query::execute(st, &req, false).await?;
        for (i, t) in out.times.iter().enumerate() {
            if *t < t0 || *t >= t1 || *t < from || overlap(*t, *t + iv, ex) * 2 >= iv {
                continue;
            }
            // No data is uptime, as in Datadog.
            let bad = out.series.iter().any(|(idx, _, s)| {
                *idx == 0
                    && s.values
                        .get(i)
                        .copied()
                        .flatten()
                        .is_some_and(|v| !spec.good(v))
            });
            if let Some(k) = b.index((*t).max(from)) {
                b.total[k] += secs;
                if !bad {
                    b.good[k] += secs;
                }
            }
        }
        t0 = t1;
    }
    Ok(b)
}

// --- status ----------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct Status {
    pub timeframe: String,
    pub target: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warning: Option<f64>,
    /// Percent good.
    pub sli: Option<f64>,
    pub good: f64,
    pub total: f64,
    pub error_budget_remaining: Option<f64>,
    /// `ok`, `warning`, `breached` or `no_data`.
    pub state: &'static str,
    pub unit: &'static str,
}

type CacheKey = (String, Option<i64>, String, u64);
static CACHE: LazyLock<Mutex<HashMap<CacheKey, (i64, Status)>>> = LazyLock::new(Default::default);

/// An SLO over one of its time frames, up to `now`; cached a minute.
pub async fn status(
    st: &AppState,
    slo: &Slo,
    timeframe: &str,
    now: i64,
) -> Result<Status, ApiError> {
    let key: CacheKey = (
        slo.id.clone().unwrap_or_default(),
        slo.modified_at,
        timeframe.to_string(),
        CORRECTIONS_GENERATION.load(Ordering::Relaxed),
    );
    if let Some((at, s)) = CACHE.lock().ok().and_then(|c| c.get(&key).cloned())
        && now - at < 60_000
    {
        return Ok(s);
    }
    let span = timeframe_ms(timeframe)
        .ok_or_else(|| ApiError::BadQuery(format!("bad time frame {timeframe}")))?;
    let th = slo
        .threshold(Some(timeframe))
        .cloned()
        .ok_or_else(|| ApiError::BadQuery("the SLO has no target".into()))?;
    let b = compute(st, slo, now - span, now, span / 720).await?;
    let sli = b.sums();
    let value = sli.value();
    let state = match value {
        None => "no_data",
        Some(v) if v < th.target => "breached",
        Some(v) if th.warning.is_some_and(|w| v < w) => "warning",
        Some(_) => "ok",
    };
    let s = Status {
        timeframe: timeframe.to_string(),
        target: th.target,
        warning: th.warning,
        sli: value,
        good: sli.good,
        total: sli.total,
        error_budget_remaining: sli.budget_remaining(th.target),
        state,
        unit: slo.unit(),
    };
    if let Ok(mut c) = CACHE.lock() {
        c.retain(|_, (at, _)| now - *at < 60_000);
        c.insert(key, (now, s.clone()));
    }
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(s: &str) -> i64 {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M")
            .unwrap()
            .and_utc()
            .timestamp_millis()
    }

    #[test]
    fn intervals() {
        let m = merge(vec![(5, 8), (1, 3), (2, 4), (8, 9)]);
        assert_eq!(m, vec![(1, 4), (5, 9)]);
        assert_eq!(overlap(0, 6, &m), 4);
        assert_eq!(subtract(&m, &[(2, 6)]), vec![(1, 2), (6, 9)]);
        assert_eq!(subtract(&[(0, 10)], &[(0, 10)]), vec![]);
    }

    #[test]
    fn sli_maths() {
        let s = Sli {
            good: 999.0,
            total: 1000.0,
        };
        assert_eq!(s.value(), Some(99.9));
        // 99.5 % allows 5 failures of 1000; 1 spent.
        assert!((s.budget_remaining(99.5).unwrap() - 80.0).abs() < 1e-9);
        assert!((s.burn_rate(99.9).unwrap() - 1.0).abs() < 1e-9);
        assert!(s.budget_remaining(99.99).unwrap() < 0.0);
        assert_eq!(Sli::default().value(), None);
    }

    #[test]
    fn rrules() {
        assert!(parse_rrule("FREQ=YEARLY").is_err());
        assert!(parse_rrule("INTERVAL=2").is_err());
        let r = parse_rrule("FREQ=WEEKLY;BYDAY=FR,MO;INTERVAL=1").unwrap();
        assert_eq!(r.by_day, vec![Weekday::Mon, Weekday::Fri]);

        // Every day 22:00–23:00 UTC, three times.
        let c = Correction {
            id: None,
            slo_id: "x".into(),
            start: at("2026-01-05 22:00") / 1000,
            end: None,
            duration: Some(3600),
            rrule: Some("FREQ=DAILY;COUNT=3".into()),
            category: "Other".into(),
            description: None,
            timezone: None,
            created_at: None,
        };
        let w = c.windows(at("2026-01-01 00:00"), at("2026-02-01 00:00"));
        assert_eq!(w.len(), 3);
        assert_eq!(w[2], (at("2026-01-07 22:00"), at("2026-01-07 23:00")));
        // Clipped to the window asked.
        let w = c.windows(at("2026-01-06 22:30"), at("2026-02-01 00:00"));
        assert_eq!(w[0], (at("2026-01-06 22:30"), at("2026-01-06 23:00")));

        // Mondays and Fridays at 09:00 Paris time (08:00 UTC in winter).
        let c = Correction {
            start: at("2026-01-05 08:00") / 1000,
            rrule: Some("FREQ=WEEKLY;BYDAY=MO,FR;UNTIL=20260112".into()),
            timezone: Some("Europe/Paris".into()),
            ..c
        };
        let w = c.windows(at("2026-01-01 00:00"), at("2026-02-01 00:00"));
        assert_eq!(
            w.iter().map(|x| x.0).collect::<Vec<_>>(),
            vec![
                at("2026-01-05 08:00"),
                at("2026-01-09 08:00"),
                at("2026-01-12 08:00")
            ]
        );

        // A one-off correction.
        let c = Correction {
            rrule: None,
            duration: None,
            end: Some(at("2026-01-05 10:00") / 1000),
            ..c
        };
        assert_eq!(
            c.windows(0, i64::MAX),
            vec![(at("2026-01-05 08:00"), at("2026-01-05 10:00"))]
        );
    }

    #[test]
    fn buckets() {
        let mut b = Buckets::new(60_000, 360_000, 60_000);
        assert_eq!(b.good.len(), 5);
        b.good = vec![1.0, 1.0, 1.0, 0.0, 1.0];
        b.total = vec![1.0; 5];
        assert_eq!(b.sums().good, 4.0);
        assert_eq!(b.sums_since(240_000).total, 2.0);
        let ts = TimeSlice {
            query: SliceQueries {
                formulas: vec![],
                queries: vec![],
            },
            comparator: "<".into(),
            threshold: 5.0,
            query_interval_seconds: None,
        };
        assert!(ts.good(4.0) && !ts.good(5.0));
        assert_eq!(ts.interval_ms(), 300_000);
    }
}
