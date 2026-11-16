//! Datadog's v2 query API (`/api/v2/query/timeseries`, `/api/v2/query/scalar`),
//! what dashboards run: named queries over metrics, logs or spans, on one
//! time grid, and formulas over them (`a / b * 100`, `per_second(a)`,
//! `timeshift(a, -3600)`, `top(a, 5, 'mean', 'desc')`…). Series are matched
//! by group tags; an ungrouped series applies to every group.

use super::{ApiError, analytics};
use crate::AppState;
use crate::metrics::query::{self as mq};
use crate::metrics::{FLUSH_INTERVAL_MS, Kind};
use crate::query::sql::Dataset;
use crate::store::greptime::{f64_at, i64_at, str_at};
use axum::Json;
use axum::extract::State;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap};

#[derive(Debug, Clone, Deserialize)]
pub struct QueryDef {
    #[serde(default = "metrics_source")]
    pub data_source: String,
    #[serde(default)]
    pub name: String,
    /// Metrics: the query (`avg:cpu{env:prod} by {host}`).
    #[serde(default)]
    pub query: String,
    /// Scalar reduction: avg (default), sum, min, max, last.
    #[serde(default)]
    pub aggregator: Option<String>,
    /// Logs / spans.
    #[serde(default)]
    pub search: Option<Search>,
    #[serde(default)]
    pub compute: Option<EventCompute>,
    #[serde(default)]
    pub group_by: Vec<EventGroupBy>,
}

