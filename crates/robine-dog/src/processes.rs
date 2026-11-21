//! Live Processes: the process-agent's `CollectorProc` payloads
//! (`/api/v1/collector`: a 16-byte header, then protobuf, possibly zstd),
//! kept in memory as each host's latest snapshot.

use crate::AppState;
use crate::api::ApiResult;
use axum::Json;
use axum::body::Bytes;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use prost::Message;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

#[derive(Clone, PartialEq, Message)]
pub struct CollectorProc {
    #[prost(string, tag = "2")]
    pub host_name: String,
    #[prost(message, repeated, tag = "3")]
    pub processes: Vec<Process>,
    #[prost(int32, tag = "6")]
    pub group_id: i32,
    #[prost(int32, tag = "7")]
    pub group_size: i32,
}

#[derive(Clone, PartialEq, Message)]
pub struct Process {
    #[prost(int32, tag = "2")]
    pub pid: i32,
    #[prost(message, optional, tag = "4")]
    pub command: Option<Command>,
    #[prost(message, optional, tag = "5")]
    pub user: Option<ProcessUser>,
    #[prost(message, optional, tag = "7")]
    pub memory: Option<MemoryStat>,
    #[prost(message, optional, tag = "8")]
    pub cpu: Option<CpuStat>,
    #[prost(int64, tag = "9")]
    pub create_time: i64,
    #[prost(int32, tag = "11")]
    pub open_fd_count: i32,
    #[prost(string, tag = "13")]
    pub container_id: String,
}

#[derive(Clone, PartialEq, Message)]
pub struct Command {
    #[prost(string, repeated, tag = "1")]
    pub args: Vec<String>,
    #[prost(string, tag = "3")]
    pub cwd: String,
    #[prost(int32, tag = "6")]
    pub ppid: i32,
    #[prost(string, tag = "8")]
    pub exe: String,
    #[prost(string, tag = "9")]
    pub comm: String,
}

#[derive(Clone, PartialEq, Message)]
pub struct ProcessUser {
    #[prost(string, tag = "1")]
    pub name: String,
    #[prost(int32, tag = "2")]
    pub uid: i32,
}

#[derive(Clone, PartialEq, Message)]
pub struct MemoryStat {
    #[prost(uint64, tag = "1")]
    pub rss: u64,
    #[prost(uint64, tag = "2")]
    pub vms: u64,
}

