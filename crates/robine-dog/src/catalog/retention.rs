//! APM retention filters and ingestion control, as Datadog's: every ingested
//! span feeds the trace metrics, then retention filters, in order, decide
//! what is kept. A trace is kept when one of its spans matches an enabled
//! filter and the trace falls in that filter's rate (decided on the trace
//! id, so every span of a trace gets the same decision). robine-dog keeps or
//! drops whole trace chunks. A trace's spans can arrive in several chunks
//! (one per service or per flush of the Agent): each is decided on its own.
//! Decisions by rate are the same for every chunk of a trace; a filter on one
//! service's spans keeps that service's chunks only.
//!
//! Defaults, created on first start: `Error Default` (`status:error`, 100%)
//! and `Default` (`*`, 100%), i.e. everything is kept until filters change.

use crate::AppState;
use crate::api::{ApiError, ApiResult};
use crate::intake::traces::SpanRow;
use crate::metrics::{Aggregator, Sample};
use crate::query::matcher::matches;
use crate::query::parser::Expr;
use axum::Json;
use axum::extract::{Path, State};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap};
use std::sync::{Mutex, RwLock};

pub const KIND: &str = "apm_retention_filter";
pub const ORDER_KIND: &str = "apm_retention_filter_order";
const ERRORS_TYPE: &str = "spans-errors-sampling-processor";
const CUSTOM_TYPE: &str = "spans-sampling-processor";
const MINUTE: i64 = 60_000;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Query {
    #[serde(default)]
    pub query: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilterDef {
    pub name: String,
    #[serde(default)]
    pub filter: Query,
    /// The share of matching traces kept, 0 to 1.
    pub rate: f64,
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default = "custom")]
    pub filter_type: String,
    /// Accepted for compatibility: robine-dog keeps whole traces anyway.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace_rate: Option<f64>,
}

fn yes() -> bool {
    true
}
fn custom() -> String {
    CUSTOM_TYPE.into()
}

struct Compiled {
    name: String,
    expr: Expr,
    rate: f64,
}

/// Spans and traces of a service in one minute.
#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct Counts {
    pub spans: u64,
    pub kept_spans: u64,
    pub traces: u64,
    pub kept_traces: u64,
    pub bytes: u64,
}

impl Counts {
    fn add(&mut self, o: &Counts) {
        self.spans += o.spans;
        self.kept_spans += o.kept_spans;
        self.traces += o.traces;
        self.kept_traces += o.kept_traces;
        self.bytes += o.bytes;
    }
}

#[derive(Default)]
struct ServiceUsage {
    /// The last hour, by minute.
    minutes: BTreeMap<i64, Counts>,
    /// Root spans by sampling decision maker (`_dd.p.dm`) and priority.
    mechanisms: BTreeMap<String, u64>,
    /// Tracer languages and versions seen.
    tracers: BTreeMap<String, u64>,
    last_seen: i64,
}

#[derive(Default)]
pub struct Retention {
    filters: RwLock<Option<Vec<Compiled>>>,
    usage: Mutex<HashMap<String, ServiceUsage>>,
}

/// A stable number in [0, 1) for a trace id: Datadog's sampling hash
/// (the id's low 64 bits times 1111111111111111111), FNV-1a then splitmix
/// for ids that are not numbers.
fn trace_fraction(trace_id: &str) -> f64 {
    let low64 = trace_id.parse::<u128>().ok().map(|n| n as u64).or_else(|| {
        let hex = trace_id.trim_start_matches("0x");
        let tail = &hex[hex.len().saturating_sub(16)..];
        u64::from_str_radix(tail, 16)
            .ok()
            .filter(|_| !tail.is_empty())
    });
    let h = match low64 {
        Some(n) => n.wrapping_mul(1_111_111_111_111_111_111),
        None => {
            let mut h: u64 = 0xcbf2_9ce4_8422_2325;
            for b in trace_id.bytes() {
                h ^= b as u64;
                h = h.wrapping_mul(0x0100_0000_01b3);
            }
            h ^= h >> 30;
            h = h.wrapping_mul(0xbf58_476d_1ce4_e5b9);
            h ^= h >> 27;
            h = h.wrapping_mul(0x94d0_49bb_1331_11eb);
            h ^ (h >> 31)
        }
    };
    (h >> 11) as f64 / (1u64 << 53) as f64
}

