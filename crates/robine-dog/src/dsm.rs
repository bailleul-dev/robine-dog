//! Data Streams Monitoring: the pathway stats tracers send to
//! `/v0.1/pipeline_stats` (msgpack, gzip; DDSketch latencies) and Kafka
//! offsets (backlogs), turned into throughput and latency per edge
//! (service → topic → service) and consumer lag per group and topic.

use crate::AppState;
use crate::api::{ApiError, ApiResult};
use axum::Json;
use axum::body::Bytes;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use prost::Message;
use rmpv::Value as M;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap};
use std::io::Read;

#[derive(Clone, PartialEq, Message)]
pub struct DdSketch {
    #[prost(message, optional, tag = "1")]
    pub mapping: Option<IndexMapping>,
    #[prost(message, optional, tag = "2")]
    pub positive_values: Option<Store>,
    #[prost(message, optional, tag = "3")]
    pub negative_values: Option<Store>,
    #[prost(double, tag = "4")]
    pub zero_count: f64,
}

#[derive(Clone, PartialEq, Message)]
pub struct IndexMapping {
    #[prost(double, tag = "1")]
    pub gamma: f64,
    #[prost(double, tag = "2")]
    pub index_offset: f64,
}

#[derive(Clone, PartialEq, Message)]
pub struct Store {
    #[prost(map = "sint32, double", tag = "1")]
    pub bin_counts: HashMap<i32, f64>,
    #[prost(double, repeated, tag = "2")]
    pub contiguous_bin_counts: Vec<f64>,
    #[prost(sint32, tag = "3")]
    pub contiguous_bin_index_offset: i32,
}

/// A sketch's count and quantiles (in its unit, seconds for DSM).
pub fn sketch(bytes: &[u8], qs: &[f64]) -> Option<(f64, Vec<f64>)> {
    let s = DdSketch::decode(bytes).ok()?;
    let m = s.mapping.clone()?;
    if m.gamma <= 1.0 {
        return None;
    }
    let mut bins: Vec<(i32, f64)> = Vec::new();
    if let Some(p) = &s.positive_values {
        bins.extend(p.bin_counts.iter().map(|(k, v)| (*k, *v)));
        for (i, c) in p.contiguous_bin_counts.iter().enumerate() {
            bins.push((p.contiguous_bin_index_offset + i as i32, *c));
        }
    }
    bins.sort_by_key(|b| b.0);
    let total: f64 = s.zero_count + bins.iter().map(|b| b.1).sum::<f64>();
    if total <= 0.0 {
        return Some((0.0, qs.iter().map(|_| 0.0).collect()));
    }
    // A bin's representative value: the middle of [γ^(i-1), γ^i] after the offset.
    let value = |i: i32| m.gamma.powf(i as f64 - m.index_offset) * 2.0 / (1.0 + m.gamma);
    let out = qs
        .iter()
        .map(|q| {
            let rank = q * (total - 1.0);
            let mut acc = s.zero_count;
            if rank < acc {
                return 0.0;
            }
            for (i, c) in &bins {
                acc += c;
                if acc > rank {
                    return value(*i);
                }
            }
            bins.last().map(|b| value(b.0)).unwrap_or(0.0)
        })
        .collect();
    Some((total, out))
}