fn metrics_source() -> String {
    "metrics".into()
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Search {
    #[serde(default)]
    pub query: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct EventCompute {
    #[serde(default = "count")]
    pub aggregation: String,
    #[serde(default)]
    pub metric: Option<String>,
}

fn count() -> String {
    "count".into()
}

#[derive(Debug, Clone, Deserialize)]
pub struct EventGroupBy {
    pub facet: String,
    #[serde(default)]
    pub limit: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FormulaDef {
    pub formula: String,
    #[serde(default)]
    pub alias: Option<String>,
    #[serde(default)]
    pub limit: Option<FormulaLimit>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FormulaLimit {
    #[serde(default)]
    pub count: Option<usize>,
    #[serde(default)]
    pub order: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Request {
    pub from: i64,
    pub to: i64,
    #[serde(default)]
    pub interval: Option<i64>,
    pub queries: Vec<QueryDef>,
    #[serde(default)]
    pub formulas: Vec<FormulaDef>,
}

type Tags = BTreeMap<String, String>;

#[derive(Debug, Clone)]
pub struct Series {
    pub tags: Tags,
    pub values: Vec<Option<f64>>,
}

struct Grid {
    from: i64,
    interval: i64,
    len: usize,
}

impl Grid {
    fn index(&self, t: i64) -> Option<usize> {
        let i = (t - self.from).div_euclid(self.interval);
        (0..self.len as i64).contains(&i).then_some(i as usize)
    }

    fn times(&self) -> Vec<i64> {
        (0..self.len)
            .map(|i| self.from + i as i64 * self.interval)
            .collect()
    }
}

async fn run_query(
    st: &AppState,
    q: &QueryDef,
    from: i64,
    to: i64,
    grid: &Grid,
    scalar: bool,
) -> Result<Vec<Series>, ApiError> {
    match q.data_source.as_str() {
        "metrics" => {
            let parsed =
                mq::parse(&q.query).map_err(|e| ApiError::BadQuery(format!("{}: {e}", q.name)))?;
            let kind = st
                .store
                .pg
                .metric(&parsed.metric)
                .await?
                .map(|m| Kind::parse(&m.typ))
                .unwrap_or(Kind::Gauge);
            let rows = st
                .store
                .gt
                .query(&mq::build_sql(&parsed, kind, from, to, grid.interval))
                .await?;
            let n = parsed.by.len();
            let mut groups: BTreeMap<Vec<String>, Vec<Option<f64>>> = BTreeMap::new();
            for r in &rows {
                let key: Vec<String> = (0..n)
                    .map(|g| str_at(r, 1 + g).unwrap_or_else(|| "N/A".into()))
                    .collect();
                let Some(i) = grid.index(i64_at(r, 0)) else {
                    continue;
                };
                groups.entry(key).or_insert_with(|| vec![None; grid.len])[i] = f64_at(r, 1 + n);
            }
            let how = q.aggregator.clone().unwrap_or_else(|| "avg".into());
            Ok(groups
                .into_iter()
                .map(|(k, values)| Series {
                    tags: parsed.by.iter().cloned().zip(k).collect(),
                    values: if scalar {
                        vec![reduce(&values, &how)]
                    } else {
                        values
                    },
                })
                .collect())
        }
        ds @ ("logs" | "spans") => {
            let dataset = if ds == "logs" {
                Dataset::Logs
            } else {
                Dataset::Spans
            };
            let compute = q.compute.clone().unwrap_or(EventCompute {
                aggregation: "count".into(),
                metric: None,
            });
            let req = analytics::AggregateReq {
                compute: vec![analytics::Compute {
                    aggregation: compute.aggregation.clone(),
                    metric: compute.metric.clone(),
                    typ: Some(if scalar { "total" } else { "timeseries" }.into()),
                    interval: (!scalar).then(|| json!(grid.interval)),
                }],
                filter: analytics::Filter {
                    query: q.search.clone().unwrap_or_default().query,
                    from: json!(from),
                    to: json!(to),
                },
                group_by: q
                    .group_by
                    .iter()
                    .map(|g| analytics::GroupBy {
                        facet: g.facet.clone(),
                        limit: g.limit.or(Some(10)),
                        sort: None,
                        missing: None,
                    })
                    .collect(),
            };
            let out = analytics::run(st, dataset, &req, to).await?;
            let zero = (compute.aggregation == "count").then_some(0.0);
            Ok(out
                .buckets
                .iter()
                .map(|b| {
                    if scalar {
                        let v = b["computes"]["c0"].as_f64().or(zero);
                        let tags = b["by"]
                            .as_object()
                            .map(|o| {
                                o.iter()
                                    .map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_string()))
                                    .collect()
                            })
                            .unwrap_or_default();
                        return Series {
                            tags,
                            values: vec![v],
                        };
                    }
                    let mut values = vec![zero; grid.len];
                    for p in b["computes"]["c0"].as_array().into_iter().flatten() {
                        let t = p["time"]
                            .as_str()
                            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                            .map(|d| d.timestamp_millis());
                        if let Some(i) = t.and_then(|t| grid.index(t)) {
                            values[i] = p["value"].as_f64();
                        }
                    }
                    let tags = b["by"]
                        .as_object()
                        .map(|o| {
                            o.iter()
                                .map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_string()))
                                .collect()
                        })
                        .unwrap_or_default();
                    Series { tags, values }
                })
                .collect())
        }
        other => Err(ApiError::BadQuery(format!(
            "unsupported data_source {other}"
        ))),
    }
}

// --- formulas ------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Num(f64),
    Str(String),
    Ref(String),
    Neg(Box<Expr>),
    Bin(char, Box<Expr>, Box<Expr>),
    Call(String, Vec<Expr>),
}

pub fn parse_formula(s: &str) -> Result<Expr, String> {
    let toks = lex(s)?;
    let mut i = 0;
    let e = sum(&toks, &mut i)?;
    if i != toks.len() {
        return Err(format!("unexpected `{}` in formula", toks[i]));
    }
    Ok(e)
}

fn lex(s: &str) -> Result<Vec<String>, String> {
    let c: Vec<char> = s.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < c.len() {
        match c[i] {
            w if w.is_whitespace() => i += 1,
            '+' | '-' | '*' | '/' | '(' | ')' | ',' => {
                out.push(c[i].to_string());
                i += 1;
            }
            q @ ('\'' | '"') => {
                let st = i;
                i += 1;
                while i < c.len() && c[i] != q {
                    i += 1;
                }
                if i >= c.len() {
                    return Err("unterminated string in formula".into());
                }
                i += 1;
                out.push(c[st..i].iter().collect());
            }
            d if d.is_ascii_digit() || d == '.' => {
                let st = i;
                while i < c.len() && (c[i].is_ascii_alphanumeric() || c[i] == '.') {
                    i += 1;
                }
                out.push(c[st..i].iter().collect());
            }
            a if a.is_alphabetic() || a == '_' => {
                let st = i;
                while i < c.len() && (c[i].is_alphanumeric() || c[i] == '_') {
                    i += 1;
                }
                out.push(c[st..i].iter().collect());
            }
            other => return Err(format!("unexpected `{other}` in formula")),
        }
    }
    Ok(out)
}

