//! Assertions on what a test observed, with Datadog's operators:
//! `is`, `isNot`, `lessThan`, `moreThan`, `contains`, `matches`,
//! `validatesJSONPath`…

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Assertion {
    /// `statusCode`, `responseTime` (milliseconds), `body`, `header`,
    /// `certificate`, `tlsVersion`, `connection`, `recordSome`,
    /// `recordEvery`, `receivedMessage`, `grpcHealthcheckStatus`,
    /// `grpcMetadata`, `latency`, `packetLossPercentage`, `packetsReceived`.
    #[serde(rename = "type")]
    pub typ: String,
    pub operator: String,
    /// The header's name, the DNS record type, the latency statistic.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub property: Option<String>,
    #[serde(default)]
    pub target: Value,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

pub fn as_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn as_number(v: &Value) -> Option<f64> {
    v.as_f64()
        .or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
}

/// Whether a text satisfies an operator, or why the operator is unknown.
pub fn text_holds(operator: &str, actual: &str, target: &str) -> Result<bool, String> {
    Ok(match operator {
        "is" | "equals" => actual == target,
        "isNot" | "notEquals" => actual != target,
        "contains" => actual.contains(target),
        "doesNotContain" | "notContains" => !actual.contains(target),
        "matches" | "matchRegex" => regex::Regex::new(target)
            .map_err(|e| format!("bad regex: {e}"))?
            .is_match(actual),
        "doesNotMatch" | "notMatchRegex" => !regex::Regex::new(target)
            .map_err(|e| format!("bad regex: {e}"))?
            .is_match(actual),
        "startsWith" => actual.starts_with(target),
        "notStartsWith" => !actual.starts_with(target),
        "isEmpty" => actual.is_empty(),
        "notIsEmpty" | "isNotEmpty" => !actual.is_empty(),
        other => return Err(format!("unknown operator {other}")),
    })
}

/// An operator in words: `lessThan` → `be less than`.
pub fn op_words(op: &str) -> String {
    match op {
        "is" | "equals" => "be".into(),
        "isNot" | "notEquals" => "not be".into(),
        "contains" => "contain".into(),
        "doesNotContain" | "notContains" => "not contain".into(),
        "matches" | "matchRegex" => "match".into(),
        "doesNotMatch" | "notMatchRegex" => "not match".into(),
        "startsWith" => "start with".into(),
        "notStartsWith" => "not start with".into(),
        "lessThan" => "be less than".into(),
        "lessThanOrEqual" => "be at most".into(),
        "moreThan" => "be more than".into(),
        "moreThanOrEqual" => "be at least".into(),
        "isInMoreThan" => "be in more than".into(),
        "isInLessThan" => "be in less than".into(),
        "isEmpty" => "be empty".into(),
        "notIsEmpty" | "isNotEmpty" => "not be empty".into(),
        other => other.into(),
    }
}

/// Why a text assertion does not hold, or `None`.
pub fn check_text(a: &Assertion, what: &str, actual: Option<&str>) -> Option<String> {
    let target = as_text(&a.target);
    let Some(actual) = actual else {
        return Some(format!("{what} is missing"));
    };
    match text_holds(&a.operator, actual, &target) {
        Ok(true) => None,
        Ok(false) => Some(format!(
            "{what} is {actual:?}, should {} {target:?}",
            op_words(&a.operator)
        )),
        Err(e) => Some(format!("{what}: {e}")),
    }
}

pub fn number_holds(operator: &str, actual: f64, target: f64) -> Result<bool, String> {
    Ok(match operator {
        "is" => actual == target,
        "isNot" => actual != target,
        "lessThan" => actual < target,
        "lessThanOrEqual" => actual <= target,
        "moreThan" => actual > target,
        "moreThanOrEqual" => actual >= target,
        other => return Err(format!("unknown operator {other}")),
    })
}

pub fn check_number(a: &Assertion, what: &str, actual: f64) -> Option<String> {
    let Some(target) = as_number(&a.target) else {
        return Some(format!("{what}: the target is not a number"));
    };
    match number_holds(&a.operator, actual, target) {
        Ok(true) => None,
        Ok(false) => Some(format!(
            "{what} is {actual}, should {} {target}",
            op_words(&a.operator)
        )),
        Err(e) => Some(format!("{what}: {e}")),
    }
}

// --- JSONPath --------------------------------------------------------------------------

/// The values a JSONPath selects: `$.a.b`, `$['a']`, `$.a[0]`, `$.a[*].b`,
/// `$..b` (any depth).
pub fn json_path<'a>(doc: &'a Value, path: &str) -> Result<Vec<&'a Value>, String> {
    let p = path.trim();
    let mut rest = p.strip_prefix('$').unwrap_or(p);
    let mut current: Vec<&Value> = vec![doc];
    while !rest.is_empty() {
        if let Some(r) = rest.strip_prefix("..") {
            let (name, r2) = take_name(r);
            if name.is_empty() {
                return Err(format!("bad JSONPath {path}"));
            }
            let mut found = Vec::new();
            for v in current {
                descend(v, &name, &mut found);
            }
            current = found;
            rest = r2;
        } else if let Some(r) = rest.strip_prefix('.') {
            let (name, r2) = take_name(r);
            current = select(current, &name);
            rest = r2;
        } else if let Some(r) = rest.strip_prefix('[') {
            let end = r.find(']').ok_or_else(|| format!("bad JSONPath {path}"))?;
            let inner = r[..end].trim();
            rest = &r[end + 1..];
            if inner == "*" {
                current = select(current, "*");
            } else if let Ok(i) = inner.parse::<i64>() {
                current = current
                    .into_iter()
                    .filter_map(|v| {
                        let a = v.as_array()?;
                        let idx = if i < 0 { a.len() as i64 + i } else { i };
                        a.get(usize::try_from(idx).ok()?)
                    })
                    .collect();
            } else {
                let name = inner.trim_matches(|c| c == '\'' || c == '"');
                current = select(current, name);
            }
        } else {
            return Err(format!("bad JSONPath {path}"));
        }
    }
    Ok(current)
}

