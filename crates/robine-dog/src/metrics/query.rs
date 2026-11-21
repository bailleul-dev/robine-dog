//! Datadog metric queries:
//! `<aggregator>:<metric>{<filter>} by {<tag>,…}[.as_count()|.as_rate()][.rollup(<method>[, <seconds>])]`.
//!
//! Evaluated as Datadog does: each series is first aggregated over time into
//! the rollup interval (sum for counts, average otherwise), then the series
//! are aggregated together (space aggregation) per `by` group. Distributions
//! skip the first step: percentiles are computed over all the samples.

use super::Kind;
use crate::query::parser;
use crate::query::sql::{Dataset, compile, tag_regex};
use crate::store::greptime::lit;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Agg {
    Avg,
    Sum,
    Min,
    Max,
    Count,
    Pct(f64),
}

impl Agg {
    fn parse(s: &str) -> Option<Agg> {
        Some(match s {
            "avg" => Agg::Avg,
            "sum" => Agg::Sum,
            "min" => Agg::Min,
            "max" => Agg::Max,
            "count" => Agg::Count,
            "p50" | "median" => Agg::Pct(0.5),
            "p75" => Agg::Pct(0.75),
            "p90" => Agg::Pct(0.9),
            "p95" => Agg::Pct(0.95),
            "p99" => Agg::Pct(0.99),
            _ => return None,
        })
    }

    /// Over distribution samples, each standing for `weight` samples.
    fn weighted_sql(self) -> String {
        match self {
            Agg::Avg => "sum(value * weight) / sum(weight)".into(),
            Agg::Sum => "sum(value * weight)".into(),
            Agg::Min => "min(value)".into(),
            Agg::Max => "max(value)".into(),
            Agg::Count => "sum(weight)".into(),
            Agg::Pct(p) => format!("approx_percentile_cont_with_weight(value, weight, {p})"),
        }
    }