fn get<'a>(m: &'a M, key: &str) -> Option<&'a M> {
    m.as_map()?
        .iter()
        .find(|(k, _)| k.as_str() == Some(key))
        .map(|(_, v)| v)
}
fn strs(v: Option<&M>) -> Vec<String> {
    v.and_then(|a| a.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}
fn u64_of(v: Option<&M>) -> u64 {
    v.and_then(|x| x.as_u64().or(x.as_i64().map(|i| i as u64)))
        .unwrap_or(0)
}
fn bytes_of(v: Option<&M>) -> &[u8] {
    match v {
        Some(M::Binary(b)) => b,
        _ => &[],
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Point {
    pub ts: i64,
    pub env: Option<String>,
    pub service: String,
    pub edge_tags: Vec<String>,
    pub hash: String,
    pub parent_hash: String,
    pub count: f64,
    pub pathway: Option<(f64, f64)>,
    pub edge: Option<(f64, f64)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Backlog {
    pub ts: i64,
    pub service: String,
    pub tags: Vec<String>,
    pub value: i64,
}

pub fn parse(root: &M) -> (Vec<Point>, Vec<Backlog>) {
    let env = get(root, "Env")
        .and_then(|v| v.as_str())
        .filter(|e| !e.is_empty())
        .map(String::from);
    let service = get(root, "Service")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();
    let (mut points, mut backlogs) = (Vec::new(), Vec::new());
    for b in get(root, "Stats")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
    {
        let ts = (u64_of(get(b, "Start")) / 1_000_000) as i64;
        for p in get(b, "Stats")
            .and_then(|v| v.as_array())
            .into_iter()
            .flatten()
        {
            // Only "current" timestamps, as Datadog's backend (origin ones double count).
            if get(p, "TimestampType").and_then(|v| v.as_str()) == Some("origin") {
                continue;
            }
            let pathway = sketch(bytes_of(get(p, "PathwayLatency")), &[0.5, 0.99]);
            let edge = sketch(bytes_of(get(p, "EdgeLatency")), &[0.5, 0.99]);
            points.push(Point {
                ts,
                env: env.clone(),
                service: service.clone(),
                edge_tags: strs(get(p, "EdgeTags")),
                hash: u64_of(get(p, "Hash")).to_string(),
                parent_hash: u64_of(get(p, "ParentHash")).to_string(),
                count: pathway.as_ref().map(|s| s.0).unwrap_or(0.0),
                pathway: pathway.map(|s| (s.1[0], s.1[1])),
                edge: edge.map(|s| (s.1[0], s.1[1])),
            });
        }
        for k in get(b, "Backlogs")
            .and_then(|v| v.as_array())
            .into_iter()
            .flatten()
        {
            backlogs.push(Backlog {
                ts,
                service: service.clone(),
                tags: strs(get(k, "Tags")),
                value: get(k, "Value").and_then(|v| v.as_i64()).unwrap_or(0),
            });
        }
    }
    (points, backlogs)
}

async fn store(st: &AppState, points: &[Point], backlogs: &[Backlog]) -> anyhow::Result<()> {
    let c = st.store.pg.conn().await?;
    for p in points {
        c.execute(
            "INSERT INTO dsm_points (ts, env, service, edge_tags, hash, parent_hash, count, pathway_p50, pathway_p99,
                edge_p50, edge_p99) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)",
            &[
                &p.ts, &p.env, &p.service, &p.edge_tags, &p.hash, &p.parent_hash, &p.count,
                &p.pathway.map(|x| x.0), &p.pathway.map(|x| x.1), &p.edge.map(|x| x.0), &p.edge.map(|x| x.1),
            ],
        )
        .await?;
    }
    for b in backlogs {
        c.execute(
            "INSERT INTO dsm_backlogs (ts, service, tags, value) VALUES ($1,$2,$3,$4)",
            &[&b.ts, &b.service, &b.tags, &b.value],
        )
        .await?;
    }
    Ok(())
}

/// `POST /v0.1/pipeline_stats` (trace port) and `/api/v0.1/pipeline_stats` (the Agent).
pub async fn intake(
    axum::extract::State(st): axum::extract::State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let gz = body.starts_with(&[0x1f, 0x8b])
        || headers
            .get("content-encoding")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.contains("gzip"));
    let mut raw = Vec::new();
    if gz {
        if flate2::read::MultiGzDecoder::new(&body[..])
            .read_to_end(&mut raw)
            .is_err()
        {
            return (StatusCode::BAD_REQUEST, "gzip").into_response();
        }
    } else {
        raw = body.to_vec();
    }
    let root = match rmpv::decode::read_value(&mut &raw[..]) {
        Ok(v) => v,
        Err(e) => return (StatusCode::BAD_REQUEST, format!("msgpack: {e}")).into_response(),
    };
    let (points, backlogs) = parse(&root);
    if let Err(e) = store(&st, &points, &backlogs).await {
        tracing::warn!(error = %e, "storing data streams stats");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    StatusCode::ACCEPTED.into_response()
}

fn tag<'a>(tags: &'a [String], k: &str) -> Option<&'a str> {
    tags.iter()
        .find_map(|t| t.strip_prefix(k)?.strip_prefix(':'))
}

/// Lag per consumer group and topic: latest produced offset minus latest
/// committed one, summed over partitions.
pub fn lags(backlogs: &[Backlog]) -> Vec<(String, String, i64)> {
    let mut produce: HashMap<(String, String), (i64, i64)> = HashMap::new();
    let mut commit: HashMap<(String, String, String), (i64, i64)> = HashMap::new();
    for b in backlogs {
        let (Some(topic), Some(part)) = (tag(&b.tags, "topic"), tag(&b.tags, "partition")) else {
            continue;
        };
        let latest = |e: &mut (i64, i64)| {
            if b.ts >= e.0 {
                *e = (b.ts, b.value)
            }
        };
        match tag(&b.tags, "type") {
            Some("kafka_produce") => latest(
                produce
                    .entry((topic.into(), part.into()))
                    .or_insert((i64::MIN, 0)),
            ),
            Some("kafka_commit") => {
                let g = tag(&b.tags, "consumer_group").unwrap_or("").to_string();
                latest(
                    commit
                        .entry((g, topic.into(), part.into()))
                        .or_insert((i64::MIN, 0)),
                )
            }
            _ => {}
        }
    }
    let mut out: BTreeMap<(String, String), i64> = BTreeMap::new();
    for ((g, topic, part), (_, committed)) in commit {
        if let Some((_, produced)) = produce.get(&(topic.clone(), part)) {
            *out.entry((g, topic)).or_default() += (produced - committed).max(0);
        }
    }
    out.into_iter().map(|((g, t), l)| (g, t, l)).collect()
}

#[derive(Debug, Deserialize)]
pub struct Filter {
    pub from: i64,
    pub to: i64,
}