fn take_name(s: &str) -> (String, &str) {
    let end = s.find(['.', '[']).unwrap_or(s.len());
    (s[..end].to_string(), &s[end..])
}

fn select<'a>(vs: Vec<&'a Value>, name: &str) -> Vec<&'a Value> {
    vs.into_iter()
        .flat_map(|v| -> Vec<&Value> {
            match (name, v) {
                ("*", Value::Array(a)) => a.iter().collect(),
                ("*", Value::Object(o)) => o.values().collect(),
                (n, Value::Object(o)) => o.get(n).into_iter().collect(),
                _ => vec![],
            }
        })
        .collect()
}

fn descend<'a>(v: &'a Value, name: &str, out: &mut Vec<&'a Value>) {
    match v {
        Value::Object(o) => {
            if let Some(x) = o.get(name) {
                out.push(x);
            }
            for x in o.values() {
                descend(x, name, out);
            }
        }
        Value::Array(a) => a.iter().for_each(|x| descend(x, name, out)),
        _ => {}
    }
}

/// `{"jsonPath": "$.status", "operator": "is", "targetValue": "ok"}`.
fn check_json_path(target: &Value, body: &str) -> Option<String> {
    let path = target["jsonPath"].as_str().unwrap_or("$");
    let op = target["operator"].as_str().unwrap_or("is");
    let expected = &target["targetValue"];
    let doc: Value = match serde_json::from_str(body) {
        Ok(d) => d,
        Err(_) => return Some("body is not JSON".into()),
    };
    let found = match json_path(&doc, path) {
        Ok(f) => f,
        Err(e) => return Some(e),
    };
    if found.is_empty() {
        return Some(format!("{path} matches nothing"));
    }
    let holds = |v: &Value| match (as_number(v), as_number(expected)) {
        (Some(a), Some(t))
            if matches!(
                op,
                "lessThan" | "moreThan" | "lessThanOrEqual" | "moreThanOrEqual"
            ) || (v.is_number() && op.starts_with("is")) =>
        {
            number_holds(op, a, t).unwrap_or(false)
        }
        _ => text_holds(op, &as_text(v), &as_text(expected)).unwrap_or(false),
    };
    if found.iter().any(|v| holds(v)) {
        None
    } else {
        Some(format!(
            "{path} is {}, should {} {}",
            as_text(found[0]),
            op_words(op),
            as_text(expected)
        ))
    }
}

