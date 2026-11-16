//! The search syntax evaluated in memory against one log, with the same
//! semantics as the SQL translation: what pipeline filters, category
//! processors and log-based metrics need at ingestion time.

use super::parser::{CmpOp, Cond, Expr, Value as Word};
use super::sql::parse_duration_ns;
use crate::intake::logs::LogRow;
use serde_json::Value;

/// `*` matches any run of characters; everything else literally.
pub fn glob(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    let (mut pi, mut ti) = (0, 0);
    let (mut star, mut mark) = (None, 0);
    while ti < t.len() {
        if pi < p.len() && p[pi] != '*' && p[pi] == t[ti] {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ti;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

fn eq(w: &Word, text: &str) -> bool {
    if w.wildcard {
        glob(&w.text, text)
    } else {
        w.text == text
    }
}

fn number(v: &str) -> Option<f64> {
    v.trim().parse::<f64>().ok().or_else(|| parse_duration_ns(v))
}

fn cmp(op: CmpOp, x: f64, y: f64) -> bool {
    match op {
        CmpOp::Gt => x > y,
        CmpOp::Ge => x >= y,
        CmpOp::Lt => x < y,
        CmpOp::Le => x <= y,
    }
}

/// The scalar values a field holds (arrays hold several).
fn scalars(v: &Value) -> Vec<String> {
    match v {
        Value::Null => vec![],
        Value::String(s) => vec![s.clone()],
        Value::Array(a) => a.iter().flat_map(scalars).collect(),
        Value::Object(_) => vec![v.to_string()],
        other => vec![other.to_string()],
    }
}

fn cond(c: &Cond, values: &[String]) -> bool {
    match c {
        Cond::Exists => !values.is_empty(),
        Cond::Eq(w) => values.iter().any(|v| eq(w, v)),
        Cond::Cmp(op, n) => {
            let Some(n) = number(n) else { return false };
            values
                .iter()
                .filter_map(|v| number(v))
                .any(|x| cmp(*op, x, n))
        }
        Cond::Range {
            lo,
            hi,
            lo_inc,
            hi_inc,
        } => {
            let bound = |s: &str| if s == "*" { None } else { number(s) };
            let (lo, hi) = (bound(lo), bound(hi));
            values.iter().filter_map(|v| number(v)).any(|x| {
                lo.is_none_or(|l| if *lo_inc { x >= l } else { x > l })
                    && hi.is_none_or(|h| if *hi_inc { x <= h } else { x < h })
            })
        }
    }
}

/// The attribute at a dotted path; a literal dotted key wins at each level.
pub fn attribute<'a>(attrs: &'a serde_json::Map<String, Value>, path: &str) -> Option<&'a Value> {
    if let Some(v) = attrs.get(path) {
        return Some(v);
    }
    let mut parts = path.splitn(2, '.');
    let head = parts.next()?;
    let rest = parts.next()?;
    match attrs.get(head)? {
        Value::Object(inner) => attribute(inner, rest),
        _ => None,
    }
}

fn field_values(log: &LogRow, key: &str) -> Vec<String> {
    let opt = |v: &Option<String>| v.iter().cloned().collect::<Vec<_>>();
    match key {
        "service" => opt(&log.service),
        "host" => opt(&log.host),
        "source" => opt(&log.source),
        "status" => vec![log.status.clone()],
        "trace_id" => opt(&log.trace_id),
        "message" => vec![log.message.clone()],
        k if k.starts_with('@') => attribute(&log.attributes, &k[1..])
            .map(scalars)
            .unwrap_or_default(),
        k => log
            .tags
            .iter()
            .filter_map(|t| {
                let (tk, tv) = t.split_once(':').unwrap_or((t.as_str(), ""));
                (tk == k).then(|| tv.to_string())
            })
            .collect(),
    }
}

/// Full-text terms match words of the message, case-insensitively.
fn text(w: &Word, log: &LogRow) -> bool {
    let needle = w.text.to_lowercase();
    let hay = log.message.to_lowercase();
    if w.wildcard {
        hay.split(|c: char| !c.is_alphanumeric() && c != '*' && c != '_' && c != '-')
            .any(|tok| glob(&needle, tok))
            || glob(&format!("*{needle}*"), &hay)
    } else if needle.contains(char::is_whitespace) {
        hay.contains(&needle)
    } else {
        hay.split(|c: char| !c.is_alphanumeric() && c != '_')
            .any(|tok| tok == needle)
            || (!needle.chars().all(|c| c.is_alphanumeric() || c == '_') && hay.contains(&needle))
    }
}

pub fn matches(e: &Expr, log: &LogRow) -> bool {
    match e {
        Expr::All => true,
        Expr::And(xs) => xs.iter().all(|x| matches(x, log)),
        Expr::Or(xs) => xs.iter().any(|x| matches(x, log)),
        Expr::Not(x) => !matches(x, log),
        Expr::Text(w) => text(w, log),
        Expr::Field { key, cond: c } => cond(c, &field_values(log, key)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query::parser::parse;
    use serde_json::json;

    fn log() -> LogRow {
        LogRow {
            ts: 0,
            status: "error".into(),
            service: Some("payment-service".into()),
            host: Some("h1".into()),
            source: Some("java".into()),
            message: "Payment declined for user u-42: card_expired".into(),
            tags: vec!["env:prod".into(), "team:pay".into()],
            attributes: json!({"http": {"status_code": 502, "method": "POST"}, "usr.id": "u-42", "items": ["a", "b"]})
                .as_object()
                .unwrap()
                .clone(),
            trace_id: None,
            span_id: None,
        }
    }

    fn m(q: &str) -> bool {
        matches(&parse(q).unwrap(), &log())
    }

    #[test]
    fn evaluates_like_the_sql() {
        assert!(m("service:payment-service status:error"));
        assert!(m("service:payment*"));
        assert!(!m("service:web-store"));
        assert!(m("env:prod -env:dev"));
        assert!(m("@http.status_code:>=500 @http.status_code:[500 TO 599]"));
        assert!(!m("@http.status_code:<500"));
        assert!(m("@http.method:POST @usr.id:u-42 @items:b"));
        assert!(m("@http.method:*"));
        assert!(!m("@nope:*"));
        assert!(m("declined"));
        assert!(m("DECLINED card_exp*"));
        assert!(!m("refused"));
        assert!(m("(service:a OR team:pay) AND NOT host:h2"));
        assert!(m("\"declined for user\""));
    }

    #[test]
    fn globs() {
        assert!(glob("a*c", "abbbc"));
        assert!(glob("*", ""));
        assert!(!glob("a*c", "abd"));
        assert!(glob("*card*", "x card y"));
    }
}
