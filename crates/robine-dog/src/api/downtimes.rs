//! Downtimes: scheduled silences. While one is active, the monitors it
//! covers keep being evaluated but notify nobody. Same shape as Datadog's
//! v1 API (`/api/v1/downtime`): a scope of tags (`*` for everything), an
//! optional monitor or monitor tags, a start and an end in epoch seconds.

use super::{ApiError, ApiResult};
use crate::AppState;
use crate::monitors::{MonitorDef, group_tags};
use crate::store::pg::new_id;
use axum::Json;
use axum::extract::{Path, State};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const KIND: &str = "downtime";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Downtime {
    #[serde(default)]
    pub id: Option<String>,
    /// `*`, or `key:value` tags a monitor group must all carry.
    #[serde(default, deserialize_with = "scope")]
    pub scope: String,
    #[serde(default)]
    pub monitor_id: Option<i64>,
    #[serde(default)]
    pub monitor_tags: Vec<String>,
    /// Epoch seconds; now when absent.
    #[serde(default)]
    pub start: Option<i64>,
    /// Epoch seconds; none: until canceled.
    #[serde(default)]
    pub end: Option<i64>,
    #[serde(default)]
    pub message: Option<String>,
    /// When it was canceled, epoch seconds.
    #[serde(default)]
    pub canceled: Option<i64>,
    /// `scheduled`, `active`, `ended` or `canceled`; computed.
    #[serde(default, skip_deserializing)]
    pub status: String,
}

/// Datadog accepts the scope as a string or a list of tags.
fn scope<'de, D: serde::Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    Ok(match Value::deserialize(d)? {
        Value::String(s) => s,
        Value::Array(a) => a
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(","),
        _ => String::new(),
    })
}

impl Downtime {
    fn with_status(mut self, now_s: i64) -> Self {
        self.status = if self.canceled.is_some() {
            "canceled"
        } else if self.start.unwrap_or(0) > now_s {
            "scheduled"
        } else if self.end.is_some_and(|e| e <= now_s) {
            "ended"
        } else {
            "active"
        }
        .into();
        self
    }

    /// Whether it silences this group of this monitor now.
    pub fn applies(&self, def: &MonitorDef, group: &str, now_ms: i64) -> bool {
        let now = now_ms / 1000;
        if self.canceled.is_some()
            || self.start.unwrap_or(0) > now
            || self.end.is_some_and(|e| e <= now)
        {
            return false;
        }
        if self.monitor_id.is_some() && self.monitor_id != def.id {
            return false;
        }
        if !self.monitor_tags.iter().all(|t| def.tags.contains(t)) {
            return false;
        }
        let scope = self.scope.trim();
        if scope.is_empty() || scope == "*" {
            return true;
        }
        let tags = group_tags(group);
        scope.split(',').map(str::trim).all(|t| {
            def.tags.iter().any(|x| x == t)
                || t.split_once(':')
                    .is_some_and(|(k, v)| tags.get(k).is_some_and(|x| x == v))
        })
    }
}

fn now_s() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub async fn stored(st: &AppState) -> anyhow::Result<Vec<Downtime>> {
    let now = now_s();
    Ok(st
        .store
        .pg
        .docs(KIND)
        .await?
        .into_iter()
        .filter_map(|d| serde_json::from_value::<Downtime>(d.body).ok())
        .map(|d| d.with_status(now))
        .collect())
}

fn check(mut d: Downtime) -> Result<Downtime, ApiError> {
    d.scope = d.scope.trim().to_string();
    if d.scope.is_empty() {
        d.scope = "*".into();
    }
    d.monitor_tags.retain(|t| !t.trim().is_empty());
    let start = *d.start.get_or_insert_with(now_s);
    if d.end.is_some_and(|e| e <= start) {
        return Err(ApiError::BadQuery("the end must be after the start".into()));
    }
    Ok(d)
}

pub async fn list(State(st): State<AppState>) -> ApiResult<Vec<Downtime>> {
    let mut all = stored(&st).await?;
    all.sort_by_key(|d| std::cmp::Reverse(d.start));
    Ok(Json(all))
}

pub async fn create(State(st): State<AppState>, Json(d): Json<Downtime>) -> ApiResult<Downtime> {
    let mut d = check(d)?;
    let id = new_id();
    d.id = Some(id.clone());
    d.canceled = None;
    st.store
        .pg
        .put_doc(
            KIND,
            &id,
            &serde_json::to_value(&d).map_err(anyhow::Error::from)?,
        )
        .await?;
    Ok(Json(d.with_status(now_s())))
}

pub async fn update(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Json(d): Json<Downtime>,
) -> ApiResult<Downtime> {
    st.store
        .pg
        .doc(KIND, &id)
        .await?
        .ok_or(ApiError::NotFound)?;
    let mut d = check(d)?;
    d.id = Some(id.clone());
    st.store
        .pg
        .put_doc(
            KIND,
            &id,
            &serde_json::to_value(&d).map_err(anyhow::Error::from)?,
        )
        .await?;
    Ok(Json(d.with_status(now_s())))
}

/// Canceling keeps the downtime in the list, as Datadog does; a second
/// delete removes it.
pub async fn cancel(State(st): State<AppState>, Path(id): Path<String>) -> ApiResult<Value> {
    let doc = st
        .store
        .pg
        .doc(KIND, &id)
        .await?
        .ok_or(ApiError::NotFound)?;
    let mut d: Downtime = serde_json::from_value(doc.body).map_err(anyhow::Error::from)?;
    if d.canceled.is_some() || d.end.is_some_and(|e| e <= now_s()) {
        st.store.pg.delete_doc(KIND, &id).await?;
    } else {
        d.canceled = Some(now_s());
        st.store
            .pg
            .put_doc(
                KIND,
                &id,
                &serde_json::to_value(&d).map_err(anyhow::Error::from)?,
            )
            .await?;
    }
    Ok(Json(json!({})))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn applies_by_time_monitor_and_scope() {
        let def: MonitorDef = serde_json::from_value(json!({
            "id": 7, "query": "avg(last_5m):avg:a{*} by {host} > 1", "tags": ["team:shop"]
        }))
        .unwrap();
        let d = |v: Value| serde_json::from_value::<Downtime>(v).unwrap();
        let all = d(json!({"scope": "*", "start": 100, "end": 200}));
        assert!(all.applies(&def, "host:a", 150_000));
        assert!(!all.applies(&def, "host:a", 99_000));
        assert!(!all.applies(&def, "host:a", 200_000));
        assert!(d(json!({"scope": ["host:a"], "start": 0})).applies(&def, "host:a", 1));
        assert!(!d(json!({"scope": "host:a", "start": 0})).applies(&def, "host:b", 1));
        assert!(d(json!({"scope": "team:shop", "start": 0})).applies(&def, "host:b", 1));
        assert!(!d(json!({"monitor_id": 8, "start": 0})).applies(&def, "host:a", 1));
        assert!(!d(json!({"monitor_tags": ["team:x"], "start": 0})).applies(&def, "host:a", 1));
        assert!(!d(json!({"start": 0, "canceled": 5})).applies(&def, "host:a", 1));
        assert_eq!(all.with_status(150).status, "active");
    }
}
