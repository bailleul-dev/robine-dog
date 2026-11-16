//! Compiles a search [`Expr`] into a SQLite `WHERE` fragment.

use super::parser::{CmpOp, Cond, Expr, Value};
use rusqlite::types::Value as SqlValue;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Dataset {
    Logs,
    Spans,
}

#[derive(Debug, Default)]
pub struct Filter {
    pub sql: String,
    pub params: Vec<SqlValue>,
}

/// Where a field lives in the schema.
#[derive(Debug, Clone)]
pub enum Target {
    /// A plain text column.
    Column(&'static str),
    /// A path inside a JSON column, e.g. `json_extract(attributes, '$."http"."status_code"')`.
    Json { column: &'static str, path: String },
    /// A span attribute: looked up in `meta`, then in `metrics`.
    SpanAttr(String),
    /// A `key:value` tag in the `tags` JSON array.
    Tag(String),
    /// Span duration in nanoseconds; values accept units (`1.5s`, `200ms`).
    Duration,
    /// Span status, stored as the `error` flag.
    SpanStatus,
}

/// `a.b.c` → `$."a"."b"."c"` (nested, as logs attributes are stored).
pub fn nested_path(path: &str) -> String {
    let mut out = String::from("$");
    for seg in path.split('.') {
        out.push_str(".\"");
        out.push_str(&seg.replace('"', "\\\""));
        out.push('"');
    }
    out
}

/// `a.b.c` → `$."a.b.c"` (flat, as span meta keys are stored).
pub fn flat_path(path: &str) -> String {
    format!("$.\"{}\"", path.replace('"', "\\\""))
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
                path: nested_path(&k[1..]),
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
            "_entry" => Target::Column("top_level"),
            "_root" => Target::Column("CAST(parent_id = '0' AS TEXT)"),
            "@duration" | "duration" => Target::Duration,
            k => Target::SpanAttr(k.strip_prefix('@').unwrap_or(k).to_string()),
        },
    }
}

