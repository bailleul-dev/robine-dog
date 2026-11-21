//! Service level objectives as in Datadog's v1 API: metric-based (good and
//! total events), monitor-based (time out of `Alert`) and time-slice (slices
//! whose value meets a condition), with their status, history, search and
//! corrections. The computation is in `crate::slo`.

use super::{ApiError, ApiResult};
use crate::AppState;
use crate::intake::logs::now_ms;
use crate::slo::{self, CORRECTION_KIND, Correction, KIND, Slo, Threshold};
use crate::store::pg::new_id;
use axum::Json;
use axum::extract::{Path, Query, State};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::Ordering;

fn bad(m: impl Into<String>) -> ApiError {
    ApiError::BadQuery(m.into())
}

async fn check(st: &AppState, mut s: Slo) -> Result<Slo, ApiError> {
    s.name = s.name.trim().to_string();
    if s.name.is_empty() {
        return Err(bad("an SLO needs a name"));
    }
    if s.typ.is_empty() {
        s.typ = if !s.monitor_ids.is_empty() {
            "monitor"
        } else if s.sli_specification.is_some() {
            "time_slice"
        } else {
            "metric"
        }
        .into();
    }
    match s.typ.as_str() {
        "metric" => {
            let q = s
                .query
                .as_ref()
                .ok_or_else(|| bad("a metric SLO needs a numerator and a denominator"))?;
            for term in q.numerator.split(" - ").chain(q.denominator.split(" - ")) {
                crate::metrics::query::parse(term.trim())
                    .map_err(|e| bad(format!("{}: {e}", term.trim())))?;
            }
            s.monitor_ids.clear();
            s.sli_specification = None;
        }
        "monitor" => {
            if s.monitor_ids.is_empty() {
                return Err(bad("a monitor SLO needs monitors"));
            }
            for id in &s.monitor_ids {
                if st
                    .store
                    .pg
                    .doc(crate::monitors::KIND, &id.to_string())
                    .await?
                    .is_none()
                {
                    return Err(bad(format!("no monitor {id}")));
                }
            }
            s.query = None;
            s.sli_specification = None;
        }
        "time_slice" => {
            let spec = &s
                .sli_specification
                .as_ref()
                .ok_or_else(|| bad("a time-slice SLO needs sli_specification.time_slice"))?
                .time_slice;
            if !matches!(spec.comparator.as_str(), ">" | ">=" | "<" | "<=") {
                return Err(bad("the comparator is >, >=, < or <="));
            }
            if spec
                .query_interval_seconds
                .is_some_and(|i| i != 60 && i != 300)
            {
                return Err(bad("query_interval_seconds is 60 or 300"));
            }
            if spec.query.queries.is_empty() || spec.query.formulas.is_empty() {
                return Err(bad("a time-slice SLO needs a query and a formula"));
            }
            for q in &spec.query.queries {
                if q.data_source == "metrics" {
                    crate::metrics::query::parse(&q.query)
                        .map_err(|e| bad(format!("{}: {e}", q.name)))?;
                }
            }
            for f in &spec.query.formulas {
                crate::api::query::parse_formula(&f.formula).map_err(bad)?;
            }
            s.query = None;
            s.monitor_ids.clear();
        }
        other => return Err(bad(format!("unknown SLO type {other}"))),
    }
    if s.thresholds.is_empty()
        && let (Some(tf), Some(target)) = (&s.timeframe, s.target_threshold)
    {
        s.thresholds.push(Threshold {
            timeframe: tf.clone(),
            target,
            warning: s.warning_threshold,
        });
    }
    if s.thresholds.is_empty() {
        return Err(bad("an SLO needs a target"));
    }
    for t in &s.thresholds {
        if slo::timeframe_ms(&t.timeframe).is_none() {
            return Err(bad("time frames are 7d, 30d or 90d"));
        }
        if !(t.target > 0.0 && t.target < 100.0) {
            return Err(bad("a target is between 0 and 100 %"));
        }
        if t.warning.is_some_and(|w| w <= t.target || w >= 100.0) {
            return Err(bad("a warning is above the target, below 100 %"));
        }
    }
    let first = &s.thresholds[0];
    s.timeframe = Some(first.timeframe.clone());
    s.target_threshold = Some(first.target);
    s.warning_threshold = first.warning;
    Ok(s)
}

async fn save(st: &AppState, s: &Slo) -> Result<(), ApiError> {
    let id = s.id.clone().unwrap_or_default();
    st.store
        .pg
        .put_doc(
            KIND,
            &id,
            &serde_json::to_value(s).map_err(anyhow::Error::from)?,
        )
        .await?;
    Ok(())
}

