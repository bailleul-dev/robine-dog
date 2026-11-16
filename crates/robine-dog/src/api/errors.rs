//! Error Tracking: errors grouped into issues, as Datadog does. An error is a
//! span with `error.type` (APM) or an error log with `error.kind` or
//! `error.stack` (Logs). Its fingerprint is the service, the type, the message
//! with numbers and ids masked, and the top frame of the stack. An issue's
//! status (For Review, Reviewed, Ignored, Resolved) is stored; a resolved issue
//! seen again is a regression, back for review.

use super::{ApiError, compile, interval_ms, time_clause};
use crate::AppState;
use crate::query::sql::{self, Dataset};
use crate::store::greptime::{i64_at, lit, str_at};
use axum::Json;
use axum::extract::{Path, Query, State};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;

pub const KIND: &str = "error_issue";
const STATUSES: &[&str] = &["FOR_REVIEW", "REVIEWED", "IGNORED", "RESOLVED"];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IssueDoc {
    pub source: String,
    pub service: String,
    #[serde(rename = "type")]
    pub typ: String,
    pub pattern: String,
    pub frame: String,
    pub first_seen: i64,
    #[serde(default = "for_review")]
    pub status: String,
    #[serde(default)]
    pub status_updated: Option<i64>,
}

fn for_review() -> String {
    "FOR_REVIEW".into()
}

/// FNV-1a: a fingerprint stable across builds.
fn fingerprint(parts: &[&str]) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for p in parts {
        for b in p.bytes().chain([0u8]) {
            h ^= b as u64;
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
    }
    format!("{h:016x}")
}

struct Exprs {
    ds: Dataset,
    table: &'static str,
    typ: String,
    msg: String,
    stack: String,
    pattern: String,
    frame: String,
    is_error: String,
}

fn text(key: &str, ds: Dataset) -> String {
    format!(
        "CAST({} AS STRING)",
        sql::target_expr(&sql::resolve(key, ds))
    )
}

