//! Service checks: the Agent's check runs (`POST /api/v1/check_run`, what
//! `datadog.agent.up` and every integration's `*.can_connect` report) and
//! DogStatsD's `_sc|<name>|<status>|…` lines. Statuses are Datadog's:
//! 0 OK, 1 WARNING, 2 CRITICAL, 3 UNKNOWN.

use crate::AppState;
use crate::intake::logs::{api_key_ok, decompress, now_ms};
use crate::query::sql::{Dataset, compile};
use crate::store::greptime::{i64_at, lit, opt_lit, str_at, tags_column};
use crate::store::writer::Insertable;
use axum::Json;
use axum::body::Bytes;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap};

pub const OK: i64 = 0;
pub const WARNING: i64 = 1;
pub const CRITICAL: i64 = 2;
pub const UNKNOWN: i64 = 3;

pub fn status_name(s: i64) -> &'static str {
    match s {
        OK => "OK",
        WARNING => "WARNING",
        CRITICAL => "CRITICAL",
        _ => "UNKNOWN",
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CheckRun {
    /// Epoch milliseconds.
    pub ts: i64,
    pub check: String,
    pub host: Option<String>,
    pub status: i64,
    pub message: String,
    /// Sorted `key:value` tags.
    pub tags: Vec<String>,
}

impl Insertable for CheckRun {
    const TABLE: &'static str = "check_runs";
    const COLUMNS: &'static str = r#"("ts", "check", "host", "status", "message", "tags")"#;

    fn values(&self, out: &mut String) {
        use std::fmt::Write;
        let _ = write!(
            out,
            "({}, {}, {}, {}, {}, {})",
            self.ts,
            lit(&self.check),
            opt_lit(self.host.as_deref()),
            self.status,
            lit(&self.message),
            lit(&tags_column(&self.tags)),
        );
    }
}

fn normalized(mut tags: Vec<String>) -> Vec<String> {
    tags.retain(|t| !t.is_empty());
    tags.sort();
    tags.dedup();
    tags
}

/// One check run of the Agent's JSON.
fn from_json(v: &Value, now: i64) -> Option<CheckRun> {
    let check = v.get("check")?.as_str()?.trim();
    if check.is_empty() {
        return None;
    }
    let tags = v
        .get("tags")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(String::from)
                .collect()
        })
        .unwrap_or_default();
    Some(CheckRun {
        ts: v
            .get("timestamp")
            .and_then(Value::as_f64)
            .map(|s| (s * 1000.0) as i64)
            .filter(|t| *t > 0)
            .unwrap_or(now),
        check: check.to_string(),
        host: v
            .get("host_name")
            .and_then(Value::as_str)
            .filter(|h| !h.is_empty())
            .map(String::from),
        status: v
            .get("status")
            .and_then(Value::as_i64)
            .unwrap_or(UNKNOWN)
            .clamp(0, 3),
        message: v
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        tags: normalized(tags),
    })
}

/// A DogStatsD service check:
/// `_sc|<name>|<status>[|d:<ts>][|h:<host>][|#<tags>][|m:<message>]`.
pub fn parse_dogstatsd(line: &str, now: i64) -> Option<CheckRun> {
    let rest = line.trim().strip_prefix("_sc|")?;
    let mut parts = rest.split('|');
    let check = parts.next()?.trim();
    let status: i64 = parts
        .next()?
        .trim()
        .parse()
        .ok()
        .filter(|s| (0..=3).contains(s))?;
    if check.is_empty() {
        return None;
    }
    let mut run = CheckRun {
        ts: now,
        check: check.to_string(),
        host: None,
        status,
        message: String::new(),
        tags: Vec::new(),
    };
    for p in parts {
        if let Some(d) = p.strip_prefix("d:") {
            if let Ok(s) = d.parse::<i64>() {
                run.ts = s * 1000;
            }
        } else if let Some(h) = p.strip_prefix("h:") {
            run.host = Some(h.to_string());
        } else if let Some(t) = p.strip_prefix('#') {
            run.tags = normalized(t.split(',').map(str::to_string).collect());
        } else if let Some(m) = p.strip_prefix("m:") {
            // The message is last and may itself hold `|`… escaped as `\n`.
            run.message = m.replace("\\n", "\n");
        }
    }
    if run.host.is_none()
        && let Some(i) = run.tags.iter().position(|t| t.starts_with("host:"))
    {
        run.host = Some(run.tags.remove(i)[5..].to_string());
    }
    Some(run)
}