async fn find(st: &AppState, id: &str) -> Result<Slo, ApiError> {
    slo::get(st, id).await?.ok_or(ApiError::NotFound)
}

#[derive(Deserialize, Default)]
pub struct ListParams {
    ids: Option<String>,
    query: Option<String>,
    tags_query: Option<String>,
    limit: Option<usize>,
    offset: Option<usize>,
}

pub async fn list(State(st): State<AppState>, Query(p): Query<ListParams>) -> ApiResult<Value> {
    let ids: Vec<&str> = p
        .ids
        .as_deref()
        .map(|s| s.split(',').map(str::trim).collect())
        .unwrap_or_default();
    let tags: Vec<&str> = p
        .tags_query
        .as_deref()
        .map(|s| {
            s.split(',')
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .collect()
        })
        .unwrap_or_default();
    let name = p.query.as_deref().map(str::to_lowercase);
    let mut all: Vec<Slo> = slo::all(&st)
        .await?
        .into_iter()
        .filter(|s| ids.is_empty() || ids.contains(&s.id.as_deref().unwrap_or("")))
        .filter(|s| tags.iter().all(|t| s.tags.iter().any(|x| x == t)))
        .filter(|s| {
            name.as_ref()
                .is_none_or(|n| s.name.to_lowercase().contains(n))
        })
        .collect();
    all.sort_by_key(|s| s.name.to_lowercase());
    let all: Vec<Slo> = all
        .into_iter()
        .skip(p.offset.unwrap_or(0))
        .take(p.limit.unwrap_or(1000))
        .collect();
    Ok(Json(json!({"data": all, "errors": []})))
}

/// Each time frame of an SLO, now.
async fn statuses(st: &AppState, s: &Slo) -> Vec<Value> {
    let now = now_ms();
    let mut out = Vec::new();
    for t in &s.thresholds {
        out.push(match slo::status(st, s, &t.timeframe, now).await {
            Ok(v) => serde_json::to_value(v).unwrap_or_default(),
            Err(e) => json!({"timeframe": t.timeframe, "target": t.target, "state": "no_data",
                             "error": error_text(&e)}),
        });
    }
    out
}

fn error_text(e: &ApiError) -> String {
    match e {
        ApiError::BadQuery(m) => m.clone(),
        ApiError::NotFound => "not found".into(),
        ApiError::Internal(e) => e.to_string(),
    }
}

/// The burn-rate and error-budget monitors of an SLO.
async fn alert_ids(st: &AppState, id: &str) -> Result<Vec<i64>, ApiError> {
    Ok(st
        .store
        .pg
        .docs(crate::monitors::KIND)
        .await?
        .into_iter()
        .filter(|d| {
            d.body["query"].as_str().is_some_and(|q| {
                crate::monitors::parse(q).is_ok_and(|p| {
                    matches!(&p.source, crate::monitors::Source::Slo { slo_id, .. } if slo_id == id)
                })
            })
        })
        .filter_map(|d| d.id.parse().ok())
        .collect())
}

pub async fn get(State(st): State<AppState>, Path(id): Path<String>) -> ApiResult<Value> {
    let s = find(&st, &id).await?;
    let mut v = serde_json::to_value(&s).map_err(anyhow::Error::from)?;
    v["overall_status"] = json!(statuses(&st, &s).await);
    v["configured_alert_ids"] = json!(alert_ids(&st, &id).await?);
    Ok(Json(json!({"data": v, "errors": []})))
}

pub async fn create(State(st): State<AppState>, Json(s): Json<Slo>) -> ApiResult<Value> {
    let mut s = check(&st, s).await?;
    let now = now_ms() / 1000;
    s.id = Some(new_id());
    s.created_at = Some(now);
    s.modified_at = Some(now);
    save(&st, &s).await?;
    Ok(Json(json!({"data": [s], "errors": []})))
}

pub async fn update(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Json(s): Json<Slo>,
) -> ApiResult<Value> {
    let old = find(&st, &id).await?;
    let mut s = check(&st, s).await?;
    s.id = Some(id);
    s.created_at = old.created_at;
    s.modified_at = Some(now_ms() / 1000);
    save(&st, &s).await?;
    Ok(Json(json!({"data": [s], "errors": []})))
}

#[derive(Deserialize)]
pub struct Force {
    force: Option<String>,
}