/// What an HTTP response is checked against.
pub struct HttpObserved<'a> {
    pub status: u16,
    pub duration_ms: f64,
    pub headers: &'a HashMap<String, String>,
    pub body: &'a str,
}

/// The first assertion an HTTP response fails.
pub fn http_failure(assertions: &[Assertion], r: &HttpObserved) -> Option<String> {
    assertions.iter().find_map(|a| match a.typ.as_str() {
        "statusCode" => check_number(a, "status code", r.status as f64),
        "responseTime" => check_number(a, "response time (ms)", r.duration_ms.round()),
        "body" if a.operator == "validatesJSONPath" => check_json_path(&a.target, r.body),
        "body" => check_text(a, "body", Some(r.body)),
        "header" => {
            let name = a.property.clone().unwrap_or_default().to_lowercase();
            check_text(
                a,
                &format!("header {name}"),
                r.headers.get(&name).map(String::as_str),
            )
        }
        other => Some(format!("unknown assertion {other}")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn http_assertions() {
        let a: Vec<Assertion> = serde_json::from_value(json!([
            {"type": "statusCode", "operator": "is", "target": 200},
            {"type": "responseTime", "operator": "lessThan", "target": 500},
            {"type": "body", "operator": "contains", "target": "ok"},
            {"type": "header", "property": "Content-Type", "operator": "matches", "target": "^application/json"}
        ]))
        .unwrap();
        let h = HashMap::from([(
            "content-type".to_string(),
            "application/json; charset=utf-8".to_string(),
        )]);
        let obs = |status, duration_ms, headers, body| HttpObserved {
            status,
            duration_ms,
            headers,
            body,
        };
        assert_eq!(
            http_failure(&a, &obs(200, 120.0, &h, r#"{"status":"ok"}"#)),
            None
        );
        assert_eq!(
            http_failure(&a, &obs(503, 120.0, &h, "")).as_deref(),
            Some("status code is 503, should be 200")
        );
        assert!(
            http_failure(&a, &obs(200, 900.0, &h, "ok"))
                .unwrap()
                .starts_with("response time")
        );
        assert!(
            http_failure(&a, &obs(200, 10.0, &h, "down"))
                .unwrap()
                .starts_with("body")
        );
        let empty = HashMap::new();
        assert!(
            http_failure(&a, &obs(200, 10.0, &empty, "ok"))
                .unwrap()
                .contains("missing")
        );
    }

    #[test]
    fn json_paths() {
        let doc = json!({"a": {"b": [{"c": 1}, {"c": 2}]}, "s": "ok", "n": {"deep": {"c": 3}}});
        let vals =
            |p: &str| -> Vec<Value> { json_path(&doc, p).unwrap().into_iter().cloned().collect() };
        assert_eq!(vals("$.s"), vec![json!("ok")]);
        assert_eq!(vals("$.a.b[1].c"), vec![json!(2)]);
        assert_eq!(vals("$.a.b[-1].c"), vec![json!(2)]);
        assert_eq!(vals("$['a']['b'][*].c"), vec![json!(1), json!(2)]);
        assert_eq!(vals("$..c"), vec![json!(1), json!(2), json!(3)]);
        assert!(vals("$.missing").is_empty());
        let a: Assertion =
            serde_json::from_value(json!({"type": "body", "operator": "validatesJSONPath",
            "target": {"jsonPath": "$.a.b[*].c", "operator": "moreThan", "targetValue": 1}}))
            .unwrap();
        let h = HashMap::new();
        let ok = HttpObserved {
            status: 200,
            duration_ms: 1.0,
            headers: &h,
            body: &doc.to_string(),
        };
        assert_eq!(http_failure(std::slice::from_ref(&a), &ok), None);
        let a2: Assertion =
            serde_json::from_value(json!({"type": "body", "operator": "validatesJSONPath",
            "target": {"jsonPath": "$.s", "operator": "is", "targetValue": "ko"}}))
            .unwrap();
        assert_eq!(
            http_failure(&[a2], &ok).as_deref(),
            Some("$.s is ok, should be ko")
        );
    }
}