fn sum(t: &[String], i: &mut usize) -> Result<Expr, String> {
    let mut l = product(t, i)?;
    while *i < t.len() && (t[*i] == "+" || t[*i] == "-") {
        let op = t[*i].chars().next().unwrap();
        *i += 1;
        l = Expr::Bin(op, Box::new(l), Box::new(product(t, i)?));
    }
    Ok(l)
}

fn product(t: &[String], i: &mut usize) -> Result<Expr, String> {
    let mut l = unary(t, i)?;
    while *i < t.len() && (t[*i] == "*" || t[*i] == "/") {
        let op = t[*i].chars().next().unwrap();
        *i += 1;
        l = Expr::Bin(op, Box::new(l), Box::new(unary(t, i)?));
    }
    Ok(l)
}

fn unary(t: &[String], i: &mut usize) -> Result<Expr, String> {
    let tok = t.get(*i).ok_or("formula ends too early")?.clone();
    *i += 1;
    if tok == "-" {
        return Ok(Expr::Neg(Box::new(unary(t, i)?)));
    }
    if tok == "(" {
        let e = sum(t, i)?;
        if t.get(*i).map(String::as_str) != Some(")") {
            return Err("missing `)` in formula".into());
        }
        *i += 1;
        return Ok(e);
    }
    if tok.starts_with('\'') || tok.starts_with('"') {
        return Ok(Expr::Str(tok[1..tok.len() - 1].to_string()));
    }
    if tok.starts_with(|c: char| c.is_ascii_digit() || c == '.') {
        // `-3600` / `1h` style durations become seconds.
        if let Some(ms) =
            analytics::parse_span_ms(&tok).filter(|_| tok.ends_with(|c: char| c.is_alphabetic()))
        {
            return Ok(Expr::Num(ms as f64 / 1000.0));
        }
        return tok
            .parse()
            .map(Expr::Num)
            .map_err(|_| format!("bad number `{tok}`"));
    }
    if t.get(*i).map(String::as_str) == Some("(") {
        *i += 1;
        let mut args = Vec::new();
        if t.get(*i).map(String::as_str) != Some(")") {
            loop {
                args.push(sum(t, i)?);
                match t.get(*i).map(String::as_str) {
                    Some(",") => *i += 1,
                    Some(")") => break,
                    _ => return Err(format!("missing `)` after {tok}(")),
                }
            }
        }
        *i += 1;
        return Ok(Expr::Call(tok, args));
    }
    Ok(Expr::Ref(tok))
}

fn refs(e: &Expr, out: &mut Vec<String>) {
    match e {
        Expr::Ref(r) => out.push(r.clone()),
        Expr::Neg(x) => refs(x, out),
        Expr::Bin(_, a, b) => {
            refs(a, out);
            refs(b, out);
        }
        Expr::Call(_, args) => args.iter().for_each(|a| refs(a, out)),
        _ => {}
    }
}

/// Timeshifts in a formula: `(query, seconds)`.
fn shifts(e: &Expr, out: &mut Vec<(String, i64)>) {
    if let Expr::Call(f, args) = e {
        let secs = match (f.as_str(), args.get(1)) {
            ("timeshift", Some(Expr::Num(s))) => Some(*s as i64),
            ("timeshift", Some(Expr::Neg(n))) => match n.as_ref() {
                Expr::Num(s) => Some(-(*s as i64)),
                _ => None,
            },
            ("hour_before", _) => Some(-3600),
            ("day_before", _) => Some(-86_400),
            ("week_before", _) => Some(-604_800),
            ("month_before", _) => Some(-2_592_000),
            _ => None,
        };
        if let (Some(s), Some(Expr::Ref(r))) = (secs, args.first()) {
            out.push((r.clone(), s));
        }
    }
    match e {
        Expr::Neg(x) => shifts(x, out),
        Expr::Bin(_, a, b) => {
            shifts(a, out);
            shifts(b, out);
        }
        Expr::Call(_, args) => args.iter().for_each(|a| shifts(a, out)),
        _ => {}
    }
}

