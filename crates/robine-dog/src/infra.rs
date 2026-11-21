//! Infrastructure: Cloud Network (NPM), Network Devices, Fleet Automation
//! (read only) and Data Jobs Monitoring.
//!
//! - NPM: the process-agent's `CollectorConnections` (system-probe
//!   connections, message type 22), summed per minute and flow.
//! - Network Devices: the Agent's NDM metadata (`/api/v2/ndm`): devices and
//!   their interfaces, from the SNMP integration.
//! - Fleet: the Agent's inventory payloads (`/api/v1/metadata`): version,
//!   install method, configuration, checks.
//! - Data Jobs: the Java tracer's Spark spans (`spark.application`,
//!   `spark.job`, `spark.stage`).

use crate::AppState;
use crate::api::{ApiError, ApiResult};
use crate::store::greptime::{i64_at, str_at};
use axum::Json;
use axum::body::Bytes;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use prost::Message;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;

// ---------- NPM ----------

#[derive(Clone, PartialEq, Message)]
pub struct CollectorConnections {
    #[prost(string, tag = "2")]
    pub host_name: String,
    #[prost(message, repeated, tag = "3")]
    pub connections: Vec<Connection>,
    #[prost(map = "int32, string", tag = "10")]
    pub container_for_pid: HashMap<i32, String>,
}

#[derive(Clone, PartialEq, Message)]
pub struct Connection {
    #[prost(int32, tag = "1")]
    pub pid: i32,
    #[prost(message, optional, tag = "5")]
    pub laddr: Option<Addr>,
    #[prost(message, optional, tag = "6")]
    pub raddr: Option<Addr>,
    #[prost(int32, tag = "11")]
    pub typ: i32,
    #[prost(uint64, tag = "16")]
    pub last_bytes_sent: u64,
    #[prost(uint64, tag = "17")]
    pub last_bytes_received: u64,
    #[prost(uint32, tag = "18")]
    pub last_retransmits: u32,
    #[prost(int32, tag = "19")]
    pub direction: i32,
    #[prost(uint32, tag = "22")]
    pub rtt: u32,
    #[prost(bool, tag = "24")]
    pub intra_host: bool,
}

#[derive(Clone, PartialEq, Message)]
pub struct Addr {
    #[prost(string, tag = "2")]
    pub ip: String,
    #[prost(int32, tag = "3")]
    pub port: i32,
    #[prost(string, tag = "5")]
    pub container_id: String,
    #[prost(string, tag = "7")]
    pub host_name: String,
}

/// One flow over a minute: (host, local ip, remote ip, remote port, protocol,
/// direction) → (sent, received, retransmits, rtt sum, connections).
type Flows =
    HashMap<(String, String, String, i32, &'static str, &'static str), (i64, i64, i64, i64, i64)>;

pub fn flows(c: &CollectorConnections) -> Flows {
    let mut out: Flows = HashMap::new();
    for x in &c.connections {
        let (Some(l), Some(r)) = (&x.laddr, &x.raddr) else {
            continue;
        };
        let dir = match x.direction {
            1 => "incoming",
            2 => "outgoing",
            3 => "local",
            _ => "unknown",
        };
        // An incoming connection's interesting port is ours.
        let port = if dir == "incoming" { l.port } else { r.port };
        let proto = if x.typ == 1 { "udp" } else { "tcp" };
        let e = out
            .entry((
                c.host_name.clone(),
                l.ip.clone(),
                r.ip.clone(),
                port,
                proto,
                dir,
            ))
            .or_default();
        e.0 += x.last_bytes_sent as i64;
        e.1 += x.last_bytes_received as i64;
        e.2 += x.last_retransmits as i64;
        e.3 += x.rtt as i64;
        e.4 += 1;
    }
    out
}

/// A process-agent message whose header says it is connections (type 22).
pub fn is_connections(data: &[u8]) -> bool {
    data.len() >= 16 && data[0] == 3 && data[2] == 22
}