#[derive(Clone, PartialEq, Message)]
pub struct CpuStat {
    #[prost(float, tag = "2")]
    pub total_pct: f32,
    #[prost(float, tag = "3")]
    pub user_pct: f32,
    #[prost(float, tag = "4")]
    pub system_pct: f32,
    #[prost(int32, tag = "5")]
    pub num_threads: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Proc {
    pub host: String,
    pub pid: i32,
    pub ppid: i32,
    pub user: String,
    pub command: String,
    pub name: String,
    pub cpu_pct: f32,
    pub rss: u64,
    pub vms: u64,
    pub threads: i32,
    pub fds: i32,
    pub started: i64,
    pub container_id: String,
}

/// Each host's latest processes (a snapshot may come in several groups),
/// with when they were received.
type Snapshots = HashMap<String, (i64, HashMap<i32, Vec<Proc>>)>;
static LIVE: LazyLock<Mutex<Snapshots>> = LazyLock::new(|| Mutex::new(HashMap::new()));

/// The protobuf body of a process-agent message, past its header.
pub fn body(data: &[u8]) -> Result<Vec<u8>, String> {
    // Version 3 header: version, encoding, type, subscription, org (4), timestamp (8).
    if data.len() >= 16 && data[0] == 3 {
        let rest = &data[16..];
        return match data[1] {
            0 => Ok(rest.to_vec()),
            2 | 3 => zstd::decode_all(rest).map_err(|e| format!("zstd: {e}")),
            e => Err(format!("encoding {e} is not supported")),
        };
    }
    Ok(data.to_vec())
}

pub fn parse(data: &[u8]) -> Result<Vec<Proc>, String> {
    let raw = body(data)?;
    let c =
        CollectorProc::decode(raw.as_slice()).map_err(|e| format!("not a CollectorProc: {e}"))?;
    if c.host_name.is_empty() {
        return Err("no host name: not a process payload".into());
    }
    Ok(c.processes
        .iter()
        .map(|p| {
            let cmd = p.command.clone().unwrap_or_default();
            let name = if !cmd.comm.is_empty() {
                cmd.comm.clone()
            } else {
                cmd.args
                    .first()
                    .map(|a| a.rsplit('/').next().unwrap_or(a).to_string())
                    .unwrap_or_default()
            };
            let cpu = p.cpu.clone().unwrap_or_default();
            let mem = p.memory.clone().unwrap_or_default();
            Proc {
                host: c.host_name.clone(),
                pid: p.pid,
                ppid: cmd.ppid,
                user: p.user.as_ref().map(|u| u.name.clone()).unwrap_or_default(),
                command: if cmd.args.is_empty() {
                    cmd.exe.clone()
                } else {
                    cmd.args.join(" ")
                },
                name,
                cpu_pct: cpu.total_pct,
                rss: mem.rss,
                vms: mem.vms,
                threads: cpu.num_threads,
                fds: p.open_fd_count,
                started: p.create_time,
                container_id: p.container_id.clone(),
            }
        })
        .collect())
}

/// `POST /api/v1/collector`.
pub async fn intake(body: Bytes) -> Response {
    let raw = match self::body(&body) {
        Ok(r) => r,
        Err(e) => return (StatusCode::BAD_REQUEST, e).into_response(),
    };
    // Other collector messages (containers, real-time stats…) are accepted.
    let Ok(c) = CollectorProc::decode(raw.as_slice()) else {
        return Json(json!({})).into_response();
    };
    if c.host_name.is_empty() || c.processes.is_empty() {
        return Json(json!({})).into_response();
    }
    if let Ok(procs) = parse(&body) {
        let now = crate::intake::logs::now_ms();
        if let Ok(mut live) = LIVE.lock() {
            let e = live
                .entry(c.host_name.clone())
                .or_insert((now, HashMap::new()));
            // A new snapshot starts with its first group.
            if c.group_id <= 1 || now - e.0 > 30_000 {
                e.1.clear();
            }
            e.0 = now;
            e.1.insert(c.group_id, procs);
        }
    }
    Json(json!({})).into_response()
}

#[derive(Debug, Deserialize)]
pub struct Filter {
    pub host: Option<String>,
    pub query: Option<String>,
}

/// `GET /api/ui/processes`: processes of hosts seen in the last 2 minutes.
pub async fn list(State(_st): State<AppState>, Query(f): Query<Filter>) -> ApiResult<Value> {
    let now = crate::intake::logs::now_ms();
    let q = f.query.unwrap_or_default().to_lowercase();
    let live = LIVE.lock().map(|g| g.clone()).unwrap_or_default();
    let mut hosts = Vec::new();
    let mut procs: Vec<Proc> = Vec::new();
    for (host, (ts, groups)) in live {
        if now - ts > 120_000 {
            continue;
        }
        hosts.push(json!({"host": host, "updated": ts}));
        if f.host.as_deref().is_some_and(|h| h != host) {
            continue;
        }
        procs.extend(groups.into_values().flatten().filter(|p| {
            q.is_empty()
                || p.command.to_lowercase().contains(&q)
                || p.user.to_lowercase().contains(&q)
                || p.pid.to_string() == q
        }));
    }
    procs.sort_by(|a, b| b.cpu_pct.total_cmp(&a.cpu_pct).then(b.rss.cmp(&a.rss)));
    let total = procs.len();
    procs.truncate(1000);
    Ok(Json(
        json!({"hosts": hosts, "total": total, "processes": procs}),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collector_payload() {
        let c = CollectorProc {
            host_name: "web-1".into(),
            group_id: 1,
            group_size: 1,
            processes: vec![Process {
                pid: 42,
                command: Some(Command {
                    args: vec!["/usr/bin/python3".into(), "app.py".into()],
                    ppid: 1,
                    ..Default::default()
                }),
                user: Some(ProcessUser {
                    name: "www".into(),
                    uid: 33,
                }),
                memory: Some(MemoryStat {
                    rss: 1000,
                    vms: 2000,
                }),
                cpu: Some(CpuStat {
                    total_pct: 12.5,
                    num_threads: 4,
                    ..Default::default()
                }),
                ..Default::default()
            }],
        };
        let mut msg = vec![3u8, 2, 12, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        msg.extend(zstd::encode_all(c.encode_to_vec().as_slice(), 1).unwrap());
        let p = parse(&msg).unwrap();
        assert_eq!(
            (p[0].pid, p[0].name.as_str(), p[0].user.as_str()),
            (42, "python3", "www")
        );
        assert_eq!(
            (p[0].command.as_str(), p[0].threads),
            ("/usr/bin/python3 app.py", 4)
        );
        assert!(parse(&[3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]).is_err());
    }
}