type Data = HashMap<(String, i64), Vec<Series>>;

fn constant(v: f64, len: usize) -> Vec<Series> {
    vec![Series {
        tags: Tags::new(),
        values: vec![Some(v); len],
    }]
}

fn map(s: Vec<Series>, f: impl Fn(Option<f64>) -> Option<f64>) -> Vec<Series> {
    s.into_iter()
        .map(|x| Series {
            tags: x.tags,
            values: x.values.into_iter().map(&f).collect(),
        })
        .collect()
}

fn combine(a: Vec<Series>, b: Vec<Series>, f: impl Fn(f64, f64) -> Option<f64>) -> Vec<Series> {
    let apply = |x: &Series, y: &Series, tags: Tags| Series {
        tags,
        values: x
            .values
            .iter()
            .zip(&y.values)
            .map(|(p, q)| match (p, q) {
                (Some(p), Some(q)) => f(*p, *q),
                _ => None,
            })
            .collect(),
    };
    let single = |s: &[Series]| s.len() == 1 && s[0].tags.is_empty();
    if single(&b) {
        return a.iter().map(|x| apply(x, &b[0], x.tags.clone())).collect();
    }
    if single(&a) {
        return b.iter().map(|y| apply(&a[0], y, y.tags.clone())).collect();
    }
    // Groups match when their common tags agree.
    let mut out = Vec::new();
    for x in &a {
        for y in &b {
            if x.tags
                .iter()
                .all(|(k, v)| y.tags.get(k).is_none_or(|w| w == v))
            {
                let mut tags = x.tags.clone();
                tags.extend(y.tags.clone());
                out.push(apply(x, y, tags));
            }
        }
    }
    out
}

fn reduce(values: &[Option<f64>], how: &str) -> Option<f64> {
    let v: Vec<f64> = values.iter().flatten().copied().collect();
    if v.is_empty() {
        return None;
    }
    Some(match how {
        "sum" | "area" => v.iter().sum(),
        "min" => v.iter().copied().fold(f64::INFINITY, f64::min),
        "max" => v.iter().copied().fold(f64::NEG_INFINITY, f64::max),
        "last" => *v.last().unwrap(),
        "l2norm" | "norm" => v.iter().map(|x| x * x).sum::<f64>().sqrt(),
        _ => v.iter().sum::<f64>() / v.len() as f64,
    })
}

