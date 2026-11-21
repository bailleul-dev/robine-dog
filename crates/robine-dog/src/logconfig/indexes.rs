//! Log indexes, as Datadog's: a log goes to the first index whose filter
//! matches it (`/api/v1/logs/config/index-order`), unless one of the index's
//! exclusion filters drops it (all of what it matches, or a sample), or the
//! index has used its daily quota. Kept logs carry `index:<name>`. Excluded
//! logs are still archived and still counted by log-based metrics, as in
//! Datadog. With no index defined, every log is kept.
//!
//! GreptimeDB has no delete on the logs table: an index's shorter retention
//! (`num_retention_days`) hides its older logs from queries instead.

use crate::AppState;
use crate::api::{ApiError, ApiResult};
use crate::intake::logs::{LogRow, now_ms};
use crate::query::matcher::matches;
use crate::query::parser::Expr;
use axum::Json;
use axum::extract::{Path, State};
use rand::Rng;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex, RwLock};

pub const KIND: &str = "log_index";
pub const ORDER_KIND: &str = "log_index_order";
const DAY: i64 = 86_400_000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Filter {
    #[serde(default)]
    pub query: String,
    /// Exclusion filters: the share of matching logs dropped (1.0: all).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sample_rate: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Exclusion {
    pub name: String,
    #[serde(default)]
    pub is_enabled: bool,
    pub filter: Filter,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Reset {
    /// `HH:MM`.
    pub reset_time: String,
    /// `+HH:MM` or `-HH:MM`.
    #[serde(default = "utc")]
    pub reset_utc_offset: String,
}

fn utc() -> String {
    "+00:00".into()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct IndexDef {
    pub name: String,
    pub filter: Filter,
    #[serde(default)]
    pub exclusion_filters: Vec<Exclusion>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub daily_limit: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub daily_limit_reset: Option<Reset>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub daily_limit_warning_threshold_percentage: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub num_retention_days: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub num_flex_logs_retention_days: Option<i64>,
}

fn minutes(s: &str) -> Option<i64> {
    let (h, m) = s.trim().split_once(':')?;
    let (h, m): (i64, i64) = (h.parse().ok()?, m.parse().ok()?);
    ((0..24).contains(&h) && (0..60).contains(&m)).then_some(h * 60 + m)
}

fn offset_minutes(s: &str) -> Option<i64> {
    let s = s.trim();
    let (sign, rest) = match s.chars().next()? {
        '+' => (1, &s[1..]),
        '-' => (-1, &s[1..]),
        _ => (1, s),
    };
    let (h, m) = rest.split_once(':').unwrap_or((rest, "0"));
    let (h, m): (i64, i64) = (h.parse().ok()?, m.parse().ok()?);
    (h <= 14 && m < 60).then_some(sign * (h * 60 + m))
}

/// When the current daily quota window started: the last reset time.
pub fn window_start(now: i64, reset: Option<&Reset>) -> i64 {
    let (at, off) = reset
        .map(|r| {
            (
                minutes(&r.reset_time).unwrap_or(0) * 60_000,
                offset_minutes(&r.reset_utc_offset).unwrap_or(0) * 60_000,
            )
        })
        .unwrap_or((0, 0));
    let local = now + off;
    let mut start = local.div_euclid(DAY) * DAY + at;
    if start > local {
        start -= DAY;
    }
    start - off
}

pub fn validate(d: &IndexDef) -> Result<(), String> {
    if d.name.is_empty()
        || !d
            .name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err("the name is letters, digits, - and _".into());
    }
    crate::query::parser::parse(&d.filter.query).map_err(|e| format!("filter: {}", e.0))?;
    for x in &d.exclusion_filters {
        crate::query::parser::parse(&x.filter.query)
            .map_err(|e| format!("exclusion filter {}: {}", x.name, e.0))?;
        if let Some(r) = x.filter.sample_rate
            && !(0.0..=1.0).contains(&r)
        {
            return Err(format!(
                "exclusion filter {}: sample_rate is between 0 and 1",
                x.name
            ));
        }
    }
    if let Some(r) = &d.daily_limit_reset {
        minutes(&r.reset_time).ok_or("daily_limit_reset.reset_time is HH:MM")?;
        offset_minutes(&r.reset_utc_offset)
            .ok_or("daily_limit_reset.reset_utc_offset is +HH:MM")?;
    }
    if d.daily_limit.is_some_and(|l| l < 0) || d.num_retention_days.is_some_and(|r| r < 1) {
        return Err("daily_limit and num_retention_days are positive".into());
    }
    Ok(())
}

struct Compiled {
    def: IndexDef,
    filter: Expr,
    exclusions: Vec<(Expr, f64)>,
}

/// Per index, in the current quota window.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Usage {
    pub window_start: i64,
    pub indexed: i64,
    pub excluded: i64,
    /// Dropped past the daily quota.
    pub over_quota: i64,
}