/// `GET /api/ui/data-streams`: edges with throughput and latency, consumer lag.
pub async fn overview(State(st): State<AppState>, Query(f): Query<Filter>) -> ApiResult<Value> {
    let c = st.store.pg.conn().await?;
    let err = |e: tokio_postgres::Error| ApiError::Internal(e.into());
    let rows = c
        .query(
            "SELECT service, edge_tags, sum(count), avg(edge_p50), max(edge_p99), avg(pathway_p50), max(pathway_p99)
             FROM dsm_points WHERE ts >= $1 AND ts <= $2 GROUP BY service, edge_tags ORDER BY 3 DESC LIMIT 500",
            &[&f.from, &f.to],
        )
        .await
        .map_err(err)?;
    let seconds = ((f.to - f.from) as f64 / 1000.0).max(1.0);
    let edges: Vec<Value> = rows
        .iter()
        .map(|r| {
            let tags: Vec<String> = r.get(1);
            json!({"service": r.get::<_, String>(0),
                "direction": tag(&tags, "direction"), "topic": tag(&tags, "topic").or(tag(&tags, "exchange")),
                "type": tag(&tags, "type"), "group": tag(&tags, "group"), "tags": tags,
                "throughput": r.get::<_, f64>(2) / seconds,
                "edge_p50": r.get::<_, Option<f64>>(3), "edge_p99": r.get::<_, Option<f64>>(4),
                "pathway_p50": r.get::<_, Option<f64>>(5), "pathway_p99": r.get::<_, Option<f64>>(6)})
        })
        .collect();
    let b = c
        .query(
            "SELECT ts, service, tags, value FROM dsm_backlogs WHERE ts >= $1 AND ts <= $2",
            &[&f.from, &f.to],
        )
        .await
        .map_err(err)?;
    let backlogs: Vec<Backlog> = b
        .iter()
        .map(|r| Backlog {
            ts: r.get(0),
            service: r.get(1),
            tags: r.get(2),
            value: r.get(3),
        })
        .collect();
    Ok(Json(json!({
        "edges": edges,
        "lag": lags(&backlogs).into_iter().map(|(g, t, l)| json!({"consumer_group": g, "topic": t, "lag": l})).collect::<Vec<_>>(),
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_sketches_and_lag() {
        // 100 values in bin 10, γ = 1.02: about 1.02^10 ≈ 1.2 s.
        let sk = DdSketch {
            mapping: Some(IndexMapping {
                gamma: 1.02,
                index_offset: 0.0,
            }),
            positive_values: Some(Store {
                contiguous_bin_counts: vec![100.0],
                contiguous_bin_index_offset: 10,
                ..Default::default()
            }),
            negative_values: None,
            zero_count: 0.0,
        }
        .encode_to_vec();
        let (n, q) = sketch(&sk, &[0.5]).unwrap();
        assert_eq!(n, 100.0);
        assert!((q[0] - 1.02f64.powi(10) * 2.0 / 2.02).abs() < 1e-9);
        let s = |x: &str| M::String(x.into());
        let root = M::Map(vec![
            (s("Env"), s("prod")),
            (s("Service"), s("orders")),
            (
                s("Stats"),
                M::Array(vec![M::Map(vec![
                    (s("Start"), M::from(1_700_000_000_000_000_000u64)),
                    (
                        s("Stats"),
                        M::Array(vec![M::Map(vec![
                            (
                                s("EdgeTags"),
                                M::Array(vec![
                                    s("direction:out"),
                                    s("topic:orders"),
                                    s("type:kafka"),
                                ]),
                            ),
                            (s("Hash"), M::from(7u64)),
                            (s("ParentHash"), M::from(0u64)),
                            (s("PathwayLatency"), M::Binary(sk.clone())),
                            (s("EdgeLatency"), M::Binary(sk)),
                            (s("TimestampType"), s("current")),
                        ])]),
                    ),
                    (
                        s("Backlogs"),
                        M::Array(vec![
                            M::Map(vec![
                                (
                                    s("Tags"),
                                    M::Array(vec![
                                        s("partition:0"),
                                        s("topic:orders"),
                                        s("type:kafka_produce"),
                                    ]),
                                ),
                                (s("Value"), M::from(120)),
                            ]),
                            M::Map(vec![
                                (
                                    s("Tags"),
                                    M::Array(vec![
                                        s("consumer_group:billing"),
                                        s("partition:0"),
                                        s("topic:orders"),
                                        s("type:kafka_commit"),
                                    ]),
                                ),
                                (s("Value"), M::from(100)),
                            ]),
                        ]),
                    ),
                ])]),
            ),
        ]);
        let (points, backlogs) = parse(&root);
        assert_eq!(points.len(), 1);
        assert_eq!(
            (points[0].service.as_str(), points[0].count, points[0].ts),
            ("orders", 100.0, 1_700_000_000_000)
        );
        assert_eq!(
            lags(&backlogs),
            vec![("billing".into(), "orders".into(), 20)]
        );
    }
}