fn eval(e: &Expr, data: &Data, len: usize, interval_s: f64) -> Result<Vec<Series>, String> {
    Ok(match e {
        Expr::Num(n) => constant(*n, len),
        Expr::Str(_) => return Err("unexpected string in formula".into()),
        Expr::Ref(r) => data
            .get(&(r.clone(), 0))
            .cloned()
            .ok_or(format!("unknown query `{r}`"))?,
        Expr::Neg(x) => map(eval(x, data, len, interval_s)?, |v| v.map(|v| -v)),
        Expr::Bin(op, a, b) => {
            let (a, b) = (
                eval(a, data, len, interval_s)?,
                eval(b, data, len, interval_s)?,
            );
            match op {
                '+' => combine(a, b, |x, y| Some(x + y)),
                '-' => combine(a, b, |x, y| Some(x - y)),
                '*' => combine(a, b, |x, y| Some(x * y)),
                _ => combine(a, b, |x, y| (y != 0.0).then(|| x / y)),
            }
        }
        Expr::Call(f, args) => {
            let arg = |i: usize| -> Result<Vec<Series>, String> {
                eval(
                    args.get(i).ok_or(format!("{f}() needs an argument"))?,
                    data,
                    len,
                    interval_s,
                )
            };
            let num = |i: usize, d: f64| match args.get(i) {
                Some(Expr::Num(n)) => *n,
                Some(Expr::Neg(x)) => match x.as_ref() {
                    Expr::Num(n) => -n,
                    _ => d,
                },
                _ => d,
            };
            let text = |i: usize, d: &str| match args.get(i) {
                Some(Expr::Str(s)) => s.clone(),
                _ => d.to_string(),
            };
            match f.as_str() {
                "abs" => map(arg(0)?, |v| v.map(f64::abs)),
                "log2" => map(arg(0)?, |v| v.filter(|v| *v > 0.0).map(f64::log2)),
                "log10" => map(arg(0)?, |v| v.filter(|v| *v > 0.0).map(f64::log10)),
                "sqrt" => map(arg(0)?, |v| v.filter(|v| *v >= 0.0).map(f64::sqrt)),
                "default_zero" => map(arg(0)?, |v| v.or(Some(0.0))),
                "clamp_min" => {
                    let m = num(1, f64::NEG_INFINITY);
                    map(arg(0)?, move |v| v.map(|v| v.max(m)))
                }
                "clamp_max" => {
                    let m = num(1, f64::INFINITY);
                    map(arg(0)?, move |v| v.map(|v| v.min(m)))
                }
                "per_second" => map(arg(0)?, move |v| v.map(|v| v / interval_s)),
                "per_minute" => map(arg(0)?, move |v| v.map(|v| v / interval_s * 60.0)),
                "per_hour" => map(arg(0)?, move |v| v.map(|v| v / interval_s * 3600.0)),
                "cumsum" => arg(0)?
                    .into_iter()
                    .map(|s| {
                        let mut acc = 0.0;
                        Series {
                            tags: s.tags,
                            values: s
                                .values
                                .into_iter()
                                .map(|v| {
                                    v.map(|v| {
                                        acc += v;
                                        acc
                                    })
                                })
                                .collect(),
                        }
                    })
                    .collect(),
                "diff" | "derivative" => {
                    let per = if f == "derivative" { interval_s } else { 1.0 };
                    arg(0)?
                        .into_iter()
                        .map(|s| {
                            let mut prev: Option<f64> = None;
                            Series {
                                tags: s.tags,
                                values: s
                                    .values
                                    .into_iter()
                                    .map(|v| {
                                        let d = match (v, prev) {
                                            (Some(v), Some(p)) => Some((v - p) / per),
                                            _ => None,
                                        };
                                        if v.is_some() {
                                            prev = v;
                                        }
                                        d
                                    })
                                    .collect(),
                            }
                        })
                        .collect()
                }
                "timeshift" | "hour_before" | "day_before" | "week_before" | "month_before" => {
                    let secs = match f.as_str() {
                        "hour_before" => -3600,
                        "day_before" => -86_400,
                        "week_before" => -604_800,
                        "month_before" => -2_592_000,
                        _ => num(1, 0.0) as i64,
                    };
                    match args.first() {
                        Some(Expr::Ref(r)) => data
                            .get(&(r.clone(), secs))
                            .cloned()
                            .ok_or(format!("unknown query `{r}`"))?,
                        _ => return Err(format!("{f}() applies to a query name")),
                    }
                }
                "top" | "bottom" => {
                    let n = num(1, 10.0).max(1.0) as usize;
                    let how = text(2, "mean");
                    let desc = if f == "bottom" {
                        false
                    } else {
                        text(3, "desc") != "asc"
                    };
                    let mut s = arg(0)?;
                    let how = if how == "mean" { "avg" } else { how.as_str() }.to_string();
                    s.sort_by(|a, b| {
                        let (x, y) = (
                            reduce(&a.values, &how).unwrap_or(f64::NEG_INFINITY),
                            reduce(&b.values, &how).unwrap_or(f64::NEG_INFINITY),
                        );
                        if desc {
                            y.total_cmp(&x)
                        } else {
                            x.total_cmp(&y)
                        }
                    });
                    s.truncate(n);
                    s
                }
                other => return Err(format!("unknown function {other}()")),
            }
        }
    })
}

pub struct Output {
    pub times: Vec<i64>,
    pub interval: i64,
    /// (output index, name, series)
    pub series: Vec<(usize, String, Series)>,
}