#[derive(Default)]
pub struct Indexes {
    list: RwLock<Arc<Vec<Compiled>>>,
    usage: Mutex<HashMap<String, Usage>>,
}

pub enum Route {
    /// No index defined: kept as is.
    Keep,
    Index(String),
    Drop,
}

/// Each index's retention when shorter than the logs table's, for queries.
static RETENTION: LazyLock<RwLock<Vec<(String, i64)>>> = LazyLock::new(Default::default);

/// A `WHERE` fragment hiding the logs past their index's retention, if any.
pub fn retention_clause() -> Option<String> {
    let r = RETENTION.read().unwrap_or_else(|e| e.into_inner());
    if r.is_empty() {
        return None;
    }
    let now = now_ms();
    Some(
        r.iter()
            .map(|(name, days)| {
                format!(
                    "NOT (CAST(ts AS BIGINT) < {} AND tags LIKE '%,index:{},%')",
                    now - days * DAY,
                    name.replace('\'', "")
                )
            })
            .collect::<Vec<_>>()
            .join(" AND "),
    )
}

impl Indexes {
    pub fn load(&self, defs: Vec<IndexDef>, table_retention_days: i64) {
        let list: Vec<Compiled> = defs
            .into_iter()
            .filter_map(|def| {
                let filter = crate::query::parser::parse(&def.filter.query).ok()?;
                let exclusions = def
                    .exclusion_filters
                    .iter()
                    .filter(|x| x.is_enabled)
                    .filter_map(|x| {
                        Some((
                            crate::query::parser::parse(&x.filter.query).ok()?,
                            x.filter.sample_rate.unwrap_or(1.0),
                        ))
                    })
                    .collect();
                Some(Compiled {
                    def,
                    filter,
                    exclusions,
                })
            })
            .collect();
        *RETENTION.write().unwrap_or_else(|e| e.into_inner()) = list
            .iter()
            .filter_map(|c| {
                c.def
                    .num_retention_days
                    .filter(|d| *d < table_retention_days)
                    .map(|d| (c.def.name.clone(), d))
            })
            .collect();
        *self.list.write().unwrap_or_else(|e| e.into_inner()) = Arc::new(list);
    }

    pub fn usage(&self, name: &str) -> Usage {
        self.usage
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(name)
            .cloned()
            .unwrap_or_default()
    }

    /// Restores a window's count (after a restart).
    pub fn set_indexed(&self, name: &str, window_start: i64, indexed: i64) {
        let mut u = self.usage.lock().unwrap_or_else(|e| e.into_inner());
        let e = u.entry(name.to_string()).or_default();
        if e.window_start < window_start || e.indexed < indexed {
            *e = Usage {
                window_start,
                indexed,
                ..Default::default()
            };
        }
    }

