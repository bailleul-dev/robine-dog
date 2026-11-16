//! Compiles a search [`Expr`] into a GreptimeDB `WHERE` fragment.
//!
//! Every value from the query is embedded through [`lit`] / [`json_path`],
//! which quote it; numbers are re-printed from their parsed `f64`.

use super::parser::{CmpOp, Cond, Expr, Value};
use crate::store::greptime::{json_path, lit, num};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Dataset {
    Logs,
    Spans,
    /// Metric tag filters (`env:prod,!host:a`): every key is a tag.
    Metrics,
}

/// Where a field lives in the schema.
#[derive(Debug, Clone)]
pub enum Target {
    /// A plain text column.
    Column(&'static str),
    /// A path inside a nested JSON column (logs attributes).
    Json {
        column: &'static str,
        path: Vec<String>,
    },
    /// A span attribute: a flat key of `meta`, else of `metrics`.
    SpanAttr(String),
    /// A `key:value` entry of the `tags` column (`,env:prod,team:a,`).
    Tag(String),
    /// Span duration in nanoseconds; values accept units (`1.5s`, `200ms`).
    Duration,
    /// Span status, stored as the `error` flag.
    SpanStatus,
    /// A boolean column (`_entry:1` → `top_level`).
    Flag(&'static str),
    /// Root spans (`_root:1`): no parent.
    Root,
}

pub fn resolve(key: &str, ds: Dataset) -> Target {
    match ds {
        Dataset::Logs => match key {
            "service" => Target::Column("service"),
            "host" => Target::Column("host"),
            "source" => Target::Column("source"),
            "status" => Target::Column("status"),
            "trace_id" => Target::Column("trace_id"),
            "message" => Target::Column("message"),
            k if k.starts_with('@') => Target::Json {
                column: "attributes",
                path: k[1..].split('.').map(str::to_string).collect(),
            },
            k => Target::Tag(k.to_string()),
        },
        Dataset::Spans => match key {
            "service" => Target::Column("service"),
            "resource_name" | "resource" => Target::Column("resource"),
            "operation_name" | "name" => Target::Column("name"),
            "env" => Target::Column("env"),
            "version" => Target::Column("version"),
            "trace_id" => Target::Column("trace_id"),
            "span_id" => Target::Column("span_id"),
            "parent_id" => Target::Column("parent_id"),
            "type" | "span.type" => Target::Column("type"),
            "status" => Target::SpanStatus,
            // Internal keys behind the "Service entry spans / Root spans" selector.
            "_entry" => Target::Flag("top_level"),
            "_root" => Target::Root,
            "@duration" | "duration" => Target::Duration,
            k => Target::SpanAttr(k.strip_prefix('@').unwrap_or(k).to_string()),
        },
        Dataset::Metrics => match key {
            "host" => Target::Column("host"),
            k => Target::Tag(k.to_string()),
        },
    }
}

/// SQL expression reading a target's value as text (not for tags).
pub fn target_expr(t: &Target) -> String {
    match t {
        Target::Column(c) => (*c).to_string(),
        Target::Json { column, path } => {
            format!(
                "json_get({column}, {})",
                json_path(path.iter().map(String::as_str))
            )
        }
        Target::SpanAttr(k) => format!(
            "COALESCE(json_get(meta, {p}), json_get(metrics, {p}))",
            p = json_path([k.as_str()])
        ),
        Target::Duration => "CAST(duration AS STRING)".to_string(),
        Target::SpanStatus => "(CASE WHEN error THEN 'error' ELSE 'ok' END)".to_string(),
        Target::Flag(c) => format!("(CASE WHEN {c} THEN '1' ELSE '0' END)"),
        Target::Root => "(CASE WHEN parent_id = '0' THEN '1' ELSE '0' END)".to_string(),
        Target::Tag(_) => unreachable!("tags have no scalar expression"),
    }
}

/// SQL expression reading a target's value as a number (NULL when it isn't one).
pub fn number_expr(t: &Target) -> String {
    match t {
        Target::Json { column, path } => format!(
            "json_get_float({column}, {})",
            json_path(path.iter().map(String::as_str))
        ),
        Target::SpanAttr(k) => format!(
            "COALESCE(json_get_float(metrics, {p}), json_get_float(meta, {p}))",
            p = json_path([k.as_str()])
        ),
        Target::Duration => "duration".to_string(),
        other => format!("TRY_CAST({} AS DOUBLE)", target_expr(other)),
    }
}

/// `1.5s` → 1_500_000_000 (ns). A bare number is nanoseconds.
pub fn parse_duration_ns(s: &str) -> Option<f64> {
    let s = s.trim();
    let idx = s
        .find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-'))
        .unwrap_or(s.len());
    let (num, unit) = s.split_at(idx);
    let n: f64 = num.parse().ok()?;
    let mult = match unit.trim() {
        "" | "ns" => 1.0,
        "us" | "µs" => 1e3,
        "ms" => 1e6,
        "s" => 1e9,
        "m" | "min" => 60e9,
        "h" => 3600e9,
        _ => return None,
    };
    Some(n * mult)
}

/// A `LIKE` pattern (escape character `\`): `*` is the wildcard, `%` and `_` are literal.
pub fn like_pattern(v: &str, contains: bool) -> String {
    let mut out = String::new();
    if contains {
        out.push('%');
    }
    for ch in v.chars() {
        match ch {
            '%' | '_' | '\\' => {
                out.push('\\');
                out.push(ch);
            }
            '*' => out.push('%'),
            c => out.push(c),
        }
    }
    if contains {
        out.push('%');
    }
    out
}

/// A regular expression matching `v` literally, with `*` as "anything but a comma".
pub fn tag_regex(v: &str) -> String {
    let mut out = String::new();
    for ch in v.chars() {
        match ch {
            '*' => out.push_str("[^,]*"),
            c if "\\.+?()[]{}|^$#&-~".contains(c) => {
                out.push('\\');
                out.push(c);
            }
            c => out.push(c),
        }
    }
    out
}

fn normalize_status(v: &str) -> String {
    crate::intake::logs::normalize_status(v).to_string()
}

fn cmp_sql(op: CmpOp) -> &'static str {
    match op {
        CmpOp::Gt => ">",
        CmpOp::Ge => ">=",
        CmpOp::Lt => "<",
        CmpOp::Le => "<=",
    }
}

/// Free text is matched as whole terms, case-insensitively, as Datadog does;
/// `*` turns a term into a substring pattern.
fn free_text(column: &str, v: &Value) -> String {
    if v.wildcard || v.text.chars().any(|c| c == '%' || c == '_') {
        format!(
            "lower({column}) LIKE {} ESCAPE '\\'",
            lit(&like_pattern(&v.text.to_lowercase(), true))
        )
    } else {
        format!(
            "matches_term(lower({column}), {})",
            lit(&v.text.to_lowercase())
        )
    }
}

struct Compiler {
    ds: Dataset,
}

impl Compiler {
    fn expr(&self, e: &Expr) -> String {
        match e {
            Expr::All => "true".into(),
            Expr::And(items) => format!(
                "({})",
                items
                    .iter()
                    .map(|i| self.expr(i))
                    .collect::<Vec<_>>()
                    .join(" AND ")
            ),
            Expr::Or(items) => format!(
                "({})",
                items
                    .iter()
                    .map(|i| self.expr(i))
                    .collect::<Vec<_>>()
                    .join(" OR ")
            ),
            Expr::Not(inner) => format!("NOT COALESCE({}, false)", self.expr(inner)),
            Expr::Text(v) => self.text(v),
            Expr::Field { key, cond } => self.field(&resolve(key, self.ds), cond),
        }
    }