pub async fn execute(st: &AppState, req: &Request, scalar: bool) -> Result<Output, ApiError> {
    if req.queries.is_empty() || req.queries.len() > 20 {
        return Err(ApiError::BadQuery("1 to 20 queries".into()));
    }
    let (from, to) = (req.from.min(req.to), req.to.max(req.from));
    let interval = req
        .interval
        .filter(|i| *i > 0)
        .unwrap_or_else(|| super::interval_ms(from, to, 150))
        .max(FLUSH_INTERVAL_MS);
    let start = from.div_euclid(interval) * interval;
    let grid = Grid {
        from: start,
        interval,
        len: (((to - start) / interval) + 1).clamp(1, 5000) as usize,
    };
    let names: Vec<String> = req
        .queries
        .iter()
        .enumerate()
        .map(|(i, q)| {
            if q.name.is_empty() {
                format!("query{}", i + 1)
            } else {
                q.name.clone()
            }
        })
        .collect();
    let formulas: Vec<(String, Expr, &FormulaDef)> = req
        .formulas
        .iter()
        .map(|f| parse_formula(&f.formula).map(|e| (f.formula.clone(), e, f)))
        .collect::<Result<_, _>>()
        .map_err(ApiError::BadQuery)?;
    // Every (query, shift) a formula needs, plus each query as is.
    let mut needed: Vec<(String, i64)> = names.iter().map(|n| (n.clone(), 0)).collect();
    for (_, e, _) in &formulas {
        let mut s = Vec::new();
        shifts(e, &mut s);
        needed.extend(s);
        let mut r = Vec::new();
        refs(e, &mut r);
        for name in r {
            if !names.contains(&name) {
                return Err(ApiError::BadQuery(format!(
                    "formula uses unknown query `{name}`"
                )));
            }
        }
    }
    needed.sort();
    needed.dedup();
    let runs =
        futures::future::join_all(
            needed.iter().map(|(name, shift)| {
                let q = &req.queries[names.iter().position(|n| n == name).unwrap()];
                let grid = Grid {
                    from: grid.from + shift * 1000,
                    interval: grid.interval,
                    len: grid.len,
                };
                async move {
                    run_query(st, q, from + shift * 1000, to + shift * 1000, &grid, scalar).await
                }
            }),
        )
        .await;
    let mut data: Data = HashMap::new();
    for (key, run) in needed.into_iter().zip(runs) {
        data.insert(key, run?);
    }
    let mut series = Vec::new();
    if formulas.is_empty() {
        for (i, n) in names.iter().enumerate() {
            for s in data.get(&(n.clone(), 0)).cloned().unwrap_or_default() {
                series.push((i, n.clone(), s));
            }
        }
    } else {
        for (i, (text, e, def)) in formulas.iter().enumerate() {
            let mut out = eval(
                e,
                &data,
                if scalar { 1 } else { grid.len },
                interval as f64 / 1000.0,
            )
            .map_err(ApiError::BadQuery)?;
            if let Some(l) = &def.limit {
                let desc = l.order.as_deref() != Some("asc");
                out.sort_by(|a, b| {
                    let (x, y) = (
                        reduce(&a.values, "avg").unwrap_or(f64::NEG_INFINITY),
                        reduce(&b.values, "avg").unwrap_or(f64::NEG_INFINITY),
                    );
                    if desc {
                        y.total_cmp(&x)
                    } else {
                        x.total_cmp(&y)
                    }
                });
                out.truncate(l.count.unwrap_or(10));
            }
            let name = def.alias.clone().unwrap_or_else(|| text.clone());
            for s in out {
                series.push((i, name.clone(), s));
            }
        }
    }
    Ok(Output {
        times: grid.times(),
        interval,
        series,
    })
}

fn group_tags(t: &Tags) -> Vec<String> {
    t.iter().map(|(k, v)| format!("{k}:{v}")).collect()
}

/// Datadog wraps requests in `data.attributes`; the UI posts them bare.
fn unwrap(body: Value) -> Result<Request, ApiError> {
    let inner = body.pointer("/data/attributes").cloned().unwrap_or(body);
    serde_json::from_value(inner).map_err(|e| ApiError::BadQuery(e.to_string()))
}