pub async fn delete(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Query(f): Query<Force>,
) -> ApiResult<Value> {
    find(&st, &id).await?;
    let alerts = alert_ids(&st, &id).await?;
    if !alerts.is_empty() && f.force.as_deref() != Some("true") {
        return Err(bad(format!(
            "the SLO is used by monitors {alerts:?}; delete them first, or pass force=true"
        )));
    }
    st.store.pg.delete_doc(KIND, &id).await?;
    for c in slo::corrections(&st, Some(&id)).await? {
        if let Some(cid) = c.id {
            st.store.pg.delete_doc(CORRECTION_KIND, &cid).await?;
        }
    }
    Ok(Json(json!({"data": [id], "errors": []})))
}

#[derive(Deserialize)]
pub struct Ids {
    ids: String,
}

/// Which SLOs can go: not those monitors are built on.
pub async fn can_delete(State(st): State<AppState>, Query(q): Query<Ids>) -> ApiResult<Value> {
    let mut ok = Vec::new();
    let mut errors = Map::new();
    for id in q.ids.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        let alerts = alert_ids(&st, id).await?;
        if alerts.is_empty() {
            ok.push(id.to_string());
        } else {
            errors.insert(id.into(), json!(format!("used by monitors {alerts:?}")));
        }
    }
    Ok(Json(json!({"data": {"ok": ok}, "errors": errors})))
}

// --- history ---------------------------------------------------------------------

#[derive(Deserialize)]
pub struct HistoryParams {
    /// Epoch seconds (Datadog's API).
    from_ts: Option<i64>,
    to_ts: Option<i64>,
    /// Epoch ms (the UI).
    from: Option<i64>,
    to: Option<i64>,
    target: Option<f64>,
}

impl HistoryParams {
    fn range(&self) -> Result<(i64, i64), ApiError> {
        let now = now_ms();
        let to = self
            .to
            .or(self.to_ts.map(|t| t * 1000))
            .unwrap_or(now)
            .min(now);
        let from = self
            .from
            .or(self.from_ts.map(|t| t * 1000))
            .unwrap_or(to - 7 * 86_400_000);
        if from >= to {
            return Err(bad("from is after to"));
        }
        if to - from > 90 * 86_400_000 + 3_600_000 {
            return Err(bad("the history spans 90 days at most"));
        }
        Ok((from, to))
    }
}

pub async fn history(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Query(p): Query<HistoryParams>,
) -> ApiResult<Value> {
    let s = find(&st, &id).await?;
    let (from, to) = p.range()?;
    let b = slo::compute(&st, &s, from, to, (to - from) / 300).await?;
    let sli = b.sums();
    let mut thresholds = Map::new();
    for t in &s.thresholds {
        thresholds.insert(
            t.timeframe.clone(),
            json!({"timeframe": t.timeframe, "target": t.target, "target_display": format!("{}", t.target),
                   "warning": t.warning}),
        );
    }
    let target = p
        .target
        .or(s.threshold(None).map(|t| t.target))
        .unwrap_or(99.9);
    let corrections: Vec<Value> = slo::corrections(&st, Some(&id))
        .await?
        .iter()
        .flat_map(|c| c.windows(from, to))
        .map(|(a, z)| json!({"start": a / 1000, "end": z / 1000, "scope": ""}))
        .collect();
    let values = |v: &[f64]| json!({"sum": v.iter().sum::<f64>(), "count": v.len(), "values": v});
    let mut data = json!({
        "from_ts": from / 1000,
        "to_ts": to / 1000,
        "type": s.typ,
        "type_id": match s.typ.as_str() { "metric" => 1, "monitor" => 0, _ => 2 },
        "slo": s,
        "thresholds": thresholds,
        "overall": {
            "name": s.name,
            "sli_value": sli.value(),
            "span_precision": 2,
            "precision": {"custom": 2},
            "error_budget_remaining": {"custom": sli.budget_remaining(target)},
            "corrections": corrections,
        },
        "series": {
            "interval": b.step / 1000,
            "res_type": "time_series",
            "times": b.times(),
            "numerator": values(&b.good),
            "denominator": values(&b.total),
            "unit": s.unit(),
        },
    });
    if s.typ == "monitor" {
        let down = slo::downtime(&st, &s, to).await?;
        data["overall"]["history"] = json!(
            down.iter()
                .filter(|(a, z)| *z > from && *a < to)
                .map(|(a, z)| [a / 1000, z / 1000])
                .collect::<Vec<_>>()
        );
    }
    Ok(Json(json!({"data": data, "errors": null})))
}

// --- search ----------------------------------------------------------------------