fn exprs(source: &str) -> Exprs {
    let (ds, table, typ, msg, stack, is_error) = if source == "logs" {
        let ds = Dataset::Logs;
        let typ = format!(
            "COALESCE({}, {})",
            text("@error.kind", ds),
            text("@error.type", ds)
        );
        let stack = text("@error.stack", ds);
        let is_error = format!(
            "status IN ('error', 'critical', 'alert', 'emergency') AND ({typ} IS NOT NULL OR {stack} IS NOT NULL)"
        );
        (
            ds,
            "logs",
            format!("COALESCE({typ}, 'Error')"),
            format!("COALESCE({}, message)", text("@error.message", ds)),
            stack,
            is_error,
        )
    } else {
        let ds = Dataset::Spans;
        let typ = text("error.type", ds);
        let is_error = format!("error AND {typ} IS NOT NULL");
        (
            ds,
            "spans",
            typ,
            text("error.message", ds),
            text("error.stack", ds),
            is_error,
        )
    };
    Exprs {
        pattern: format!(
            "regexp_replace(COALESCE({msg}, ''), '[0-9a-fA-F]{{8,}}|[0-9]+', '*', 'g')"
        ),
        // The first indented line of the stack: the top frame in Java, Python, Go, JS…
        frame: format!(
            "COALESCE(regexp_replace({stack}, '(?s)^.*?\\n[ \\t]+([^\\n]+).*$', '${{1}}'), '')"
        ),
        ds,
        table,
        typ,
        msg,
        stack,
        is_error,
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Bucket and timestamp columns are in nanoseconds for spans.
fn to_ms(v: i64, ds: Dataset) -> i64 {
    if ds == Dataset::Spans {
        v / 1_000_000
    } else {
        v
    }
}

#[derive(Debug, Deserialize)]
pub struct ListReq {
    pub from: i64,
    pub to: i64,
    #[serde(default)]
    pub query: String,
}

#[derive(Default)]
struct Agg {
    source: String,
    service: String,
    typ: String,
    pattern: String,
    frame: String,
    message: String,
    count: i64,
    first: i64,
    last: i64,
    series: Vec<i64>,
}

fn status_of(doc: &IssueDoc, last_seen: i64) -> (String, bool) {
    if doc.status == "RESOLVED" && doc.status_updated.is_some_and(|t| last_seen > t) {
        ("FOR_REVIEW".into(), true)
    } else {
        (doc.status.clone(), false)
    }
}

pub async fn issues(
    State(st): State<AppState>,
    Json(req): Json<ListReq>,
) -> Result<Json<Value>, ApiError> {
    let interval = interval_ms(req.from, req.to, 40);
    let buckets = ((req.to - req.from) / interval + 1).clamp(1, 200) as usize;
    let start = req.from.div_euclid(interval) * interval;
    let mut groups: HashMap<String, Agg> = HashMap::new();
    for source in ["spans", "logs"] {
        let e = exprs(source);
        let filter = compile(&req.query, e.ds)?;
        let sql = format!(
            "SELECT svc, et, nm, fr, b, count(*) AS c, max(msg) AS m, min(t) AS f, max(t) AS l FROM (
                SELECT COALESCE(service, '') AS svc, {typ} AS et, {msg} AS msg, {pattern} AS nm, {frame} AS fr,
                       CAST(date_bin(INTERVAL '{interval} milliseconds', ts) AS BIGINT) AS b, CAST(ts AS BIGINT) AS t
                FROM {table} WHERE {time} AND {is_error} AND {filter}
            ) GROUP BY svc, et, nm, fr, b",
            typ = e.typ,
            msg = e.msg,
            pattern = e.pattern,
            frame = e.frame,
            table = e.table,
            time = time_clause(e.ds, req.from, req.to),
            is_error = e.is_error,
        );
        for r in st.store.gt.query(&sql).await? {
            let (svc, typ, pattern, frame) = (
                str_at(&r, 0).unwrap_or_default(),
                str_at(&r, 1).unwrap_or_default(),
                str_at(&r, 2).unwrap_or_default(),
                str_at(&r, 3).unwrap_or_default(),
            );
            let id = fingerprint(&[source, &svc, &typ, &pattern, &frame]);
            let b = to_ms(i64_at(&r, 4), e.ds);
            let c = i64_at(&r, 5);
            let g = groups.entry(id).or_insert_with(|| Agg {
                source: source.into(),
                service: svc.clone(),
                typ: typ.clone(),
                pattern: pattern.clone(),
                frame: frame.clone(),
                first: i64::MAX,
                series: vec![0; buckets],
                ..Default::default()
            });
            g.count += c;
            g.first = g.first.min(to_ms(i64_at(&r, 7), e.ds));
            g.last = g.last.max(to_ms(i64_at(&r, 8), e.ds));
            if g.message.is_empty() {
                g.message = str_at(&r, 6).unwrap_or_default();
            }
            let i = ((b - start) / interval).clamp(0, buckets as i64 - 1) as usize;
            g.series[i] += c;
        }
    }
    let docs: HashMap<String, IssueDoc> = st
        .store
        .pg
        .docs(KIND)
        .await?
        .into_iter()
        .filter_map(|d| Some((d.id, serde_json::from_value(d.body).ok()?)))
        .collect();
    let mut out: Vec<(String, Agg)> = groups.into_iter().collect();
    out.sort_by_key(|a| std::cmp::Reverse(a.1.count));
    out.truncate(200);
    let mut list = Vec::new();
    for (id, g) in out {
        // Remember new issues (their first sighting) and earlier sightings.
        let doc = match docs.get(&id) {
            Some(d) if d.first_seen <= g.first => d.clone(),
            other => {
                let d = IssueDoc {
                    source: g.source.clone(),
                    service: g.service.clone(),
                    typ: g.typ.clone(),
                    pattern: g.pattern.clone(),
                    frame: g.frame.clone(),
                    first_seen: other.map(|d| d.first_seen.min(g.first)).unwrap_or(g.first),
                    status: other.map(|d| d.status.clone()).unwrap_or_else(for_review),
                    status_updated: other.and_then(|d| d.status_updated),
                };
                st.store
                    .pg
                    .put_doc(
                        KIND,
                        &id,
                        &serde_json::to_value(&d).map_err(anyhow::Error::from)?,
                    )
                    .await?;
                d
            }
        };
        let (status, regression) = status_of(&doc, g.last);
        list.push(json!({
            "id": id,
            "source": g.source,
            "service": g.service,
            "type": g.typ,
            "message": g.message,
            "pattern": g.pattern,
            "frame": g.frame,
            "count": g.count,
            "first_seen": doc.first_seen,
            "last_seen": g.last,
            "status": status,
            "regression": regression,
            "series": g.series,
        }));
    }
    Ok(Json(
        json!({"interval": interval, "start": start, "issues": list}),
    ))
}

#[derive(Debug, Deserialize)]
pub struct Range {
    pub from: i64,
    pub to: i64,
}

pub async fn issue(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Query(range): Query<Range>,
) -> Result<Json<Value>, ApiError> {
    let doc: IssueDoc = st
        .store
        .pg
        .doc(KIND, &id)
        .await?
        .and_then(|d| serde_json::from_value(d.body).ok())
        .ok_or(ApiError::NotFound)?;
    let e = exprs(&doc.source);
    let matching = format!(
        "{} AND COALESCE(service, '') = {} AND {} = {} AND {} = {} AND {} = {}",
        e.is_error,
        lit(&doc.service),
        e.typ,
        lit(&doc.typ),
        e.pattern,
        lit(&doc.pattern),
        e.frame,
        lit(&doc.frame)
    );
    let time = time_clause(e.ds, range.from, range.to);
    let interval = interval_ms(range.from, range.to, 60);
    let series_sql = format!(
        "SELECT CAST(date_bin(INTERVAL '{interval} milliseconds', ts) AS BIGINT) AS b, count(*) FROM {} WHERE {time} AND {matching} GROUP BY b ORDER BY b",
        e.table
    );
    let samples_sql = if doc.source == "logs" {
        format!(
            "SELECT CAST(ts AS BIGINT) AS t, CAST(id AS STRING), trace_id, span_id, host, {msg}, {stack}, service, status
             FROM logs WHERE {time} AND {matching} ORDER BY ts DESC LIMIT 200",
            msg = e.msg,
            stack = e.stack
        )
    } else {
        format!(
            "SELECT CAST(ts AS BIGINT) AS t, CAST(id AS STRING), trace_id, span_id, COALESCE({host}, {host2}), {msg}, {stack}, resource, env, version
             FROM spans WHERE {time} AND {matching} ORDER BY ts DESC LIMIT 200",
            host = text("_dd.hostname", Dataset::Spans),
            host2 = text("host", Dataset::Spans),
            msg = e.msg,
            stack = e.stack
        )
    };
    let (series, samples) = futures::try_join!(
        st.store.gt.query(&series_sql),
        st.store.gt.query(&samples_sql)
    )?;
    let series: Vec<[i64; 2]> = series
        .iter()
        .map(|r| [to_ms(i64_at(r, 0), e.ds), i64_at(r, 1)])
        .collect();
    let mut breakdown: HashMap<&str, HashMap<String, i64>> = HashMap::new();
    let occurrences: Vec<Value> = samples
        .iter()
        .map(|r| {
            let s = |i| str_at(r, i);
            let mut o = json!({
                "ts": to_ms(i64_at(r, 0), e.ds),
                "id": s(1),
                "trace_id": s(2),
                "span_id": s(3),
                "host": s(4),
                "message": s(5),
                "stack": s(6),
            });
            if doc.source == "spans" {
                o["resource"] = json!(s(7));
                o["env"] = json!(s(8));
                o["version"] = json!(s(9));
                for (k, v) in [
                    ("version", s(9)),
                    ("env", s(8)),
                    ("host", s(4)),
                    ("resource", s(7)),
                ] {
                    *breakdown
                        .entry(k)
                        .or_default()
                        .entry(v.unwrap_or_default())
                        .or_default() += 1;
                }
            } else {
                *breakdown
                    .entry("host")
                    .or_default()
                    .entry(s(4).unwrap_or_default())
                    .or_default() += 1;
            }
            o
        })
        .collect();
    let total: i64 = series.iter().map(|p| p[1]).sum();
    let last_seen = occurrences
        .first()
        .and_then(|o| o["ts"].as_i64())
        .unwrap_or(0);
    let (status, regression) = status_of(&doc, last_seen);
    let breakdown: HashMap<&str, Vec<(String, i64)>> = breakdown
        .into_iter()
        .map(|(k, m)| {
            let mut v: Vec<(String, i64)> = m.into_iter().filter(|(k, _)| !k.is_empty()).collect();
            v.sort_by_key(|a| std::cmp::Reverse(a.1));
            v.truncate(10);
            (k, v)
        })
        .collect();
    Ok(Json(json!({
        "id": id,
        "source": doc.source,
        "service": doc.service,
        "type": doc.typ,
        "pattern": doc.pattern,
        "frame": doc.frame,
        "first_seen": doc.first_seen,
        "last_seen": last_seen,
        "status": status,
        "regression": regression,
        "count": total,
        "interval": interval,
        "series": series,
        "occurrences": occurrences,
        "breakdown": breakdown,
        "query": issue_query(&doc),
    })))
}

/// A search query for the issue's events in the Explorer.
fn issue_query(doc: &IssueDoc) -> String {
    let q = |s: &str| format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""));
    if doc.source == "logs" {
        format!(
            "service:{} status:error @error.kind:{}",
            q(&doc.service),
            q(&doc.typ)
        )
    } else {
        format!(
            "service:{} status:error @error.type:{}",
            q(&doc.service),
            q(&doc.typ)
        )
    }
}