/// The decision maker of a root span, as Datadog's Ingestion Control shows it.
fn mechanism(s: &SpanRow) -> String {
    let dm = s.meta.get("_dd.p.dm").and_then(Value::as_str).unwrap_or("");
    let priority = s
        .metrics
        .get("_sampling_priority_v1")
        .and_then(Value::as_f64);
    let name = match dm.trim_start_matches('-') {
        "0" | "" if priority.is_none() => "none",
        "0" | "" => "default",
        "1" => "agent rate",
        "2" => "remote rate (auto)",
        "3" => "rule",
        "4" => "manual",
        "5" => "appsec",
        "8" => "single span",
        "11" => "remote rate (user)",
        "12" => "remote rate (dynamic)",
        _ => "other",
    };
    match priority {
        Some(p) if p > 0.0 => format!("{name}: keep"),
        Some(_) => format!("{name}: drop"),
        None => name.to_string(),
    }
}

fn span_size(s: &SpanRow) -> u64 {
    (s.service.len()
        + s.name.len()
        + s.resource.len()
        + 80
        + s.meta
            .iter()
            .map(|(k, v)| k.len() + v.to_string().len())
            .sum::<usize>()
        + s.metrics.iter().map(|(k, _)| k.len() + 8).sum::<usize>()) as u64
}

impl Retention {
    pub fn load(&self, defs: &[(String, FilterDef)]) {
        let compiled = defs
            .iter()
            .filter(|(_, d)| d.enabled)
            .filter_map(|(_, d)| {
                Some(Compiled {
                    name: d.name.clone(),
                    expr: crate::query::parser::parse(&d.filter.query).ok()?,
                    rate: d.rate.clamp(0.0, 1.0),
                })
            })
            .collect();
        *self.filters.write().unwrap_or_else(|e| e.into_inner()) = Some(compiled);
    }

    /// The filter keeping a trace, if any. Before any configuration is
    /// loaded, everything is kept.
    pub fn keep(&self, chunk: &[SpanRow]) -> Option<String> {
        let filters = self.filters.read().unwrap_or_else(|e| e.into_inner());
        let Some(filters) = filters.as_ref() else {
            return Some(String::new());
        };
        let trace = chunk.first().map(|s| s.trace_id.as_str()).unwrap_or("");
        let f = trace_fraction(trace);
        filters
            .iter()
            .filter(|c| chunk.iter().any(|s| matches(&c.expr, s)))
            .find(|c| f < c.rate)
            .map(|c| c.name.clone())
    }

    /// Counts a trace, kept or not, and sends Datadog's usage metrics.
    pub fn record(&self, agg: &Aggregator, chunk: &[SpanRow], kept: bool, now: i64) {
        let minute = now - now % MINUTE;
        let mut by_service: HashMap<&str, Counts> = HashMap::new();
        for s in chunk {
            let c = by_service.entry(s.service.as_str()).or_default();
            c.spans += 1;
            c.bytes += span_size(s);
            if kept {
                c.kept_spans += 1;
            }
        }
        let root = chunk
            .iter()
            .find(|s| s.parent_id == "0" || s.parent_id.is_empty())
            .or(chunk.first());
        let mut usage = self.usage.lock().unwrap_or_else(|e| e.into_inner());
        for (service, c) in &by_service {
            let u = usage.entry(service.to_string()).or_default();
            let m = u.minutes.entry(minute).or_default();
            m.add(c);
            if root.is_some_and(|r| r.service == *service) {
                m.traces += 1;
                if kept {
                    m.kept_traces += 1;
                }
            }
            u.last_seen = now;
            while u
                .minutes
                .first_key_value()
                .is_some_and(|(k, _)| *k <= minute - 60 * MINUTE)
            {
                u.minutes.pop_first();
            }
            let tags = vec![format!("service:{service}")];
            agg.add(
                now,
                "datadog.estimated_usage.apm.ingested_spans",
                None,
                tags.clone(),
                Sample::Count(c.spans as f64),
            );
            agg.add(
                now,
                "datadog.estimated_usage.apm.ingested_bytes",
                None,
                tags.clone(),
                Sample::Count(c.bytes as f64),
            );
            agg.add(
                now,
                "datadog.estimated_usage.apm.indexed_spans",
                None,
                tags,
                Sample::Count(c.kept_spans as f64),
            );
        }
        if let Some(r) = root {
            let u = usage.entry(r.service.clone()).or_default();
            *u.mechanisms.entry(mechanism(r)).or_default() += 1;
            let lang = r
                .meta
                .get("language")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            let version = r
                .meta
                .get("_dd.tracer_version")
                .and_then(Value::as_str)
                .unwrap_or("");
            let tracer = format!("{lang} {version}").trim().to_string();
            *u.tracers.entry(tracer).or_default() += 1;
        }
        usage.retain(|_, u| u.last_seen > now - 24 * 60 * MINUTE);
    }