#[derive(Deserialize)]
pub struct SearchParams {
    query: Option<String>,
    #[serde(rename = "page[size]")]
    size: Option<usize>,
    #[serde(rename = "page[number]")]
    number: Option<usize>,
    include_facets: Option<bool>,
}

/// Words of the name or description, `type:metric`, `tag:value` and
/// `service:x`-style tags; all must match.
fn matches(s: &Slo, query: &str) -> bool {
    let text = format!(
        "{} {}",
        s.name.to_lowercase(),
        s.description.as_deref().unwrap_or("").to_lowercase()
    );
    query.split_whitespace().all(|w| {
        let w = w.trim_matches('"');
        match w.split_once(':') {
            Some(("type", t)) => s.typ == t || (t == "time-slice" && s.typ == "time_slice"),
            Some(("tag", t)) => s.tags.iter().any(|x| x == t),
            Some(("id", t)) => s.id.as_deref() == Some(t),
            Some(_) => s.tags.iter().any(|x| x == w) || text.contains(&w.to_lowercase()),
            None => text.contains(&w.to_lowercase()),
        }
    })
}

pub async fn search(State(st): State<AppState>, Query(p): Query<SearchParams>) -> ApiResult<Value> {
    let query = p.query.unwrap_or_default();
    let mut found: Vec<Slo> = slo::all(&st)
        .await?
        .into_iter()
        .filter(|s| matches(s, &query))
        .collect();
    found.sort_by_key(|s| s.name.to_lowercase());
    let size = p.size.unwrap_or(25).clamp(1, 100);
    let number = p.number.unwrap_or(0);
    let total = found.len();
    let mut facets: BTreeMap<String, BTreeMap<String, usize>> = BTreeMap::new();
    for s in &found {
        *facets
            .entry("slo_type".into())
            .or_default()
            .entry(s.typ.clone())
            .or_default() += 1;
        for t in &s.tags {
            let (k, _) = t.split_once(':').unwrap_or((t, ""));
            *facets
                .entry(format!("all_tags.{k}"))
                .or_default()
                .entry(t.clone())
                .or_default() += 1;
        }
    }
    let mut slos = Vec::new();
    for s in found.into_iter().skip(number * size).take(size) {
        let overall = statuses(&st, &s).await;
        slos.push(json!({
            "data": {
                "id": s.id,
                "type": "slo",
                "attributes": {
                    "name": s.name,
                    "description": s.description,
                    "slo_type": s.typ,
                    "thresholds": s.thresholds,
                    "all_tags": s.tags,
                    "monitor_ids": s.monitor_ids,
                    "query": s.query,
                    "created_at": s.created_at,
                    "modified_at": s.modified_at,
                    "overall_status": overall,
                    "status": overall.first(),
                }
            }
        }));
    }
    let facets: Map<String, Value> = if p.include_facets.unwrap_or(false) {
        facets
            .into_iter()
            .map(|(k, v)| {
                (
                    k,
                    json!(
                        v.into_iter()
                            .map(|(name, count)| json!({"name": name, "count": count}))
                            .collect::<Vec<_>>()
                    ),
                )
            })
            .collect()
    } else {
        Map::new()
    };
    Ok(Json(json!({
        "data": {"type": "search_slo", "attributes": {"slos": slos, "facets": facets}},
        "meta": {"pagination": {
            "total": total, "size": size, "number": number,
            "first_number": 0, "last_number": total.saturating_sub(1) / size,
            "prev_number": number.saturating_sub(1), "next_number": number + 1,
            "type": "number_size",
        }},
        "links": {},
    })))
}

/// The UI's view: each time frame's status.
pub async fn status(State(st): State<AppState>, Path(id): Path<String>) -> ApiResult<Value> {
    let s = find(&st, &id).await?;
    Ok(Json(json!(statuses(&st, &s).await)))
}

// --- corrections -----------------------------------------------------------------

fn correction_json(c: &Correction) -> Value {
    let mut attrs = serde_json::to_value(c).unwrap_or_default();
    if let Some(m) = attrs.as_object_mut() {
        m.remove("id");
    }
    json!({"id": c.id, "type": "correction", "attributes": attrs})
}

