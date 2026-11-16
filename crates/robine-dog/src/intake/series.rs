//! Metric submission over HTTP, as Datadog's API takes it:
//! `POST /api/v1/series` and `POST /api/v2/series` (JSON, or the Agent's
//! protobuf), authenticated by `DD-API-KEY`. Points are stored as sent,
//! without aggregation.

use super::logs::{api_key_ok, decompress};
use crate::AppState;
use crate::metrics::{Kind, Point, normalize_tags};
use axum::Json;
use axum::body::Bytes;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::json;
use std::collections::HashMap;

#[derive(Deserialize)]
struct V1Payload {
    series: Vec<V1Series>,
}

#[derive(Deserialize)]
struct V1Series {
    metric: String,
    points: Vec<(f64, f64)>,
    #[serde(rename = "type")]
    typ: Option<String>,
    host: Option<String>,
    #[serde(default)]
    tags: Vec<String>,
}

#[derive(Deserialize)]
struct V2Payload {
    series: Vec<V2Series>,
}

#[derive(Deserialize)]
struct V2Point {
    timestamp: i64,
    value: f64,
}

#[derive(Deserialize)]
struct V2Resource {
    name: String,
    #[serde(rename = "type")]
    typ: String,
}

#[derive(Deserialize)]
struct V2Series {
    metric: String,
    points: Vec<V2Point>,
    #[serde(rename = "type", default)]
    typ: i64,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    resources: Vec<V2Resource>,
}

pub struct Series {
    pub metric: String,
    pub kind: Kind,
    pub host: Option<String>,
    pub tags: Vec<String>,
    /// `(epoch seconds, value)`.
    pub points: Vec<(i64, f64)>,
}

fn kind_v1(t: Option<&str>) -> Kind {
    match t {
        Some("count") => Kind::Count,
        Some("rate") => Kind::Rate,
        _ => Kind::Gauge,
    }
}

fn kind_v2(t: i64) -> Kind {
    match t {
        1 => Kind::Count,
        2 => Kind::Rate,
        _ => Kind::Gauge,
    }
}

pub fn parse_v1(body: &[u8]) -> anyhow::Result<Vec<Series>> {
    let p: V1Payload = serde_json::from_slice(body)?;
    Ok(p.series
        .into_iter()
        .map(|s| Series {
            kind: kind_v1(s.typ.as_deref()),
            host: s.host.filter(|h| !h.is_empty()),
            tags: normalize_tags(s.tags),
            points: s.points.into_iter().map(|(t, v)| (t as i64, v)).collect(),
            metric: s.metric,
        })
        .collect())
}

pub fn parse_v2(body: &[u8]) -> anyhow::Result<Vec<Series>> {
    let p: V2Payload = serde_json::from_slice(body)?;
    Ok(p.series
        .into_iter()
        .map(|s| Series {
            kind: kind_v2(s.typ),
            host: s
                .resources
                .iter()
                .find(|r| r.typ == "host")
                .map(|r| r.name.clone()),
            tags: normalize_tags(s.tags),
            points: s
                .points
                .into_iter()
                .map(|p| (p.timestamp, p.value))
                .collect(),
            metric: s.metric,
        })
        .collect())
}

pub fn valid_name(m: &str) -> bool {
    !m.is_empty()
        && m.len() <= 200
        && m.chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

pub async fn store(st: &AppState, series: Vec<Series>) -> anyhow::Result<usize> {
    let mut kinds: HashMap<String, String> = HashMap::new();
    let mut points = Vec::new();
    for s in series.into_iter().filter(|s| valid_name(&s.metric)) {
        kinds.insert(s.metric.clone(), s.kind.as_str().to_string());
        for (t, v) in s.points {
            if v.is_finite() {
                points.push(Point {
                    ts: t * 1000,
                    metric: s.metric.clone(),
                    host: s.host.clone(),
                    tags: s.tags.clone(),
                    value: v,
                    weight: 1.0,
                });
            }
        }
    }
    let n = points.len();
    st.store
        .pg
        .touch_metrics(&kinds.into_iter().collect::<Vec<_>>())
        .await?;
    st.store.points.send(points).await?;
    Ok(n)
}

async fn handle(
    st: AppState,
    headers: HeaderMap,
    q: HashMap<String, String>,
    body: Bytes,
    parse: fn(&[u8]) -> anyhow::Result<Vec<Series>>,
) -> Response {
    if !api_key_ok(&st, &headers, &q) {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({"errors": ["Forbidden"]})),
        )
            .into_response();
    }
    // The Agent sends protobuf (`MetricPayload`); scripts and libraries JSON.
    let parse = if headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|c| c.contains("protobuf"))
    {
        super::agent::parse_series
    } else {
        parse
    };
    let body = match decompress(&headers, body) {
        Ok(b) => b,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"errors": [e.to_string()]})),
            )
                .into_response();
        }
    };
    let series = match parse(&body) {
        Ok(s) => s,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"errors": [e.to_string()]})),
            )
                .into_response();
        }
    };
    match store(&st, series).await {
        Ok(n) => {
            tracing::debug!(points = n, "series received");
            (StatusCode::ACCEPTED, Json(json!({"errors": []}))).into_response()
        }
        Err(e) => {
            tracing::error!(error = %format!("{e:#}"), "storing series");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"errors": ["Storage error"]})),
            )
                .into_response()
        }
    }
}

pub async fn v1(
    State(st): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
    body: Bytes,
) -> Response {
    handle(st, headers, q, body, parse_v1).await
}

pub async fn v2(
    State(st): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
    body: Bytes,
) -> Response {
    handle(st, headers, q, body, parse_v2).await
}

/// `GET /api/v1/validate`: what clients call to check their API key.
pub async fn validate(
    State(st): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    if api_key_ok(&st, &headers, &q) {
        Json(json!({"valid": true})).into_response()
    } else {
        (
            StatusCode::FORBIDDEN,
            Json(json!({"errors": ["Forbidden"]})),
        )
            .into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_both_versions() {
        let v1 = parse_v1(br#"{"series":[{"metric":"a.b","points":[[1700000000,1.5]],"type":"count","host":"h","tags":["env:prod","env:prod"]}]}"#).unwrap();
        assert_eq!(v1[0].kind, Kind::Count);
        assert_eq!(v1[0].tags, vec!["env:prod"]);
        assert_eq!(v1[0].points, vec![(1_700_000_000, 1.5)]);

        let v2 = parse_v2(br#"{"series":[{"metric":"a.b","type":3,"points":[{"timestamp":1700000000,"value":2}],"resources":[{"name":"h1","type":"host"}]}]}"#).unwrap();
        assert_eq!(v2[0].kind, Kind::Gauge);
        assert_eq!(v2[0].host.as_deref(), Some("h1"));
    }
}