/// SQL expression reading a target's value (not usable for tags).
pub fn target_expr(t: &Target, params: &mut Vec<SqlValue>) -> String {
    match t {
        Target::Column(c) => (*c).to_string(),
        Target::Json { column, path } => {
            params.push(SqlValue::Text(path.clone()));
            format!("json_extract({column}, ?)")
        }
        Target::SpanAttr(k) => {
            params.push(SqlValue::Text(flat_path(k)));
            params.push(SqlValue::Text(flat_path(k)));
            "COALESCE(json_extract(meta, ?), json_extract(metrics, ?))".to_string()
        }
        Target::Duration => "duration".to_string(),
        Target::SpanStatus => "(CASE WHEN error THEN 'error' ELSE 'ok' END)".to_string(),
        Target::Tag(_) => unreachable!("tags have no scalar expression"),
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

struct Compiler {
    ds: Dataset,
    params: Vec<SqlValue>,
}

impl Compiler {
    fn expr(&mut self, e: &Expr) -> String {
        match e {
            Expr::All => "1".into(),
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
            Expr::Not(inner) => format!("NOT COALESCE({}, 0)", self.expr(inner)),
            Expr::Text(v) => self.text(v),
            Expr::Field { key, cond } => {
                let target = resolve(key, self.ds);
                self.field(&target, cond)
            }
        }
    }

    fn text(&mut self, v: &Value) -> String {
        let pat = like_pattern(&v.text, true);
        match self.ds {
            Dataset::Logs => {
                self.params.push(SqlValue::Text(pat));
                "message LIKE ? ESCAPE '\\'".into()
            }
            Dataset::Spans => {
                self.params.push(SqlValue::Text(pat.clone()));
                self.params.push(SqlValue::Text(pat));
                "(resource LIKE ? ESCAPE '\\' OR name LIKE ? ESCAPE '\\')".into()
            }
        }
    }

    fn number(&self, target: &Target, raw: &str) -> Option<f64> {
        match target {
            Target::Duration => parse_duration_ns(raw),
            _ => raw.parse().ok(),
        }
    }

    fn field(&mut self, target: &Target, cond: &Cond) -> String {
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
            Cond::Exists => {
                let lhs = target_expr(target, &mut self.params);
                format!("{lhs} IS NOT NULL")
            }
            Cond::Eq(v) if v.wildcard => {
                let lhs = target_expr(target, &mut self.params);
                self.params
                    .push(SqlValue::Text(like_pattern(&v.text, false)));
                format!("CAST({lhs} AS TEXT) LIKE ? ESCAPE '\\'")
            }
            Cond::Eq(v) => match target {
                Target::Duration => {
                    let n = parse_duration_ns(&v.text).unwrap_or(-1.0);
                    self.params.push(SqlValue::Real(n));
                    "duration = ?".into()
                }
                Target::Column(_) | Target::SpanStatus => {
                    let lhs = target_expr(target, &mut self.params);
                    self.params.push(SqlValue::Text(v.text.clone()));
                    format!("{lhs} = ?")
                }
                _ => {
                    // JSON values may be numbers or strings ("500" in span meta, 500 in metrics).
                    let lhs = target_expr(target, &mut self.params);
                    let mut lhs2_params = Vec::new();
                    let lhs2 = target_expr(target, &mut lhs2_params);
                    self.params.push(SqlValue::Text(v.text.clone()));
                    match v.text.parse::<f64>() {
                        Ok(n) => {
                            self.params.extend(lhs2_params);
                            self.params.push(SqlValue::Real(n));
                            format!("(CAST({lhs} AS TEXT) = ? OR {lhs2} = ?)")
                        }
                        Err(_) => {
                            let b = match v.text.as_str() {
                                "true" => Some(1),
                                "false" => Some(0),
                                _ => None,
                            };
                            match b {
                                Some(b) => {
                                    self.params.extend(lhs2_params);
                                    self.params.push(SqlValue::Integer(b));
                                    format!("(CAST({lhs} AS TEXT) = ? OR {lhs2} = ?)")
                                }
                                None => format!("CAST({lhs} AS TEXT) = ?"),
                            }
                        }
                    }
                }
            },
            Cond::Cmp(op, raw) => {
                let lhs = target_expr(target, &mut self.params);
                match self.number(target, raw) {
                    Some(n) => {
                        self.params.push(SqlValue::Real(n));
                        format!("CAST({lhs} AS REAL) {} ?", cmp_sql(*op))
                    }
                    None => {
                        self.params.push(SqlValue::Text(raw.clone()));
                        format!("CAST({lhs} AS TEXT) {} ?", cmp_sql(*op))
                    }
                }
            }
            Cond::Range {
                lo,
                hi,
                lo_inc,
                hi_inc,
            } => {
                let lhs = target_expr(target, &mut self.params);
                let mut lhs2_params = Vec::new();
                let lhs2 = target_expr(target, &mut lhs2_params);
                let lo_op = if *lo_inc { ">=" } else { ">" };
                let hi_op = if *hi_inc { "<=" } else { "<" };
                let (lo_v, hi_v, cast) = match (self.number(target, lo), self.number(target, hi)) {
                    (Some(a), Some(b)) => (SqlValue::Real(a), SqlValue::Real(b), "REAL"),
                    _ => (
                        SqlValue::Text(lo.clone()),
                        SqlValue::Text(hi.clone()),
                        "TEXT",
                    ),
                };
                self.params.push(lo_v);
                self.params.extend(lhs2_params);
                self.params.push(hi_v);
                format!("(CAST({lhs} AS {cast}) {lo_op} ? AND CAST({lhs2} AS {cast}) {hi_op} ?)")
            }
        }
    }

    fn tag(&mut self, key: &str, cond: &Cond) -> String {
        let prefix = format!("{key}:");
        match cond {
            Cond::Exists => {
                self.params
                    .push(SqlValue::Text(like_pattern(&prefix, false) + "%"));
                "EXISTS (SELECT 1 FROM json_each(tags) WHERE json_each.value LIKE ? ESCAPE '\\')"
                    .into()
            }
            Cond::Eq(v) if v.wildcard => {
                self.params.push(SqlValue::Text(like_pattern(
                    &format!("{prefix}{}", v.text),
                    false,
                )));
                "EXISTS (SELECT 1 FROM json_each(tags) WHERE json_each.value LIKE ? ESCAPE '\\')"
                    .into()
            }
            Cond::Eq(v) => {
                self.params
                    .push(SqlValue::Text(format!("{prefix}{}", v.text)));
                "EXISTS (SELECT 1 FROM json_each(tags) WHERE json_each.value = ?)".into()
            }
            Cond::Cmp(op, raw) => {
                self.params
                    .push(SqlValue::Text(like_pattern(&prefix, false) + "%"));
                self.params
                    .push(SqlValue::Integer(prefix.chars().count() as i64 + 1));
                self.params.push(
                    raw.parse::<f64>()
                        .map(SqlValue::Real)
                        .unwrap_or(SqlValue::Text(raw.clone())),
                );
                format!(
                    "EXISTS (SELECT 1 FROM json_each(tags) WHERE json_each.value LIKE ? ESCAPE '\\' AND CAST(substr(json_each.value, ?) AS REAL) {} ?)",
                    cmp_sql(*op)
                )
            }
            Cond::Range {
                lo,
                hi,
                lo_inc,
                hi_inc,
            } => {
                let lo_op = if *lo_inc { ">=" } else { ">" };
                let hi_op = if *hi_inc { "<=" } else { "<" };
                let start = prefix.chars().count() as i64 + 1;
                self.params
                    .push(SqlValue::Text(like_pattern(&prefix, false) + "%"));
                self.params.push(SqlValue::Integer(start));
                self.params
                    .push(SqlValue::Real(lo.parse().unwrap_or(f64::MIN)));
                self.params.push(SqlValue::Integer(start));
                self.params
                    .push(SqlValue::Real(hi.parse().unwrap_or(f64::MAX)));
                format!(
                    "EXISTS (SELECT 1 FROM json_each(tags) WHERE json_each.value LIKE ? ESCAPE '\\' AND CAST(substr(json_each.value, ?) AS REAL) {lo_op} ? AND CAST(substr(json_each.value, ?) AS REAL) {hi_op} ?)"
                )
            }
        }
    }
}

pub fn compile(e: &Expr, ds: Dataset) -> Filter {
    let mut c = Compiler {
        ds,
        params: Vec::new(),
    };
    let sql = c.expr(e);
    Filter {
        sql,
        params: c.params,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query::parser::parse;

    #[test]
    fn durations() {
        assert_eq!(parse_duration_ns("1.5s"), Some(1.5e9));
        assert_eq!(parse_duration_ns("200ms"), Some(2e8));
        assert_eq!(parse_duration_ns("10"), Some(10.0));
        assert_eq!(parse_duration_ns("3 parsecs"), None);
    }

    #[test]
    fn nested_and_flat_paths() {
        assert_eq!(
            nested_path("http.status_code"),
            "$.\"http\".\"status_code\""
        );
        assert_eq!(flat_path("http.status_code"), "$.\"http.status_code\"");
    }

    #[test]
    fn compiles_and_binds_every_param() {
        for (q, ds) in [
            (
                "service:web @http.status_code:>=500 -env:prod \"timeout\"",
                Dataset::Logs,
            ),
            (
                "@duration:[1ms TO 2s] status:error resource_name:GET*",
                Dataset::Spans,
            ),
            ("@http.status_code:500 env:*", Dataset::Spans),
            ("version:>2 retries:[1 TO 3]", Dataset::Logs),
        ] {
            let f = compile(&parse(q).unwrap(), ds);
            assert_eq!(f.sql.matches('?').count(), f.params.len(), "{q}: {}", f.sql);
        }
    }
}