#[derive(Debug, Deserialize)]
pub struct StatusReq {
    pub status: String,
}

pub async fn set_status(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<StatusReq>,
) -> Result<Json<Value>, ApiError> {
    if !STATUSES.contains(&req.status.as_str()) {
        return Err(ApiError::BadQuery(format!(
            "status is one of {}",
            STATUSES.join(", ")
        )));
    }
    let mut doc: IssueDoc = st
        .store
        .pg
        .doc(KIND, &id)
        .await?
        .and_then(|d| serde_json::from_value(d.body).ok())
        .ok_or(ApiError::NotFound)?;
    doc.status = req.status;
    doc.status_updated = Some(now_ms());
    st.store
        .pg
        .put_doc(
            KIND,
            &id,
            &serde_json::to_value(&doc).map_err(anyhow::Error::from)?,
        )
        .await?;
    Ok(Json(json!({"id": id, "status": doc.status})))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprints_and_regressions() {
        assert_eq!(fingerprint(&["a", "b"]), fingerprint(&["a", "b"]));
        assert_ne!(fingerprint(&["ab", ""]), fingerprint(&["a", "b"]));
        let mut d = IssueDoc {
            source: "spans".into(),
            service: "s".into(),
            typ: "E".into(),
            pattern: "p".into(),
            frame: "f".into(),
            first_seen: 0,
            status: "RESOLVED".into(),
            status_updated: Some(100),
        };
        assert_eq!(status_of(&d, 50), ("RESOLVED".into(), false));
        assert_eq!(status_of(&d, 150), ("FOR_REVIEW".into(), true));
        d.status = "IGNORED".into();
        assert_eq!(status_of(&d, 150), ("IGNORED".into(), false));
    }
}
