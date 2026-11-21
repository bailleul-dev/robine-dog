//! Metrics without Limits: per metric, the tags kept at write time (an
//! allowlist, or a denylist with `exclude_tags_mode`); the other tags are
//! aggregated away (counts and rates summed, gauges averaged), which bounds
//! the number of series. `/api/v2/metrics/{name}/tags` configures it,
//! `/api/v2/metrics/{name}/volumes` tells the series before and after.

use crate::AppState;
use crate::api::{ApiError, ApiResult};
use crate::metrics::Point;
use axum::Json;
use axum::extract::{Path, State};
use serde_json::{Value, json};
use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::sync::{LazyLock, Mutex, RwLock};

const KIND: &str = "metric_tag_config";

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub tags: Vec<String>,
    pub exclude: bool,
    /// gauge, count, rate or distribution.
    pub metric_type: String,
}

impl Config {
    fn keeps(&self, key: &str) -> bool {
        self.tags.iter().any(|t| t == key) != self.exclude
    }
}

/// (ts, metric, host, tags).
type SeriesKey = (i64, String, Option<String>, Vec<String>);

static CONFIGS: LazyLock<RwLock<HashMap<String, Config>>> = LazyLock::new(Default::default);
/// Per metric, this hour's distinct series before and after (hashes).
type Seen = HashMap<String, (HashSet<u64>, HashSet<u64>)>;
static SEEN: LazyLock<Mutex<(i64, Seen)>> = LazyLock::new(Default::default);

fn parse(a: &Value) -> Result<Config, ApiError> {
    let tags: Vec<String> = a["tags"]
        .as_array()
        .ok_or_else(|| ApiError::BadQuery("attributes.tags: a list of tag keys".into()))?
        .iter()
        .filter_map(|t| t.as_str().map(String::from))
        .collect();
    Ok(Config {
        tags,
        exclude: a["exclude_tags_mode"].as_bool().unwrap_or(false),
        metric_type: a["metric_type"].as_str().unwrap_or("gauge").to_string(),
    })
}

fn key_hash(p: &Point) -> u64 {
    let mut h = DefaultHasher::new();
    (&p.host, &p.tags).hash(&mut h);
    h.finish()
}

/// The points, with the tags of configured metrics reduced and merged.
pub fn apply(points: Vec<Point>) -> Vec<Point> {
    let configs = CONFIGS.read().unwrap_or_else(|e| e.into_inner());
    if configs.is_empty() {
        return points;
    }
    let mut out = Vec::with_capacity(points.len());
    // (ts, metric, host, tags) → (index in out, samples merged)
    let mut merged: HashMap<SeriesKey, (usize, f64)> = HashMap::new();
    let hour = crate::intake::logs::now_ms() / 3_600_000;
    let mut seen = SEEN.lock().unwrap_or_else(|e| e.into_inner());
    if seen.0 != hour {
        *seen = (hour, HashMap::new());
    }
    for mut p in points {
        let Some(c) = configs.get(&p.metric) else {
            out.push(p);
            continue;
        };
        let before = key_hash(&p);
        p.tags
            .retain(|t| c.keeps(t.split_once(':').map(|x| x.0).unwrap_or(t)));
        if !c.keeps("host") {
            p.host = None;
        }
        let s = seen.1.entry(p.metric.clone()).or_default();
        s.0.insert(before);
        s.1.insert(key_hash(&p));
        if c.metric_type == "distribution" {
            out.push(p);
            continue;
        }
        let k = (p.ts, p.metric.clone(), p.host.clone(), p.tags.clone());
        match merged.get_mut(&k) {
            Some((i, n)) => {
                let q = &mut out[*i];
                if c.metric_type == "gauge" {
                    q.value = (q.value * *n + p.value) / (*n + 1.0);
                } else {
                    q.value += p.value;
                }
                *n += 1.0;
            }
            None => {
                merged.insert(k, (out.len(), 1.0));
                out.push(p);
            }
        }
    }
    out
}