    /// Per service: the last hour's totals, sampling mechanisms and tracers.
    pub fn usage(&self, now: i64) -> Vec<Value> {
        let usage = self.usage.lock().unwrap_or_else(|e| e.into_inner());
        let mut out: Vec<Value> = usage
            .iter()
            .map(|(service, u)| {
                let mut total = Counts::default();
                for (m, c) in &u.minutes {
                    if *m > now - 60 * MINUTE {
                        total.add(c);
                    }
                }
                json!({
                    "service": service,
                    "last_hour": total,
                    "spans_per_second": total.spans as f64 / 3600.0,
                    "mechanisms": u.mechanisms,
                    "tracers": u.tracers,
                    "last_seen": u.last_seen,
                })
            })
            .collect();
        out.sort_by(|a, b| {
            b["last_hour"]["spans"]
                .as_u64()
                .cmp(&a["last_hour"]["spans"].as_u64())
        });
        out
    }
}

/// Stores traces: trace metrics and usage from all of them, then those the
/// retention filters keep.
pub fn filter(st: &AppState, chunks: Vec<Vec<SpanRow>>, now: i64) -> Vec<Vec<SpanRow>> {
    let r = &st.catalog.retention;
    chunks
        .into_iter()
        .filter_map(|mut chunk| {
            let kept = r.keep(&chunk);
            r.record(&st.aggregator, &chunk, kept.is_some(), now);
            let name = kept?;
            if !name.is_empty() {
                for s in &mut chunk {
                    s.meta
                        .insert("_dd.retention_filter".into(), Value::String(name.clone()));
                }
            }
            Some(chunk)
        })
        .collect()
}

// ---------------------------------------------------------------- storage

async fn order(st: &AppState) -> anyhow::Result<Vec<String>> {
    Ok(st
        .store
        .pg
        .doc(ORDER_KIND, "order")
        .await?
        .and_then(|d| serde_json::from_value(d.body["ids"].clone()).ok())
        .unwrap_or_default())
}

async fn set_order(st: &AppState, ids: &[String]) -> anyhow::Result<()> {
    st.store
        .pg
        .put_doc(ORDER_KIND, "order", &json!({"ids": ids}))
        .await?;
    Ok(())
}

/// The filters in their execution order.
async fn stored(st: &AppState) -> anyhow::Result<Vec<(String, FilterDef, i64, i64)>> {
    let docs = st.store.pg.docs(KIND).await?;
    let mut defs: Vec<(String, FilterDef, i64, i64)> = docs
        .into_iter()
        .filter_map(|d| {
            Some((
                d.id,
                serde_json::from_value(d.body).ok()?,
                d.created_at,
                d.updated_at,
            ))
        })
        .collect();
    let order = order(st).await?;
    defs.sort_by_key(|(id, _, created, _)| {
        (
            order.iter().position(|o| o == id).unwrap_or(usize::MAX),
            *created,
        )
    });
    Ok(defs)
}

pub async fn reload(st: &AppState) -> anyhow::Result<()> {
    let defs: Vec<(String, FilterDef)> = stored(st)
        .await?
        .into_iter()
        .map(|(id, d, _, _)| (id, d))
        .collect();
    st.catalog.retention.load(&defs);
    Ok(())
}

/// The default filters, on first start.
pub async fn init(st: &AppState) -> anyhow::Result<()> {
    if st.store.pg.docs(KIND).await?.is_empty()
        && st.store.pg.doc(ORDER_KIND, "order").await?.is_none()
    {
        let errors = FilterDef {
            name: "Error Default".into(),
            filter: Query {
                query: "status:error".into(),
            },
            rate: 1.0,
            enabled: true,
            filter_type: ERRORS_TYPE.into(),
            trace_rate: None,
        };
        let all = FilterDef {
            name: "Default".into(),
            filter: Query { query: "*".into() },
            rate: 1.0,
            enabled: true,
            filter_type: CUSTOM_TYPE.into(),
            trace_rate: None,
        };
        let (a, b) = (crate::store::pg::new_id(), crate::store::pg::new_id());
        st.store
            .pg
            .put_doc(KIND, &a, &serde_json::to_value(&errors)?)
            .await?;
        st.store
            .pg
            .put_doc(KIND, &b, &serde_json::to_value(&all)?)
            .await?;
        set_order(st, &[a, b]).await?;
    }
    reload(st).await
}