    fn sql(self, col: &str) -> String {
        match self {
            Agg::Avg => format!("avg({col})"),
            Agg::Sum => format!("sum({col})"),
            Agg::Min => format!("min({col})"),
            Agg::Max => format!("max({col})"),
            Agg::Count => format!("CAST(count({col}) AS DOUBLE)"),
            Agg::Pct(p) => format!("approx_percentile_cont({col}, {p})"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum As {
    Default,
    Count,
    Rate,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MetricQuery {
    pub space: Agg,
    pub metric: String,
    /// The filter, in the logs search syntax (`env:prod AND -host:a`).
    pub filter: String,
    pub by: Vec<String>,
    pub as_: As,
    pub rollup: Option<(Agg, Option<i64>)>,
    /// `.fill(null|zero|linear|last[, max gap in seconds])`.
    pub fill: Option<(String, Option<i64>)>,
}

fn err<T>(m: impl Into<String>) -> Result<T, String> {
    Err(m.into())
}

/// `env:prod,!host:a` → `env:prod AND -host:a` (commas are Datadog's AND).
fn filter_to_search(f: &str) -> String {
    let f = f.trim();
    if f.is_empty() || f == "*" {
        return String::new();
    }
    f.split(',')
        .map(|t| {
            let t = t.trim();
            match t.strip_prefix('!') {
                Some(rest) => format!("-{rest}"),
                None => t.to_string(),
            }
        })
        .filter(|t| !t.is_empty())
        .collect::<Vec<_>>()
        .join(" AND ")
}

pub fn parse(q: &str) -> Result<MetricQuery, String> {
    let q = q.trim();
    let (space, rest) = match q.split_once(':') {
        Some((a, rest)) if !a.contains('{') && !a.contains('.') => match Agg::parse(a.trim()) {
            Some(agg) => (agg, rest),
            None => return err(format!("unknown aggregator `{a}`")),
        },
        _ => (Agg::Avg, q),
    };
    let brace = rest.find('{');
    let (metric, mut tail) = match brace {
        Some(i) => (rest[..i].trim(), &rest[i..]),
        None => {
            let end = rest
                .find(|c: char| c.is_whitespace() || c == '(')
                .unwrap_or(rest.len());
            // A function suffix (`.as_count()`) without a filter.
            let name_end = rest[..end]
                .find(".as_")
                .or_else(|| rest[..end].find(".rollup"))
                .unwrap_or(end);
            (rest[..name_end].trim(), &rest[name_end..])
        }
    };
    if metric.is_empty()
        || !metric
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '.' | '_' | '-'))
    {
        return err("expected a metric name");
    }
    let mut filter = String::new();
    if tail.starts_with('{') {
        let close = tail.find('}').ok_or("unclosed `{`")?;
        filter = filter_to_search(&tail[1..close]);
        tail = &tail[close + 1..];
    }
    let mut by = Vec::new();
    let mut as_ = As::Default;
    let mut rollup = None;
    let mut fill = None;
    loop {
        tail = tail.trim_start();
        if tail.is_empty() {
            break;
        }
        if let Some(r) = tail.strip_prefix("by") {
            let r = r.trim_start();
            let r = r.strip_prefix('{').ok_or("expected `{` after `by`")?;
            let close = r.find('}').ok_or("unclosed `by {`")?;
            by = r[..close]
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            tail = &r[close + 1..];
        } else if let Some(r) = tail.strip_prefix('.') {
            let open = r.find('(').ok_or("expected `(`")?;
            let close = r.find(')').ok_or("expected `)`")?;
            let (name, args) = (&r[..open], r[open + 1..close].trim());
            match name {
                "as_count" => as_ = As::Count,
                "as_rate" => as_ = As::Rate,
                "rollup" => {
                    let mut a = args.split(',').map(str::trim);
                    let method = a.next().unwrap_or("avg");
                    let agg = Agg::parse(method).ok_or(format!("unknown rollup `{method}`"))?;
                    let secs = match a.next() {
                        Some(s) => Some(
                            s.parse::<i64>()
                                .map_err(|_| "rollup seconds must be a number")?,
                        ),
                        None => None,
                    };
                    rollup = Some((agg, secs));
                }
                "fill" => {
                    let mut a = args.split(',').map(str::trim);
                    let mode = match a.next().unwrap_or("null") {
                        "" | "null" => "null",
                        "zero" | "0" => "zero",
                        "linear" => "linear",
                        "last" => "last",
                        other => return err(format!("unknown fill `{other}`")),
                    };
                    let limit = match a.next() {
                        Some(s) => Some(
                            s.parse::<i64>()
                                .map_err(|_| "fill limit must be a number of seconds")?,
                        ),
                        None => None,
                    };
                    fill = Some((mode.to_string(), limit));
                }
                other => return err(format!("unsupported function `.{other}()`")),
            }
            tail = &r[close + 1..];
        } else {
            return err(format!("unexpected `{tail}`"));
        }
    }
    parser::parse(&filter).map_err(|e| e.0)?;
    Ok(MetricQuery {
        space,
        metric: metric.to_string(),
        filter,
        by,
        as_,
        rollup,
        fill,
    })
}

/// Applies the query's `.fill()` to values on a grid of `interval_ms`.
pub fn apply_fill(q: &MetricQuery, values: &[Option<f64>], interval_ms: i64) -> Vec<Option<f64>> {
    match &q.fill {
        Some((mode, limit)) => {
            let limit =
                limit.map(|s| ((s * 1000) as f64 / interval_ms.max(1) as f64).ceil() as usize);
            super::functions::fill(values, mode, limit)
        }
        None => values.to_vec(),
    }
}

/// Splits a metric query wrapped in functions, as the Metrics Explorer and
/// classic graphs write them: `anomalies(ewma_5(avg:x{*} by {host}), 'basic', 2)`
/// → (`avg:x{*} by {host}`, `anomalies(ewma_5(q), 'basic', 2)`), the formula
/// reading the query as `q`. `None` for a bare query.
pub fn split_functions(q: &str) -> Option<(String, String)> {
    let q = q.trim();
    let open = q.find('(')?;
    let name = &q[..open];
    if name.is_empty()
        || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        || !q.ends_with(')')
    {
        return None;
    }
    let inner = &q[open + 1..q.len() - 1];
    // The first argument ends at the first comma outside parentheses,
    // braces and quotes; the function's parenthesis must close at the end.
    let (mut depth, mut quote, mut first_end) = (0i32, None::<char>, None);
    for (i, c) in inner.char_indices() {
        match (quote, c) {
            (Some(qc), c) if c == qc => quote = None,
            (Some(_), _) => {}
            (None, '\'' | '"') => quote = Some(c),
            (None, '(' | '{') => depth += 1,
            (None, ')' | '}') => {
                depth -= 1;
                if depth < 0 {
                    return None;
                }
            }
            (None, ',') if depth == 0 && first_end.is_none() => first_end = Some(i),
            _ => {}
        }
    }
    if depth != 0 {
        return None;
    }
    let (first, rest) = inner.split_at(first_end.unwrap_or(inner.len()));
    let (base, formula) = match split_functions(first) {
        Some(x) => x,
        None => (first.trim().to_string(), "q".to_string()),
    };
    Some((base, format!("{name}({formula}{rest})")))
}

/// SQL expression for one `by` key.
fn group_expr(key: &str) -> String {
    if key == "host" {
        "host".into()
    } else {
        format!(
            "array_element(regexp_match(tags, {}), 1)",
            lit(&format!(",{}:([^,]*),", tag_regex(key)))
        )
    }
}

/// The rollup interval: as asked, else about 150 points over the range, never
/// finer than the 10-second flush interval.
pub fn interval_ms(q: &MetricQuery, from: i64, to: i64) -> i64 {
    match q.rollup {
        Some((_, Some(secs))) if secs > 0 => (secs * 1000).max(super::FLUSH_INTERVAL_MS),
        _ => crate::api::interval_ms(from, to, 150).max(super::FLUSH_INTERVAL_MS),
    }
}

/// Builds the statement; its rows are `[bucket ms, group values…, value]`.
pub fn build_sql(q: &MetricQuery, kind: Kind, from: i64, to: i64, interval: i64) -> String {
    let filter = compile(
        &parser::parse(&q.filter).expect("validated"),
        Dataset::Metrics,
    );
    let groups: Vec<String> =
        q.by.iter()
            .enumerate()
            .map(|(i, k)| format!("{} AS g{i}", group_expr(k)))
            .collect();
    let gnames: Vec<String> = (0..q.by.len()).map(|i| format!("g{i}")).collect();
    let bucket = format!("CAST(date_bin(INTERVAL '{interval} milliseconds', ts) AS BIGINT) AS b");
    let where_ = format!(
        "metric = {} AND ts >= {from} AND ts <= {to} AND {filter}",
        lit(&q.metric)
    );
    let mut select: Vec<String> = vec![bucket];
    select.extend(groups);
    let group_by = std::iter::once("b".to_string())
        .chain(gnames.iter().cloned())
        .collect::<Vec<_>>()
        .join(", ");
    let secs = interval as f64 / 1000.0;

    if kind == Kind::Distribution {
        select.push(format!("{} AS v", q.space.weighted_sql()));
        return format!(
            "SELECT {} FROM metric_points WHERE {where_} GROUP BY {group_by} ORDER BY b",
            select.join(", ")
        );
    }

    let time_agg = match q.rollup {
        Some((a, _)) => a,
        None if kind == Kind::Count => Agg::Sum,
        None => Agg::Avg,
    };
    let scale = match (kind, q.as_) {
        (Kind::Count, As::Rate) => format!(" / {secs:?}"),
        (Kind::Rate, As::Count) => format!(" * {secs:?}"),
        _ => String::new(),
    };
    select.push(format!("{}{scale} AS v", time_agg.sql("value")));
    let space = match q.space {
        Agg::Pct(_) => Agg::Avg,
        a => a,
    };
    let outer: Vec<String> = std::iter::once("b".to_string())
        .chain(gnames.iter().cloned())
        .chain(std::iter::once(format!("{} AS v", space.sql("v"))))
        .collect();
    format!(
        "SELECT {} FROM (SELECT {}, host AS series_host, tags AS series_tags FROM metric_points WHERE {where_} \
         GROUP BY {group_by}, series_host, series_tags) GROUP BY {group_by} ORDER BY b",
        outer.join(", "),
        select.join(", ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_datadog_queries() {
        let q = parse(
            "sum:trace.http.request.hits{service:web,!env:dev} by {resource_name}.as_count()",
        )
        .unwrap();
        assert_eq!(q.space, Agg::Sum);
        assert_eq!(q.metric, "trace.http.request.hits");
        assert_eq!(q.filter, "service:web AND -env:dev");
        assert_eq!(q.by, vec!["resource_name"]);
        assert_eq!(q.as_, As::Count);

        let q = parse("system.load.1").unwrap();
        assert_eq!(
            (q.space, q.metric.as_str(), q.filter.as_str()),
            (Agg::Avg, "system.load.1", "")
        );

        let q = parse("p95:trace.http.request{*}.rollup(max, 60)").unwrap();
        assert_eq!(q.space, Agg::Pct(0.95));
        assert_eq!(q.rollup, Some((Agg::Max, Some(60))));

        let q = parse("max:x.y.as_rate()").unwrap();
        assert_eq!((q.metric.as_str(), q.as_), ("x.y", As::Rate));

        assert!(parse("bogus:metric{*}").is_err());
        assert!(parse("avg:metric{env:prod").is_err());
        assert!(parse("avg:metric{*}.nope()").is_err());
        assert!(parse("avg:bad name{*}").is_err());

        let q = parse("avg:x{*}.fill(linear, 300)").unwrap();
        assert_eq!(q.fill, Some(("linear".into(), Some(300))));
        assert_eq!(
            apply_fill(&q, &[Some(1.0), None, Some(3.0)], 10_000),
            vec![Some(1.0), Some(2.0), Some(3.0)]
        );
        assert!(parse("avg:x{*}.fill(sideways)").is_err());
    }

    #[test]
    fn splits_functions() {
        assert_eq!(split_functions("avg:x{a:b,c:d} by {host}"), None);
        assert_eq!(split_functions("avg:x{*}.rollup(max, 60)"), None);
        assert_eq!(
            split_functions("anomalies(ewma_5(avg:x{a:b,c:d} by {host,env}), 'basic', 2)"),
            Some((
                "avg:x{a:b,c:d} by {host,env}".to_string(),
                "anomalies(ewma_5(q), 'basic', 2)".to_string()
            ))
        );
        assert_eq!(
            split_functions("trend_line(sum:hits{*}.as_count())"),
            Some((
                "sum:hits{*}.as_count()".to_string(),
                "trend_line(q)".to_string()
            ))
        );
        assert_eq!(split_functions("f(a))"), None);
    }

    #[test]
    fn builds_two_level_aggregation() {
        let q = parse("sum:hits{env:prod} by {service}").unwrap();
        let sql = build_sql(&q, Kind::Count, 0, 60_000, 10_000);
        assert!(sql.contains("sum(value) AS v"), "{sql}");
        assert!(
            sql.starts_with("SELECT b, g0, sum(v) AS v FROM (SELECT"),
            "{sql}"
        );
        assert!(sql.contains(r"tags LIKE '%,env:prod,%'"), "{sql}");
        assert!(
            sql.contains("regexp_match(tags, ',service:([^,]*),')"),
            "{sql}"
        );

        let q = parse("p99:lat{*}").unwrap();
        let sql = build_sql(&q, Kind::Distribution, 0, 60_000, 10_000);
        assert!(
            sql.contains("approx_percentile_cont_with_weight(value, weight, 0.99)"),
            "{sql}"
        );

        let q = parse("sum:hits{*}.as_rate()").unwrap();
        assert!(build_sql(&q, Kind::Count, 0, 1, 10_000).contains("sum(value) / 10.0 AS v"));
    }
}