pub async fn timeseries(
    State(st): State<AppState>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    let req = unwrap(body)?;
    let out = execute(&st, &req, false).await?;
    Ok(Json(json!({
        "data": {
            "type": "timeseries_response",
            "attributes": {
                "times": out.times,
                "interval": out.interval,
                "series": out.series.iter().map(|(i, name, s)| json!({
                    "query_index": i,
                    "name": name,
                    "group_tags": group_tags(&s.tags),
                    "unit": null,
                })).collect::<Vec<_>>(),
                "values": out.series.iter().map(|(_, _, s)| &s.values).collect::<Vec<_>>(),
            }
        }
    })))
}

/// One value per series, as columns: each query is reduced first (events over
/// the whole range, metrics by their `aggregator`, avg by default), then the
/// formulas apply.
pub async fn scalar(
    State(st): State<AppState>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    let req = unwrap(body)?;
    let out = execute(&st, &req, true).await?;
    let mut keys: Vec<String> = out
        .series
        .iter()
        .flat_map(|(_, _, s)| s.tags.keys().cloned())
        .collect();
    keys.sort();
    keys.dedup();
    let mut columns: Vec<Value> = keys
        .iter()
        .map(|k| {
            json!({
                "type": "group",
                "name": k,
                "values": out.series.iter().map(|(_, _, s)| vec![s.tags.get(k).cloned().unwrap_or_default()]).collect::<Vec<_>>(),
            })
        })
        .collect();
    let outputs: Vec<(usize, String)> = {
        let mut seen: Vec<(usize, String)> =
            out.series.iter().map(|(i, n, _)| (*i, n.clone())).collect();
        seen.dedup();
        seen
    };
    for (i, name) in &outputs {
        columns.push(json!({
            "type": "number",
            "name": name,
            "query_index": i,
            "values": out.series.iter().map(|(j, _, s)| if j == i { s.values.first().copied().flatten() } else { None }).collect::<Vec<_>>(),
            "meta": {"unit": null},
        }));
    }
    Ok(Json(
        json!({"data": {"type": "scalar_response", "attributes": {"columns": columns}}}),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn series(tags: &[(&str, &str)], v: &[f64]) -> Series {
        Series {
            tags: tags
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            values: v.iter().map(|x| Some(*x)).collect(),
        }
    }

    #[test]
    fn formulas() {
        let mut data: Data = HashMap::new();
        data.insert(
            ("a".into(), 0),
            vec![
                series(&[("host", "x")], &[10.0, 20.0]),
                series(&[("host", "y")], &[1.0, 3.0]),
            ],
        );
        data.insert(("b".into(), 0), vec![series(&[], &[2.0, 4.0])]);
        data.insert(
            ("a".into(), -3600),
            vec![series(&[("host", "x")], &[5.0, 5.0])],
        );
        let run = |f: &str| eval(&parse_formula(f).unwrap(), &data, 2, 10.0).unwrap();
        let r = run("a / b * 100");
        assert_eq!(r[0].values, vec![Some(500.0), Some(500.0)]);
        assert_eq!(r[1].values, vec![Some(50.0), Some(75.0)]);
        assert_eq!(run("-b + 1")[0].values, vec![Some(-1.0), Some(-3.0)]);
        assert_eq!(run("cumsum(b)")[0].values, vec![Some(2.0), Some(6.0)]);
        assert_eq!(run("diff(b)")[0].values, vec![None, Some(2.0)]);
        assert_eq!(run("per_second(b)")[0].values, vec![Some(0.2), Some(0.4)]);
        let top = run("top(a, 1, 'mean', 'desc')");
        assert_eq!(top.len(), 1);
        assert_eq!(top[0].tags["host"], "x");
        assert_eq!(
            run("a - timeshift(a, -3600)")[0].values,
            vec![Some(5.0), Some(15.0)]
        );
        assert_eq!(run("a - hour_before(a)").len(), 1);
        assert!(parse_formula("a +").is_err());
        assert!(eval(&parse_formula("nope(a)").unwrap(), &data, 2, 10.0).is_err());
        let mut s = Vec::new();
        shifts(
            &parse_formula("a - timeshift(a, -1h) + week_before(b)").unwrap(),
            &mut s,
        );
        assert_eq!(
            s,
            vec![("a".to_string(), -3600), ("b".to_string(), -604_800)]
        );
    }
}