    pub fn route(&self, log: &LogRow, now: i64) -> Route {
        let list = self.list.read().unwrap_or_else(|e| e.into_inner()).clone();
        if list.is_empty() {
            return Route::Keep;
        }
        let Some(ix) = list.iter().find(|c| matches(&c.filter, log)) else {
            return Route::Drop;
        };
        let name = &ix.def.name;
        let start = window_start(now, ix.def.daily_limit_reset.as_ref());
        let mut usage = self.usage.lock().unwrap_or_else(|e| e.into_inner());
        let u = usage.entry(name.clone()).or_default();
        if u.window_start != start {
            *u = Usage {
                window_start: start,
                ..Default::default()
            };
        }
        if let Some((_, rate)) = ix.exclusions.iter().find(|(f, _)| matches(f, log))
            && (*rate >= 1.0 || rand::rng().random_bool(rate.clamp(0.0, 1.0)))
        {
            u.excluded += 1;
            return Route::Drop;
        }
        if ix.def.daily_limit.is_some_and(|l| u.indexed >= l) {
            u.over_quota += 1;
            return Route::Drop;
        }
        u.indexed += 1;
        Route::Index(name.clone())
    }
}

// --- API: `/api/v1/logs/config/indexes` -------------------------------------

async fn stored(st: &AppState) -> anyhow::Result<Vec<IndexDef>> {
    let mut defs: Vec<IndexDef> = st
        .store
        .pg
        .docs(KIND)
        .await?
        .into_iter()
        .filter_map(|d| serde_json::from_value(d.body).ok())
        .collect();
    let order = order(st).await?;
    defs.sort_by_key(|d| {
        order
            .iter()
            .position(|o| *o == d.name)
            .unwrap_or(usize::MAX)
    });
    Ok(defs)
}

async fn order(st: &AppState) -> anyhow::Result<Vec<String>> {
    Ok(st
        .store
        .pg
        .doc(ORDER_KIND, "order")
        .await?
        .and_then(|d| serde_json::from_value(d.body["index_names"].clone()).ok())
        .unwrap_or_default())
}

async fn set_order_names(st: &AppState, names: &[String]) -> anyhow::Result<()> {
    st.store
        .pg
        .put_doc(ORDER_KIND, "order", &json!({"index_names": names}))
        .await?;
    Ok(())
}

/// Creates the `main` index on a first start, and loads the indexes.
pub async fn init(st: &AppState) -> anyhow::Result<()> {
    if st.store.pg.docs(KIND).await?.is_empty()
        && st.store.pg.doc(ORDER_KIND, "order").await?.is_none()
    {
        let main = IndexDef {
            name: "main".into(),
            filter: Filter {
                query: String::new(),
                sample_rate: None,
            },
            exclusion_filters: vec![],
            daily_limit: None,
            daily_limit_reset: None,
            daily_limit_warning_threshold_percentage: None,
            num_retention_days: Some(st.cfg.retention_days),
            num_flex_logs_retention_days: None,
        };
        st.store
            .pg
            .put_doc(KIND, "main", &serde_json::to_value(&main)?)
            .await?;
        set_order_names(st, &["main".to_string()]).await?;
    }
    reload(st).await?;
    // The quota windows, counted again after a restart.
    let now = now_ms();
    for d in stored(st)
        .await?
        .into_iter()
        .filter(|d| d.daily_limit.is_some())
    {
        let start = window_start(now, d.daily_limit_reset.as_ref());
        let sql = format!(
            "SELECT count(*) FROM logs WHERE ts >= {start} AND tags LIKE '%,index:{},%'",
            d.name
        );
        if let Ok(rows) = st.store.gt.query(&sql).await {
            let n = rows
                .first()
                .map_or(0, |r| crate::store::greptime::i64_at(r, 0));
            st.logconfig.indexes.set_indexed(&d.name, start, n);
        }
    }
    Ok(())
}

pub async fn reload(st: &AppState) -> anyhow::Result<()> {
    st.logconfig
        .indexes
        .load(stored(st).await?, st.cfg.retention_days);
    Ok(())
}