pub async fn store_connections(st: &AppState, data: &[u8]) -> anyhow::Result<usize> {
    let raw = crate::processes::body(data).map_err(|e| anyhow::anyhow!(e))?;
    let c = CollectorConnections::decode(raw.as_slice())?;
    let f = flows(&c);
    let minute = crate::intake::logs::now_ms().div_euclid(60_000) * 60_000;
    let db = st.store.pg.conn().await?;
    // Which host owns each local address: remote ends resolve to hosts.
    let mut locals: Vec<&String> = f.keys().map(|k| &k.1).collect();
    locals.sort();
    locals.dedup();
    for ip in locals.into_iter().filter(|ip| !ip.starts_with("127.")) {
        db.execute(
            "INSERT INTO npm_hosts (ip, host, seen) VALUES ($1,$2,$3) ON CONFLICT (ip) DO UPDATE SET host = EXCLUDED.host, seen = EXCLUDED.seen",
            &[ip, &c.host_name, &minute],
        )
        .await?;
    }
    for ((host, lip, rip, port, proto, dir), (sent, recv, retr, rtt, n)) in &f {
        db.execute(
            "INSERT INTO npm_flows (ts, host, local_ip, remote_ip, port, protocol, direction, bytes_sent, bytes_received, retransmits, rtt_sum, connections)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)",
            &[&minute, host, lip, rip, port, &proto.to_string(), &dir.to_string(), sent, recv, retr, rtt, n],
        )
        .await?;
    }
    Ok(f.len())
}

/// `POST /api/v1/connections`.
pub async fn connections(State(st): State<AppState>, body: Bytes) -> Response {
    match store_connections(&st, &body).await {
        Ok(_) => Json(json!({})).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
    }
}

#[derive(Debug, Deserialize)]
pub struct Range {
    pub from: i64,
    pub to: i64,
}

/// `GET /api/ui/npm`: flows over the range, remote ends resolved to hosts.
pub async fn npm(State(st): State<AppState>, Query(r): Query<Range>) -> ApiResult<Value> {
    let db = st.store.pg.conn().await?;
    let rows = db
        .query(
            "SELECT f.host, COALESCE(h.host, f.remote_ip), f.port, f.protocol, f.direction,
                    sum(f.bytes_sent)::BIGINT, sum(f.bytes_received)::BIGINT, sum(f.retransmits)::BIGINT,
                    (sum(f.rtt_sum) / GREATEST(sum(f.connections), 1))::BIGINT, max(f.connections)::BIGINT
             FROM npm_flows f LEFT JOIN npm_hosts h ON h.ip = f.remote_ip
             WHERE f.ts BETWEEN $1 AND $2 AND f.direction <> 'local'
             GROUP BY 1, 2, 3, 4, 5 ORDER BY 6 + 7 DESC LIMIT 1000",
            &[&r.from, &r.to],
        )
        .await
        .map_err(anyhow::Error::from)?;
    let data: Vec<Value> = rows
        .iter()
        .map(|x| {
            json!({"source": x.get::<_, String>(0), "dest": x.get::<_, String>(1), "port": x.get::<_, i32>(2),
                "protocol": x.get::<_, String>(3), "direction": x.get::<_, String>(4),
                "bytes_sent": x.get::<_, i64>(5), "bytes_received": x.get::<_, i64>(6),
                "retransmits": x.get::<_, i64>(7), "rtt_us": x.get::<_, i64>(8), "connections": x.get::<_, i64>(9)})
        })
        .collect();
    Ok(Json(json!({"data": data})))
}

pub fn spawn_purge(st: AppState) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
            let cutoff = crate::intake::logs::now_ms() - 3 * 86_400_000;
            if let Ok(db) = st.store.pg.conn().await {
                let _ = db
                    .execute("DELETE FROM npm_flows WHERE ts < $1", &[&cutoff])
                    .await;
            }
        }
    });
}

// ---------- Network Devices ----------

const DEVICE: &str = "ndm_device";
const INTERFACE: &str = "ndm_interface";

fn json_body(headers: &HeaderMap, body: Bytes) -> Option<Value> {
    let raw = crate::intake::logs::decompress(headers, body).ok()?;
    serde_json::from_slice(&raw).ok()
}