    fn text(&self, v: &Value) -> String {
        match self.ds {
            Dataset::Logs => free_text("message", v),
            Dataset::Spans => {
                let pat = lit(&like_pattern(&v.text.to_lowercase(), true));
                format!(
                    "(lower(resource) LIKE {pat} ESCAPE '\\' OR lower(name) LIKE {pat} ESCAPE '\\')"
                )
            }
            // A bare word in a metric filter is a valueless tag (`#mytag`).
            Dataset::Metrics => self.tag_eq(&v.text, v.wildcard),
        }
    }

    fn number(&self, target: &Target, raw: &str) -> Option<f64> {
        match target {
            Target::Duration => parse_duration_ns(raw),
            _ => raw.parse().ok(),
        }
    }

    fn field(&self, target: &Target, cond: &Cond) -> String {
        if let Target::Tag(key) = target {
            return self.tag(key, cond);
        }
        let mut cond = cond.clone();
        if let (Target::Column("status"), Cond::Eq(v), Dataset::Logs) = (target, &cond, self.ds)
            && !v.wildcard
        {
            cond = Cond::Eq(Value {
                text: normalize_status(&v.text),
                wildcard: false,
            });
        }
        match &cond {
            Cond::Exists => format!("{} IS NOT NULL", target_expr(target)),
            Cond::Eq(v) if v.wildcard => format!(
                "{} LIKE {} ESCAPE '\\'",
                target_expr(target),
                lit(&like_pattern(&v.text, false))
            ),
            Cond::Eq(v) => match target {
                Target::Duration => {
                    format!(
                        "duration = {}",
                        num(parse_duration_ns(&v.text).unwrap_or(-1.0))
                    )
                }
                Target::Flag(c) => {
                    let on = matches!(v.text.as_str(), "1" | "true");
                    format!("{c} = {on}")
                }
                Target::Root => {
                    let on = matches!(v.text.as_str(), "1" | "true");
                    format!("(parent_id = '0') = {on}")
                }
                Target::Column(_) | Target::SpanStatus => {
                    format!("{} = {}", target_expr(target), lit(&v.text))
                }
                _ => {
                    // JSON values may be numbers or strings ("500" in span meta, 500 in metrics).
                    let text = format!("{} = {}", target_expr(target), lit(&v.text));
                    match v.text.parse::<f64>() {
                        Ok(n) => format!("({text} OR {} = {})", number_expr(target), num(n)),
                        Err(_) => text,
                    }
                }
            },
            Cond::Cmp(op, raw) => match self.number(target, raw) {
                Some(n) => format!("{} {} {}", number_expr(target), cmp_sql(*op), num(n)),
                None => format!("{} {} {}", target_expr(target), cmp_sql(*op), lit(raw)),
            },
            Cond::Range {
                lo,
                hi,
                lo_inc,
                hi_inc,
            } => {
                let lo_op = if *lo_inc { ">=" } else { ">" };
                let hi_op = if *hi_inc { "<=" } else { "<" };
                match (self.number(target, lo), self.number(target, hi)) {
                    (Some(a), Some(b)) => {
                        let e = number_expr(target);
                        format!("({e} {lo_op} {} AND {e} {hi_op} {})", num(a), num(b))
                    }
                    _ => {
                        let e = target_expr(target);
                        format!("({e} {lo_op} {} AND {e} {hi_op} {})", lit(lo), lit(hi))
                    }
                }
            }
        }
    }