fn validate(d: &FilterDef) -> Result<(), String> {
    if d.name.trim().is_empty() {
        return Err("the filter has no name".into());
    }
    if !(0.0..=1.0).contains(&d.rate) {
        return Err("rate is between 0 and 1".into());
    }
    if d.trace_rate.is_some_and(|r| !(0.0..=1.0).contains(&r)) {
        return Err("trace_rate is between 0 and 1".into());
    }
    crate::query::parser::parse(&d.filter.query).map_err(|e| format!("filter.query: {}", e.0))?;
    Ok(())
}

fn out(id: &str, d: &FilterDef, created: i64, updated: i64) -> Value {
    json!({
        "id": id,
        "type": "apm_retention_filter",
        "attributes": {
            "name": d.name, "filter": d.filter, "rate": d.rate, "trace_rate": d.trace_rate,
            "enabled": d.enabled, "filter_type": d.filter_type,
            "editable": true,
            "execution_order": 0,
            "created_at": created / 1000, "modified_at": updated / 1000,
        }
    })
}

fn body(v: Value) -> Result<FilterDef, ApiError> {
    let a = v.pointer("/data/attributes").cloned().unwrap_or(v);
    let d: FilterDef = serde_json::from_value(a).map_err(|e| ApiError::BadQuery(e.to_string()))?;
    validate(&d).map_err(ApiError::BadQuery)?;
    Ok(d)
}

/// `GET /api/v2/apm/config/retention-filters`.
pub async fn list(State(st): State<AppState>) -> ApiResult<Value> {
    let defs = stored(&st).await?;
    let data: Vec<Value> = defs
        .iter()
        .enumerate()
        .map(|(i, (id, d, c, u))| {
            let mut v = out(id, d, *c, *u);
            v["attributes"]["execution_order"] = json!(i + 1);
            v
        })
        .collect();
    Ok(Json(json!({"data": data})))
}

pub async fn get(State(st): State<AppState>, Path(id): Path<String>) -> ApiResult<Value> {
    let defs = stored(&st).await?;
    let (id, d, c, u) = defs
        .iter()
        .find(|(i, ..)| *i == id)
        .ok_or(ApiError::NotFound)?;
    Ok(Json(json!({"data": out(id, d, *c, *u)})))
}

pub async fn create(State(st): State<AppState>, Json(v): Json<Value>) -> ApiResult<Value> {
    let mut d = body(v)?;
    d.filter_type = CUSTOM_TYPE.into();
    let id = crate::store::pg::new_id();
    let doc = st
        .store
        .pg
        .put_doc(
            KIND,
            &id,
            &serde_json::to_value(&d).map_err(anyhow::Error::from)?,
        )
        .await?;
    // New filters run before the others, as in Datadog.
    let mut ids: Vec<String> = stored(&st)
        .await?
        .into_iter()
        .map(|(i, ..)| i)
        .filter(|i| *i != id)
        .collect();
    ids.insert(0, id.clone());
    set_order(&st, &ids).await?;
    reload(&st).await?;
    Ok(Json(
        json!({"data": out(&id, &d, doc.created_at, doc.updated_at)}),
    ))
}

pub async fn update(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Json(v): Json<Value>,
) -> ApiResult<Value> {
    let old = st
        .store
        .pg
        .doc(KIND, &id)
        .await?
        .ok_or(ApiError::NotFound)?;
    let old: FilterDef = serde_json::from_value(old.body).map_err(anyhow::Error::from)?;
    let mut d = body(v)?;
    d.filter_type = old.filter_type.clone();
    if old.filter_type == ERRORS_TYPE {
        // Datadog's error filter: only its rate and state change.
        d.filter = old.filter;
    }
    let doc = st
        .store
        .pg
        .put_doc(
            KIND,
            &id,
            &serde_json::to_value(&d).map_err(anyhow::Error::from)?,
        )
        .await?;
    reload(&st).await?;
    Ok(Json(
        json!({"data": out(&id, &d, doc.created_at, doc.updated_at)}),
    ))
}