/// `POST /api/v2/ndm`: `NetworkDevicesMetadata`, one or a list.
pub async fn ndm(State(st): State<AppState>, headers: HeaderMap, body: Bytes) -> Response {
    let Some(v) = json_body(&headers, body) else {
        return (StatusCode::BAD_REQUEST, "JSON expected").into_response();
    };
    let payloads = match v {
        Value::Array(l) => l,
        o => vec![o],
    };
    let now = crate::intake::logs::now_ms();
    let r = async {
        for p in &payloads {
            for d in p["devices"].as_array().into_iter().flatten() {
                if let Some(id) = d["id"].as_str() {
                    let mut b = d.clone();
                    b["last_seen"] = json!(now);
                    b["namespace"] = p["namespace"].clone();
                    st.store.pg.put_doc(DEVICE, id, &b).await?;
                }
            }
            for i in p["interfaces"].as_array().into_iter().flatten() {
                if let (Some(d), Some(n)) = (i["device_id"].as_str(), i["index"].as_i64()) {
                    let mut b = i.clone();
                    b["last_seen"] = json!(now);
                    st.store
                        .pg
                        .put_doc(INTERFACE, &format!("{d}:{n}"), &b)
                        .await?;
                }
            }
        }
        anyhow::Ok(())
    }
    .await;
    match r {
        Ok(()) => (StatusCode::ACCEPTED, Json(json!({}))).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// `GET /api/ui/network-devices`.
pub async fn devices(State(st): State<AppState>) -> ApiResult<Value> {
    let interfaces = st.store.pg.docs(INTERFACE).await?;
    let mut by_device: HashMap<String, Vec<Value>> = HashMap::new();
    for i in interfaces {
        let d = i.body["device_id"].as_str().unwrap_or("").to_string();
        by_device.entry(d).or_default().push(i.body);
    }
    let data: Vec<Value> = st
        .store
        .pg
        .docs(DEVICE)
        .await?
        .into_iter()
        .map(|d| {
            let mut b = d.body;
            let mut ifs = by_device.remove(&d.id).unwrap_or_default();
            ifs.sort_by_key(|i| i["index"].as_i64().unwrap_or(0));
            b["interfaces"] = json!(ifs);
            b
        })
        .collect();
    Ok(Json(json!({"data": data})))
}

// ---------- Fleet ----------

const AGENT: &str = "fleet_agent";

/// `POST /api/v1/metadata`: inventories (`agent_metadata`, `check_metadata`,
/// `host_metadata`), the latest of each kind per host.
pub async fn inventory(State(st): State<AppState>, headers: HeaderMap, body: Bytes) -> Response {
    let Some(v) = json_body(&headers, body) else {
        return (StatusCode::ACCEPTED, Json(json!({}))).into_response();
    };
    let Some(host) = v["hostname"].as_str().map(String::from) else {
        return (StatusCode::ACCEPTED, Json(json!({}))).into_response();
    };
    let r = async {
        let mut doc = st
            .store
            .pg
            .doc(AGENT, &host)
            .await?
            .map(|d| d.body)
            .unwrap_or_else(|| json!({"hostname": host}));
        let mut touched = false;
        for k in ["agent_metadata", "check_metadata", "host_metadata"] {
            if !v[k].is_null() {
                doc[k] = v[k].clone();
                touched = true;
            }
        }
        if touched {
            doc["uuid"] = v["uuid"].clone();
            doc["last_seen"] = json!(crate::intake::logs::now_ms());
            st.store.pg.put_doc(AGENT, &host, &doc).await?;
        }
        anyhow::Ok(())
    }
    .await;
    if let Err(e) = r {
        tracing::warn!(error = %e, "inventory payload");
    }
    (StatusCode::ACCEPTED, Json(json!({}))).into_response()
}

/// `GET /api/ui/fleet`: every Agent, from its inventory or host metadata.
pub async fn fleet(State(st): State<AppState>) -> ApiResult<Value> {
    let mut agents: HashMap<String, Value> = st
        .store
        .pg
        .docs(AGENT)
        .await?
        .into_iter()
        .map(|d| (d.id, d.body))
        .collect();
    for h in st.hosts.list() {
        let e = agents
            .entry(h.name.clone())
            .or_insert_with(|| json!({"hostname": h.name}));
        if e["agent_metadata"]["agent_version"].is_null() {
            e["agent_version"] = json!(h.agent_version);
        }
        e["os"] = json!(h.os);
    }
    let mut data: Vec<Value> = agents
        .into_values()
        .map(|mut a| {
            let m = &a["agent_metadata"];
            let version = m["agent_version"]
                .as_str()
                .or(a["agent_version"].as_str())
                .map(String::from);
            let checks: Vec<String> = a["check_metadata"]
                .as_object()
                .map(|o| o.keys().cloned().collect())
                .unwrap_or_default();
            a["version"] = json!(version);
            a["checks"] = json!(checks);
            a
        })
        .collect();
    data.sort_by(|a, b| a["hostname"].as_str().cmp(&b["hostname"].as_str()));
    Ok(Json(json!({"data": data})))
}

// ---------- Data Jobs ----------

/// `GET /api/ui/data-jobs`: Spark applications, jobs and stages.
pub async fn data_jobs(State(st): State<AppState>, Query(r): Query<Range>) -> ApiResult<Value> {
    let time = crate::api::time_clause(crate::query::sql::Dataset::Spans, r.from, r.to);
    let summary = format!(
        "SELECT service, name, count(*), sum(CASE WHEN error THEN 1 ELSE 0 END), avg(duration), max(duration), CAST(max(ts) AS BIGINT) AS last
         FROM spans WHERE {time} AND name IN ('spark.application', 'spark.job', 'spark.stage', 'spark.sql')
         GROUP BY service, name ORDER BY service, name"
    );
    let runs = format!(
        "SELECT service, resource, CAST(ts AS BIGINT) AS t, duration, error, trace_id, span_id
         FROM spans WHERE {time} AND name = 'spark.application' ORDER BY ts DESC LIMIT 200"
    );
    let (summary, runs) = futures::try_join!(st.store.gt.query(&summary), st.store.gt.query(&runs))
        .map_err(ApiError::from)?;
    let f = |x: &Vec<Value>, i: usize| x.get(i).and_then(|v| v.as_f64()).unwrap_or(0.0);
    Ok(Json(json!({
        "summary": summary.iter().map(|x| json!({"app": str_at(x, 0), "kind": str_at(x, 1), "count": i64_at(x, 2),
            "errors": i64_at(x, 3), "avg_ns": f(x, 4), "max_ns": f(x, 5), "last_seen": i64_at(x, 6) / 1_000_000})).collect::<Vec<_>>(),
        "runs": runs.iter().map(|x| json!({"app": str_at(x, 0), "resource": str_at(x, 1), "ts": i64_at(x, 2) / 1_000_000,
            "duration_ns": f(x, 3), "error": x.get(4).and_then(|v| v.as_bool()).unwrap_or(false),
            "trace_id": str_at(x, 5), "span_id": str_at(x, 6)})).collect::<Vec<_>>(),
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connections_become_flows() {
        let a = |ip: &str, port| {
            Some(Addr {
                ip: ip.into(),
                port,
                ..Default::default()
            })
        };
        let c = CollectorConnections {
            host_name: "web-1".into(),
            connections: vec![
                Connection {
                    laddr: a("10.0.0.1", 40000),
                    raddr: a("10.0.0.2", 5432),
                    direction: 2,
                    last_bytes_sent: 100,
                    rtt: 300,
                    ..Default::default()
                },
                Connection {
                    laddr: a("10.0.0.1", 40001),
                    raddr: a("10.0.0.2", 5432),
                    direction: 2,
                    last_bytes_sent: 50,
                    rtt: 100,
                    ..Default::default()
                },
                Connection {
                    laddr: a("10.0.0.1", 443),
                    raddr: a("1.2.3.4", 51000),
                    direction: 1,
                    last_bytes_received: 10,
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let mut buf = vec![3u8, 0, 22, 0];
        buf.extend([0u8; 12]);
        c.encode(&mut buf).unwrap();
        assert!(is_connections(&buf));
        let raw = crate::processes::body(&buf).unwrap();
        let f = flows(&CollectorConnections::decode(raw.as_slice()).unwrap());
        let pg = f[&(
            "web-1".to_string(),
            "10.0.0.1".to_string(),
            "10.0.0.2".to_string(),
            5432,
            "tcp",
            "outgoing",
        )];
        assert_eq!((pg.0, pg.3, pg.4), (150, 400, 2));
        assert!(f.contains_key(&(
            "web-1".to_string(),
            "10.0.0.1".to_string(),
            "1.2.3.4".to_string(),
            443,
            "tcp",
            "incoming"
        )));
    }
}