fn out(st: &AppState, d: &IndexDef) -> Value {
    let mut v = serde_json::to_value(d).unwrap_or_default();
    let u = st.logconfig.indexes.usage(&d.name);
    let now = now_ms();
    let current = u.window_start == window_start(now, d.daily_limit_reset.as_ref());
    v["is_rate_limited"] = json!(current && d.daily_limit.is_some_and(|l| u.indexed >= l));
    if v.get("num_retention_days").is_none() {
        v["num_retention_days"] = json!(st.cfg.retention_days);
    }
    v["usage"] = if current {
        json!(u)
    } else {
        json!(Usage::default())
    };
    v
}

/// `GET /api/v1/logs/config/indexes`.
pub async fn list(State(st): State<AppState>) -> ApiResult<Value> {
    let defs = stored(&st).await?;
    Ok(Json(
        json!({"indexes": defs.iter().map(|d| out(&st, d)).collect::<Vec<_>>()}),
    ))
}

/// `GET /api/v1/logs/config/indexes/{name}`.
pub async fn get(State(st): State<AppState>, Path(name): Path<String>) -> ApiResult<Value> {
    let d = stored(&st)
        .await?
        .into_iter()
        .find(|d| d.name == name)
        .ok_or(ApiError::NotFound)?;
    Ok(Json(out(&st, &d)))
}

async fn save(st: &AppState, d: &IndexDef) -> Result<(), ApiError> {
    validate(d).map_err(ApiError::BadQuery)?;
    st.store
        .pg
        .put_doc(
            KIND,
            &d.name,
            &serde_json::to_value(d).map_err(anyhow::Error::from)?,
        )
        .await?;
    Ok(())
}

/// `POST /api/v1/logs/config/indexes`: added last in the order.
pub async fn create(State(st): State<AppState>, Json(d): Json<IndexDef>) -> ApiResult<Value> {
    if stored(&st).await?.iter().any(|x| x.name == d.name) {
        return Err(ApiError::BadQuery(format!("an index {} exists", d.name)));
    }
    save(&st, &d).await?;
    let mut o = order(&st).await?;
    o.retain(|n| *n != d.name);
    o.push(d.name.clone());
    set_order_names(&st, &o).await?;
    reload(&st).await?;
    Ok(Json(out(&st, &d)))
}

/// `PUT /api/v1/logs/config/indexes/{name}`.
pub async fn update(
    State(st): State<AppState>,
    Path(name): Path<String>,
    Json(mut d): Json<Value>,
) -> ApiResult<Value> {
    if !stored(&st).await?.iter().any(|x| x.name == name) {
        return Err(ApiError::NotFound);
    }
    d["name"] = json!(name);
    let d: IndexDef = serde_json::from_value(d).map_err(|e| ApiError::BadQuery(e.to_string()))?;
    save(&st, &d).await?;
    reload(&st).await?;
    Ok(Json(out(&st, &d)))
}

/// `DELETE /api/v1/logs/config/indexes/{name}`.
pub async fn delete(State(st): State<AppState>, Path(name): Path<String>) -> ApiResult<Value> {
    if !st.store.pg.delete_doc(KIND, &name).await? {
        return Err(ApiError::NotFound);
    }
    let mut o = order(&st).await?;
    o.retain(|n| *n != name);
    set_order_names(&st, &o).await?;
    reload(&st).await?;
    Ok(Json(json!({})))
}

/// `GET /api/v1/logs/config/index-order`.
pub async fn get_order(State(st): State<AppState>) -> ApiResult<Value> {
    let names: Vec<String> = stored(&st).await?.into_iter().map(|d| d.name).collect();
    Ok(Json(json!({"index_names": names})))
}

#[derive(Deserialize)]
pub struct Order {
    index_names: Vec<String>,
}