pub async fn init(st: &AppState) -> anyhow::Result<()> {
    let mut m = HashMap::new();
    for d in st.store.pg.docs(KIND).await? {
        if let Ok(c) = parse(&d.body) {
            m.insert(d.id, c);
        }
    }
    *CONFIGS.write().unwrap_or_else(|e| e.into_inner()) = m;
    Ok(())
}

fn view(name: &str, b: &Value) -> Value {
    json!({"type": "manage_tags", "id": name, "attributes": b})
}

/// `GET /api/v2/metrics/{name}/tags`.
pub async fn get(State(st): State<AppState>, Path(name): Path<String>) -> ApiResult<Value> {
    let d = st
        .store
        .pg
        .doc(KIND, &name)
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(json!({"data": view(&name, &d.body)})))
}

/// `POST` / `PATCH /api/v2/metrics/{name}/tags`.
pub async fn put(
    State(st): State<AppState>,
    Path(name): Path<String>,
    Json(v): Json<Value>,
) -> ApiResult<Value> {
    let mut a = v["data"]["attributes"].clone();
    if let Some(old) = st.store.pg.doc(KIND, &name).await? {
        for (k, x) in old.body.as_object().cloned().unwrap_or_default() {
            if a.get(&k).is_none() {
                a[k] = x;
            }
        }
    }
    let c = parse(&a)?;
    a["created_at"] = a
        .get("created_at")
        .cloned()
        .unwrap_or(json!(crate::intake::logs::now_ms()));
    st.store.pg.put_doc(KIND, &name, &a).await?;
    CONFIGS
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .insert(name.clone(), c);
    Ok(Json(json!({"data": view(&name, &a)})))
}

/// `DELETE /api/v2/metrics/{name}/tags`.
pub async fn delete(State(st): State<AppState>, Path(name): Path<String>) -> ApiResult<Value> {
    st.store.pg.delete_doc(KIND, &name).await?;
    CONFIGS
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&name);
    Ok(Json(json!({})))
}

/// `GET /api/v2/metrics/{name}/volumes`: distinct series this hour.
pub async fn volumes(Path(name): Path<String>) -> ApiResult<Value> {
    let seen = SEEN.lock().unwrap_or_else(|e| e.into_inner());
    let (i, o) = seen
        .1
        .get(&name)
        .map(|(a, b)| (a.len(), b.len()))
        .unwrap_or((0, 0));
    Ok(Json(json!({"data": {"type": "metric_volumes", "id": name,
        "attributes": {"ingested_volume": i, "indexed_volume": o}}})))
}

/// `GET /api/ui/metrics/tag-configs`: every configuration with its volumes.
pub async fn list(State(st): State<AppState>) -> ApiResult<Value> {
    let docs = st.store.pg.docs(KIND).await?;
    let seen = SEEN.lock().unwrap_or_else(|e| e.into_inner());
    Ok(Json(json!({"data": docs.iter().map(|d| {
        let (i, o) = seen.1.get(&d.id).map(|(a, b)| (a.len(), b.len())).unwrap_or((0, 0));
        json!({"metric": d.id, "tags": d.body["tags"], "exclude_tags_mode": d.body["exclude_tags_mode"],
            "metric_type": d.body["metric_type"], "ingested_series": i, "indexed_series": o})
    }).collect::<Vec<_>>()})))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_are_aggregated_away() {
        CONFIGS.write().unwrap().insert(
            "req.count".into(),
            Config {
                tags: vec!["env".into()],
                exclude: false,
                metric_type: "count".into(),
            },
        );
        let p = |pod: &str, v| Point {
            ts: 0,
            metric: "req.count".into(),
            host: Some("h".into()),
            tags: vec!["env:prod".into(), format!("pod:{pod}")],
            value: v,
            weight: 1.0,
        };
        let out = apply(vec![p("a", 1.0), p("b", 2.0), p("c", 3.0)]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].tags, vec!["env:prod".to_string()]);
        assert_eq!(out[0].host, None);
        assert_eq!(out[0].value, 6.0);
        CONFIGS.write().unwrap().remove("req.count");
    }
}