pub async fn delete(State(st): State<AppState>, Path(id): Path<String>) -> ApiResult<Value> {
    let old = st
        .store
        .pg
        .doc(KIND, &id)
        .await?
        .ok_or(ApiError::NotFound)?;
    if old.body["filter_type"] == ERRORS_TYPE {
        return Err(ApiError::BadQuery(
            "the error filter cannot be deleted: disable it instead".into(),
        ));
    }
    st.store.pg.delete_doc(KIND, &id).await?;
    reload(&st).await?;
    Ok(Json(json!({})))
}

/// `PUT /api/v2/apm/config/retention-filters-execution-order`.
pub async fn reorder(State(st): State<AppState>, Json(v): Json<Value>) -> ApiResult<Value> {
    let ids: Vec<String> = v["data"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| x["id"].as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let mut known: Vec<String> = stored(&st).await?.into_iter().map(|(i, ..)| i).collect();
    let mut wanted = ids.clone();
    known.sort();
    wanted.sort();
    wanted.dedup();
    if known != wanted || wanted.len() != ids.len() {
        return Err(ApiError::BadQuery(
            "data lists every retention filter once".into(),
        ));
    }
    set_order(&st, &ids).await?;
    reload(&st).await?;
    Ok(Json(json!({})))
}

/// `GET /api/ui/apm/ingestion`: Ingestion Control.
pub async fn ingestion(State(st): State<AppState>) -> ApiResult<Value> {
    let now = crate::intake::logs::now_ms();
    Ok(Json(json!({"services": st.catalog.retention.usage(now)})))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span(trace: &str, service: &str, error: bool) -> SpanRow {
        SpanRow {
            trace_id: trace.into(),
            span_id: "1".into(),
            parent_id: "0".into(),
            service: service.into(),
            name: "web.request".into(),
            resource: "GET /".into(),
            typ: None,
            env: None,
            version: None,
            start: 0,
            duration: 10,
            error,
            top_level: true,
            meta: Default::default(),
            metrics: Default::default(),
        }
    }

    fn def(name: &str, q: &str, rate: f64) -> (String, FilterDef) {
        (
            name.into(),
            FilterDef {
                name: name.into(),
                filter: Query { query: q.into() },
                rate,
                enabled: true,
                filter_type: CUSTOM_TYPE.into(),
                trace_rate: None,
            },
        )
    }

    #[test]
    fn filters_keep_whole_traces_by_rate() {
        let r = Retention::default();
        assert!(
            r.keep(&[span("1", "a", false)]).is_some(),
            "kept before loading"
        );
        r.load(&[
            def("errors", "status:error", 1.0),
            def("shop", "service:shop", 0.25),
        ]);
        assert_eq!(r.keep(&[span("1", "a", true)]).as_deref(), Some("errors"));
        assert_eq!(r.keep(&[span("1", "a", false)]), None);
        let kept = (0..4000)
            .filter(|i| r.keep(&[span(&i.to_string(), "shop", false)]).is_some())
            .count();
        assert!((850..1150).contains(&kept), "{kept}");
        let kept = (0..4000)
            .filter(|i| r.keep(&[span(&format!("t-{i}"), "shop", false)]).is_some())
            .count();
        assert!((850..1150).contains(&kept), "{kept}");
        // Decided on the trace id: always the same.
        let t = span("12345", "shop", false);
        assert_eq!(r.keep(std::slice::from_ref(&t)), r.keep(&[t]));
    }

    #[test]
    fn usage_counts_the_last_hour() {
        let r = Retention::default();
        let agg = Aggregator::default();
        let now = 10 * 3_600_000;
        r.record(
            &agg,
            &[span("1", "a", false), span("1", "b", false)],
            true,
            now - 2 * 3_600_000,
        );
        r.record(&agg, &[span("2", "a", false)], true, now);
        r.record(&agg, &[span("3", "a", false)], false, now);
        let u = r.usage(now);
        let a = u.iter().find(|v| v["service"] == "a").unwrap();
        assert_eq!(a["last_hour"]["spans"], 2);
        assert_eq!(a["last_hour"]["kept_spans"], 1);
        assert_eq!(a["last_hour"]["traces"], 2);
        assert_eq!(a["mechanisms"]["none"], 3);
    }
}