/// `PUT /api/v1/logs/config/index-order`: every index, in the new order.
pub async fn set_order(State(st): State<AppState>, Json(o): Json<Order>) -> ApiResult<Value> {
    let names: Vec<String> = stored(&st).await?.into_iter().map(|d| d.name).collect();
    let mut a = o.index_names.clone();
    let mut b = names.clone();
    a.sort();
    b.sort();
    if a != b {
        return Err(ApiError::BadQuery(
            "index_names lists every index once".into(),
        ));
    }
    set_order_names(&st, &o.index_names).await?;
    reload(&st).await?;
    Ok(Json(json!({"index_names": o.index_names})))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn log(service: &str, status: &str) -> LogRow {
        LogRow {
            ts: 0,
            status: status.into(),
            service: Some(service.into()),
            host: None,
            source: None,
            message: "m".into(),
            tags: vec![],
            attributes: Default::default(),
            trace_id: None,
            span_id: None,
        }
    }

    fn index(name: &str, query: &str, excl: &[(&str, f64)], limit: Option<i64>) -> IndexDef {
        IndexDef {
            name: name.into(),
            filter: Filter {
                query: query.into(),
                sample_rate: None,
            },
            exclusion_filters: excl
                .iter()
                .map(|(q, r)| Exclusion {
                    name: q.to_string(),
                    is_enabled: true,
                    filter: Filter {
                        query: q.to_string(),
                        sample_rate: Some(*r),
                    },
                })
                .collect(),
            daily_limit: limit,
            daily_limit_reset: None,
            daily_limit_warning_threshold_percentage: None,
            num_retention_days: Some(3),
            num_flex_logs_retention_days: None,
        }
    }

    #[test]
    fn routes_excludes_and_limits() {
        let ix = Indexes::default();
        assert!(matches!(ix.route(&log("a", "info"), 0), Route::Keep));
        ix.load(
            vec![
                index("payments", "service:pay", &[("status:debug", 1.0)], Some(2)),
                index("main", "*", &[("status:info", 0.0)], None),
                index("never", "*", &[], None),
            ],
            15,
        );
        let name = |r: Route| match r {
            Route::Index(n) => n,
            Route::Keep => "keep".into(),
            Route::Drop => "drop".into(),
        };
        let now = 10 * DAY + 5;
        assert_eq!(name(ix.route(&log("pay", "debug"), now)), "drop");
        assert_eq!(name(ix.route(&log("pay", "info"), now)), "payments");
        assert_eq!(name(ix.route(&log("pay", "info"), now)), "payments");
        // Over the daily quota, until the next window.
        assert_eq!(name(ix.route(&log("pay", "info"), now)), "drop");
        assert_eq!(name(ix.route(&log("pay", "info"), now + DAY)), "payments");
        // A sample rate of 0 excludes nothing; the first match wins.
        assert_eq!(name(ix.route(&log("web", "info"), now)), "main");
        let u = ix.usage("payments");
        assert_eq!((u.indexed, u.excluded, u.over_quota), (1, 0, 0));
        assert!(retention_clause().unwrap().contains("index:payments"));
    }

    #[test]
    fn quota_windows() {
        let r = Reset {
            reset_time: "14:00".into(),
            reset_utc_offset: "+02:00".into(),
        };
        // 12:00 UTC is 14:00 at +02:00.
        let day = 20 * DAY;
        assert_eq!(
            window_start(day + 12 * 3_600_000, Some(&r)),
            day + 12 * 3_600_000
        );
        assert_eq!(
            window_start(day + 11 * 3_600_000, Some(&r)),
            day - DAY + 12 * 3_600_000
        );
        assert_eq!(window_start(day + 5, None), day);
        let mut bad = index("x y", "*", &[], None);
        assert!(validate(&bad).is_err());
        bad.name = "ok".into();
        bad.exclusion_filters.push(Exclusion {
            name: "e".into(),
            is_enabled: true,
            filter: Filter {
                query: "*".into(),
                sample_rate: Some(2.0),
            },
        });
        assert!(validate(&bad).is_err());
    }
}
