//! Multistep API tests: requests in order, each able to use what the
//! previous ones extracted (`extractedValues`) as `{{ NAME }}`.

use super::assertions::{Assertion, json_path};
use super::net::{self, HttpResponse};
use super::variables::{Vars, render_value};
use super::{Request, StepResult, TestOptions};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Parser {
    /// `raw`, `json_path`, `regex`.
    #[serde(rename = "type", default)]
    pub typ: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Extract {
    pub name: String,
    /// `http_body`, `http_header`, `http_status`.
    #[serde(rename = "type", default)]
    pub typ: String,
    /// The header's name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    #[serde(default)]
    pub parser: Parser,
    #[serde(default)]
    pub secure: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ApiStep {
    pub name: String,
    /// `http`, `ssl`, `tcp`, `dns`, `udp`, `websocket`, `grpc`, `icmp`,
    /// or `wait` (`value` seconds).
    #[serde(default)]
    pub subtype: String,
    #[serde(default)]
    pub request: Request,
    #[serde(default)]
    pub assertions: Vec<Assertion>,
    #[serde(
        rename = "extractedValues",
        default,
        skip_serializing_if = "Vec::is_empty"
    )]
    pub extracted_values: Vec<Extract>,
    /// The test goes on when this step fails.
    #[serde(rename = "allowFailure", default)]
    pub allow_failure: bool,
    /// With `allowFailure`: whether its failure still fails the test.
    #[serde(
        rename = "isCritical",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub is_critical: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<f64>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// A value extracted from a response.
pub fn extract(e: &Extract, r: &HttpResponse) -> Result<String, String> {
    let source = match e.typ.as_str() {
        "http_status" => r.status.to_string(),
        "http_header" => {
            let name = e.field.clone().unwrap_or_default().to_lowercase();
            r.headers
                .get(&name)
                .cloned()
                .ok_or_else(|| format!("no header {name}"))?
        }
        _ => r.body.clone(),
    };
    let pattern = e.parser.value.clone().unwrap_or_default();
    match e.parser.typ.as_str() {
        "json_path" => {
            let doc: Value =
                serde_json::from_str(&source).map_err(|_| "the body is not JSON".to_string())?;
            let found = json_path(&doc, &pattern)?;
            let v = found
                .first()
                .ok_or_else(|| format!("{pattern} matches nothing"))?;
            Ok(match v {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            })
        }
        "regex" => {
            let re = regex::Regex::new(&pattern).map_err(|e| format!("bad regex: {e}"))?;
            let c = re
                .captures(&source)
                .ok_or_else(|| format!("{pattern} matches nothing"))?;
            Ok(c.get(1)
                .or_else(|| c.get(0))
                .map(|m| m.as_str().to_string())
                .unwrap_or_default())
        }
        _ => Ok(source),
    }
}

/// The steps' results, and whether the test passed.
pub async fn run(
    steps: &[ApiStep],
    opts: &TestOptions,
    vars: &mut Vars,
) -> (Vec<StepResult>, Option<String>) {
    let mut results = Vec::new();
    let mut failure = None;
    for (i, s) in steps.iter().enumerate() {
        let started = Instant::now();
        if s.subtype == "wait" {
            let secs = s.value.unwrap_or(1.0).clamp(0.0, 180.0);
            tokio::time::sleep(Duration::from_secs_f64(secs)).await;
            results.push(StepResult {
                name: if s.name.is_empty() {
                    format!("Wait {secs} s")
                } else {
                    s.name.clone()
                },
                typ: "wait".into(),
                passed: true,
                duration_ms: started.elapsed().as_secs_f64() * 1000.0,
                ..Default::default()
            });
            continue;
        }
        let req: Request = serde_json::to_value(&s.request)
            .ok()
            .map(|v| render_value(&v, vars))
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_else(|| s.request.clone());
        let subtype = if s.subtype.is_empty() {
            "http"
        } else {
            &s.subtype
        };
        let obs = net::run(subtype, &req, &s.assertions, opts).await;
        let mut step_failure = obs.failure.clone();
        let mut extracted = Map::new();
        if step_failure.is_none() {
            for e in &s.extracted_values {
                let Some(resp) = &obs.http else {
                    step_failure = Some(format!("{}: nothing to extract from", e.name));
                    break;
                };
                match extract(e, resp) {
                    Ok(v) => {
                        extracted.insert(
                            e.name.clone(),
                            if e.secure {
                                "••••••".into()
                            } else {
                                Value::String(v.clone())
                            },
                        );
                        vars.insert(e.name.clone(), v);
                    }
                    Err(why) => {
                        step_failure = Some(format!("extracting {}: {why}", e.name));
                        break;
                    }
                }
            }
        }
        let passed = step_failure.is_none();
        results.push(StepResult {
            name: if s.name.is_empty() {
                format!("Step {}", i + 1)
            } else {
                s.name.clone()
            },
            typ: subtype.to_string(),
            passed,
            duration_ms: obs.duration_ms,
            failure: step_failure.clone(),
            allow_failure: s.allow_failure,
            status_code: obs.status_code,
            url: (!req.url.is_empty()).then(|| req.url.clone()),
            details: obs.details,
            extracted,
            ..Default::default()
        });
        if let Some(f) = step_failure {
            let critical = !s.allow_failure || s.is_critical.unwrap_or(true);
            if critical && failure.is_none() {
                failure = Some(format!("step {} ({}): {f}", i + 1, results[i].name));
            }
            if !s.allow_failure {
                break;
            }
        }
    }
    (results, failure)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::HashMap;

    #[test]
    fn extraction() {
        let r = HttpResponse {
            status: 201,
            headers: HashMap::from([("location".into(), "/orders/42".into())]),
            body: r#"{"order": {"id": 42, "token": "t-1"}}"#.into(),
        };
        let e = |v: Value| serde_json::from_value::<Extract>(v).unwrap();
        assert_eq!(
            extract(
                &e(
                    json!({"name": "ID", "type": "http_body", "parser": {"type": "json_path", "value": "$.order.id"}})
                ),
                &r
            ),
            Ok("42".into())
        );
        assert_eq!(
            extract(
                &e(
                    json!({"name": "T", "type": "http_body", "parser": {"type": "json_path", "value": "$.order.token"}})
                ),
                &r
            ),
            Ok("t-1".into())
        );
        assert_eq!(
            extract(
                &e(
                    json!({"name": "N", "type": "http_header", "field": "Location", "parser": {"type": "regex", "value": "/orders/(\\d+)"}})
                ),
                &r
            ),
            Ok("42".into())
        );
        assert_eq!(
            extract(
                &e(json!({"name": "S", "type": "http_status", "parser": {"type": "raw"}})),
                &r
            ),
            Ok("201".into())
        );
        assert!(
            extract(
                &e(json!({"name": "X", "type": "http_header", "field": "nope"})),
                &r
            )
            .is_err()
        );
    }
}