async fn check_correction(st: &AppState, c: &Correction) -> Result<(), ApiError> {
    find(st, &c.slo_id).await?;
    if !slo::CATEGORIES.contains(&c.category.as_str()) {
        return Err(bad(format!(
            "the category is one of {}",
            slo::CATEGORIES.join(", ")
        )));
    }
    match &c.rrule {
        Some(r) if !r.trim().is_empty() => {
            slo::parse_rrule(r).map_err(bad)?;
            if c.duration.is_none_or(|d| d <= 0) {
                return Err(bad("a recurring correction needs a duration"));
            }
        }
        _ => {
            if c.end.is_none_or(|e| e <= c.start) && c.duration.is_none_or(|d| d <= 0) {
                return Err(bad("a correction needs an end after its start"));
            }
        }
    }
    if let Some(tz) = &c.timezone
        && tz.parse::<chrono_tz::Tz>().is_err()
    {
        return Err(bad(format!("unknown time zone {tz}")));
    }
    Ok(())
}

async fn save_correction(st: &AppState, c: &Correction) -> Result<(), ApiError> {
    st.store
        .pg
        .put_doc(
            CORRECTION_KIND,
            c.id.as_deref().unwrap_or_default(),
            &serde_json::to_value(c).map_err(anyhow::Error::from)?,
        )
        .await?;
    slo::CORRECTIONS_GENERATION.fetch_add(1, Ordering::Relaxed);
    Ok(())
}

#[derive(Deserialize)]
pub struct CorrectionBody {
    data: CorrectionData,
}

#[derive(Deserialize)]
pub struct CorrectionData {
    attributes: Value,
}

pub async fn list_corrections(
    State(st): State<AppState>,
    Query(q): Query<HashMap<String, String>>,
) -> ApiResult<Value> {
    let mut all = slo::corrections(&st, q.get("slo_id").map(String::as_str)).await?;
    all.sort_by_key(|c| std::cmp::Reverse(c.start));
    Ok(Json(json!({
        "data": all.iter().map(correction_json).collect::<Vec<_>>(),
        "meta": {"page": {"total_count": all.len(), "total_filtered_count": all.len()}},
    })))
}

pub async fn slo_corrections(
    State(st): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Value> {
    find(&st, &id).await?;
    let mut q = HashMap::new();
    q.insert("slo_id".to_string(), id);
    list_corrections(State(st), Query(q)).await
}

pub async fn create_correction(
    State(st): State<AppState>,
    Json(body): Json<CorrectionBody>,
) -> ApiResult<Value> {
    let mut c: Correction =
        serde_json::from_value(body.data.attributes).map_err(|e| bad(e.to_string()))?;
    check_correction(&st, &c).await?;
    c.id = Some(new_id());
    c.created_at = Some(now_ms() / 1000);
    save_correction(&st, &c).await?;
    Ok(Json(json!({"data": correction_json(&c)})))
}

async fn find_correction(st: &AppState, id: &str) -> Result<Correction, ApiError> {
    st.store
        .pg
        .doc(CORRECTION_KIND, id)
        .await?
        .and_then(|d| serde_json::from_value(d.body).ok())
        .ok_or(ApiError::NotFound)
}

pub async fn get_correction(
    State(st): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Value> {
    Ok(Json(
        json!({"data": correction_json(&find_correction(&st, &id).await?)}),
    ))
}

pub async fn update_correction(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<CorrectionBody>,
) -> ApiResult<Value> {
    let old = find_correction(&st, &id).await?;
    let mut merged = serde_json::to_value(&old).map_err(anyhow::Error::from)?;
    if let (Some(m), Some(patch)) = (merged.as_object_mut(), body.data.attributes.as_object()) {
        for (k, v) in patch {
            m.insert(k.clone(), v.clone());
        }
    }
    let mut c: Correction = serde_json::from_value(merged).map_err(|e| bad(e.to_string()))?;
    c.id = Some(id);
    c.created_at = old.created_at;
    check_correction(&st, &c).await?;
    save_correction(&st, &c).await?;
    Ok(Json(json!({"data": correction_json(&c)})))
}

pub async fn delete_correction(
    State(st): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Value> {
    if !st.store.pg.delete_doc(CORRECTION_KIND, &id).await? {
        return Err(ApiError::NotFound);
    }
    slo::CORRECTIONS_GENERATION.fetch_add(1, Ordering::Relaxed);
    Ok(Json(json!({})))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_syntax() {
        let s: Slo = serde_json::from_value(json!({
            "name": "Checkout availability", "type": "metric",
            "query": {"numerator": "a", "denominator": "b"},
            "thresholds": [], "tags": ["team:shop", "env:prod"]
        }))
        .unwrap();
        assert!(matches(&s, "checkout"));
        assert!(matches(&s, "type:metric team:shop"));
        assert!(matches(&s, "tag:env:prod avail"));
        assert!(!matches(&s, "type:monitor"));
        assert!(!matches(&s, "team:search"));
    }
}