    fn tag_eq(&self, entry: &str, wildcard: bool) -> String {
        if wildcard {
            format!(
                "regexp_like(tags, {})",
                lit(&format!(",{},", tag_regex(entry)))
            )
        } else {
            format!(
                "tags LIKE {} ESCAPE '\\'",
                lit(&format!("%,{},%", like_pattern(entry, false)))
            )
        }
    }

    /// The first value of tag `key` as a number.
    fn tag_number(key: &str) -> String {
        format!(
            "TRY_CAST(array_element(regexp_match(tags, {}), 1) AS DOUBLE)",
            lit(&format!(",{}:([^,]*),", tag_regex(key)))
        )
    }

    fn tag(&self, key: &str, cond: &Cond) -> String {
        match cond {
            Cond::Exists => format!(
                "tags LIKE {} ESCAPE '\\'",
                lit(&format!("%,{}:%", like_pattern(key, false)))
            ),
            Cond::Eq(v) => self.tag_eq(&format!("{key}:{}", v.text), v.wildcard),
            Cond::Cmp(op, raw) => match raw.parse::<f64>() {
                Ok(n) => format!("{} {} {}", Self::tag_number(key), cmp_sql(*op), num(n)),
                Err(_) => "false".into(),
            },
            Cond::Range {
                lo,
                hi,
                lo_inc,
                hi_inc,
            } => {
                let lo_op = if *lo_inc { ">=" } else { ">" };
                let hi_op = if *hi_inc { "<=" } else { "<" };
                let e = Self::tag_number(key);
                format!(
                    "({e} {lo_op} {} AND {e} {hi_op} {})",
                    num(lo.parse().unwrap_or(f64::MIN)),
                    num(hi.parse().unwrap_or(f64::MAX))
                )
            }
        }
    }
}

pub fn compile(e: &Expr, ds: Dataset) -> String {
    Compiler { ds }.expr(e)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query::parser::parse;

    fn sql(q: &str, ds: Dataset) -> String {
        compile(&parse(q).unwrap(), ds)
    }

    #[test]
    fn durations() {
        assert_eq!(parse_duration_ns("1.5s"), Some(1.5e9));
        assert_eq!(parse_duration_ns("200ms"), Some(2e8));
        assert_eq!(parse_duration_ns("10"), Some(10.0));
        assert_eq!(parse_duration_ns("3 parsecs"), None);
    }

    #[test]
    fn columns_attributes_and_text() {
        assert_eq!(sql("service:web", Dataset::Logs), "service = 'web'");
        assert_eq!(sql("status:ERROR", Dataset::Logs), "status = 'error'");
        assert_eq!(
            sql("@http.status_code:>=500", Dataset::Logs),
            r#"json_get_float(attributes, '$["http"]["status_code"]') >= 500.0"#
        );
        assert_eq!(
            sql("timeout", Dataset::Logs),
            "matches_term(lower(message), 'timeout')"
        );
        assert_eq!(
            sql("time*", Dataset::Logs),
            r"lower(message) LIKE '%time%%' ESCAPE '\'"
        );
        assert_eq!(
            sql("@peer.service:redis", Dataset::Spans),
            r#"COALESCE(json_get(meta, '$["peer.service"]'), json_get(metrics, '$["peer.service"]')) = 'redis'"#
        );
        assert_eq!(sql("_entry:1", Dataset::Spans), "top_level = true");
    }

    #[test]
    fn tags() {
        assert_eq!(
            sql("env:prod", Dataset::Logs),
            r"tags LIKE '%,env:prod,%' ESCAPE '\'"
        );
        assert_eq!(
            sql("env:*", Dataset::Logs),
            r"tags LIKE '%,env:%' ESCAPE '\'"
        );
        assert_eq!(
            sql("env:pr*d", Dataset::Logs),
            "regexp_like(tags, ',env:pr[^,]*d,')"
        );
        assert_eq!(
            sql("team:a_b", Dataset::Metrics),
            r"tags LIKE '%,team:a\_b,%' ESCAPE '\'"
        );
        assert_eq!(sql("host:h1", Dataset::Metrics), "host = 'h1'");
    }

    #[test]
    fn quotes_never_escape() {
        let s = sql(r#"service:"x' OR 1=1 --""#, Dataset::Logs);
        assert_eq!(s, "service = 'x'' OR 1=1 --'");
        let s = sql(r#"@a\.b:"it's""#, Dataset::Logs);
        assert!(s.contains("'it''s'"), "{s}");
    }

    #[test]
    fn negation_and_numbers() {
        assert_eq!(
            sql("-service:a @n:[1 TO 5]", Dataset::Logs),
            r#"(NOT COALESCE(service = 'a', false) AND (json_get_float(attributes, '$["n"]') >= 1.0 AND json_get_float(attributes, '$["n"]') <= 5.0))"#
        );
        assert_eq!(
            sql("@duration:>1.5s", Dataset::Spans),
            "duration > 1500000000.0"
        );
    }
}