/// `POST /api/v1/check_run`: the Agent's check runs, a JSON array.
pub async fn intake(
    State(st): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
    body: Bytes,
) -> Response {
    if !api_key_ok(&st, &headers, &q) {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({"errors": ["Forbidden"]})),
        )
            .into_response();
    }
    let raw = match decompress(&headers, body) {
        Ok(b) => b,
        Err(e) => return (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
    };
    let v: Value = match serde_json::from_slice(&raw) {
        Ok(v) => v,
        Err(e) => return (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
    };
    let now = now_ms();
    let runs: Vec<CheckRun> = match &v {
        Value::Array(a) => a.iter().filter_map(|r| from_json(r, now)).collect(),
        other => from_json(other, now).into_iter().collect(),
    };
    if let Err(e) = st.store.checks.send(runs).await {
        tracing::warn!(error = %e, "storing check runs");
    }
    (StatusCode::ACCEPTED, Json(json!({"status": "ok"}))).into_response()
}

/// The latest statuses of a check, newest last, per group of `by` tags
/// (`host` is the host column), over the runs matching `filter` (tags, in
/// the metric filter syntax) in `[from, to]`, at most `n` per group.
pub async fn latest(
    st: &AppState,
    check: &str,
    filter: &str,
    by: &[String],
    n: usize,
    from: i64,
    to: i64,
) -> anyhow::Result<Vec<(BTreeMap<String, String>, Vec<i64>)>> {
    let search = filter_search(filter);
    let cond = compile(
        &crate::query::parser::parse(&search).map_err(|e| anyhow::anyhow!(e.0))?,
        Dataset::Metrics,
    );
    let key = |k: &str| {
        if k == "host" {
            "host".to_string()
        } else {
            format!(
                "array_element(regexp_match(tags, {}), 1)",
                lit(&format!(",{}:([^,]*),", crate::query::sql::tag_regex(k)))
            )
        }
    };
    let cols: Vec<String> = by.iter().map(|k| key(k)).collect();
    let sel = if cols.is_empty() {
        String::new()
    } else {
        format!("{}, ", cols.join(", "))
    };
    let rows = st
        .store
        .gt
        .query(&format!(
            "SELECT {sel}CAST(ts AS BIGINT) AS t, status FROM check_runs
             WHERE \"check\" = {} AND ts >= {from} AND ts <= {to} AND {cond}
             ORDER BY ts DESC LIMIT 20000",
            lit(check)
        ))
        .await?;
    let mut groups: BTreeMap<Vec<String>, Vec<i64>> = BTreeMap::new();
    for r in &rows {
        let k: Vec<String> = (0..by.len())
            .map(|i| str_at(r, i).unwrap_or_else(|| "N/A".into()))
            .collect();
        let g = groups.entry(k).or_default();
        if g.len() < n.max(1) {
            g.push(i64_at(r, by.len() + 1));
        }
    }
    Ok(groups
        .into_iter()
        .map(|(k, mut v)| {
            v.reverse();
            (by.iter().cloned().zip(k).collect(), v)
        })
        .collect())
}

/// `env:prod,!host:a` (or `*`) → the search syntax.
fn filter_search(f: &str) -> String {
    let f = f.trim();
    if f.is_empty() || f == "*" {
        return String::new();
    }
    f.split(',')
        .map(|t| {
            let t = t.trim().trim_matches('"');
            match t.strip_prefix('!') {
                Some(rest) => format!("-{rest}"),
                None => t.to_string(),
            }
        })
        .filter(|t| !t.is_empty())
        .collect::<Vec<_>>()
        .join(" AND ")
}

/// A check's state now, as a service check monitor or the check status
/// widget judge it: CRITICAL after `critical` consecutive CRITICAL runs,
/// WARNING after `warning` consecutive WARNING-or-worse runs, OK after `ok`
/// consecutive OK runs; `None` while undecided (the state stays).
pub fn judge(recent: &[i64], ok: usize, warning: usize, critical: usize) -> Option<i64> {
    let tail = |n: usize, f: &dyn Fn(i64) -> bool| {
        n > 0 && recent.len() >= n && recent[recent.len() - n..].iter().all(|s| f(*s))
    };
    if tail(critical.max(1), &|s| s == CRITICAL) {
        Some(CRITICAL)
    } else if tail(warning.max(1), &|s| s == WARNING || s == CRITICAL) {
        Some(WARNING)
    } else if tail(ok.max(1), &|s| s == OK) {
        Some(OK)
    } else {
        None
    }
}

#[derive(Deserialize)]
pub struct ListParams {
    from: Option<i64>,
    to: Option<i64>,
}

#[derive(Serialize)]
pub struct CheckSummary {
    check: String,
    host: Option<String>,
    status: &'static str,
    message: String,
    last: i64,
    tags: Vec<String>,
}

/// `GET /api/ui/checks`: each check's last run per host.
pub async fn list(
    State(st): State<AppState>,
    Query(p): Query<ListParams>,
) -> Result<Json<Vec<CheckSummary>>, crate::api::ApiError> {
    let to = p.to.unwrap_or_else(now_ms);
    let from = p.from.unwrap_or(to - 3_600_000);
    let rows = st
        .store
        .gt
        .query(&format!(
            "SELECT \"check\", host, status, message, CAST(ts AS BIGINT) AS t, tags FROM check_runs
             WHERE ts >= {from} AND ts <= {to} ORDER BY ts DESC LIMIT 20000"
        ))
        .await?;
    let mut seen: BTreeMap<(String, Option<String>), CheckSummary> = BTreeMap::new();
    for r in &rows {
        let key = (str_at(r, 0).unwrap_or_default(), str_at(r, 1));
        seen.entry(key.clone()).or_insert_with(|| CheckSummary {
            check: key.0,
            host: key.1,
            status: status_name(i64_at(r, 2)),
            message: str_at(r, 3).unwrap_or_default(),
            last: i64_at(r, 4),
            tags: crate::store::greptime::parse_tags_column(&str_at(r, 5).unwrap_or_default()),
        });
    }
    Ok(Json(seen.into_values().collect()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_check_runs() {
        let r = from_json(
            &json!({"check": "datadog.agent.up", "host_name": "h1", "timestamp": 1_700_000_000, "status": 0, "tags": ["env:prod"]}),
            5,
        )
        .unwrap();
        assert_eq!(r.ts, 1_700_000_000_000);
        assert_eq!(r.host.as_deref(), Some("h1"));
        assert_eq!(r.status, OK);
        assert!(from_json(&json!({"status": 0}), 0).is_none());

        let r = parse_dogstatsd(
            "_sc|redis.can_connect|2|d:1700000000|h:db-1|#env:prod,port:6379|m:refused",
            0,
        )
        .unwrap();
        assert_eq!(r.check, "redis.can_connect");
        assert_eq!(r.status, CRITICAL);
        assert_eq!(r.host.as_deref(), Some("db-1"));
        assert_eq!(r.tags, vec!["env:prod", "port:6379"]);
        assert_eq!(r.message, "refused");
        assert_eq!(r.ts, 1_700_000_000_000);
        assert!(parse_dogstatsd("_sc|x|9", 0).is_none());
        assert!(parse_dogstatsd("a:1|c", 0).is_none());
    }

    #[test]
    fn judges_consecutive_runs() {
        assert_eq!(judge(&[OK, CRITICAL, CRITICAL], 1, 1, 2), Some(CRITICAL));
        assert_eq!(judge(&[OK, OK, CRITICAL], 1, 2, 2), None);
        assert_eq!(judge(&[CRITICAL, OK], 1, 1, 1), Some(OK));
        assert_eq!(judge(&[CRITICAL, OK], 2, 1, 1), None);
        assert_eq!(judge(&[WARNING], 1, 1, 1), Some(WARNING));
        assert_eq!(judge(&[], 1, 1, 1), None);
    }
}
