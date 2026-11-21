//! Continuous Profiler: the profiles Datadog's profilers upload every minute
//! (multipart, pprof files), through the Agent (`/api/v2/profile`) or
//! straight to the trace port (`/profiling/v1/input`). Each profile is kept
//! as its files and as folded stacks per sample type; the UI merges the
//! folded stacks of a period into a flame graph.

pub mod pprof;

use crate::AppState;
use crate::api::{ApiError, ApiResult};
use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use pprof::Folded;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap};

/// What a profile is about, from its tags.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Meta {
    pub start: i64,
    pub end: i64,
    pub service: String,
    pub env: Option<String>,
    pub version: Option<String>,
    pub host: Option<String>,
    pub language: Option<String>,
    pub runtime_id: Option<String>,
    pub tags: Vec<String>,
}

/// A stored profile, without its data.
#[derive(Debug, Clone, Serialize)]
pub struct ProfileRow {
    pub id: i64,
    #[serde(flatten)]
    pub meta: Meta,
    pub types: Vec<String>,
    pub files: Vec<String>,
    pub size: i64,
}

fn rfc3339_ms(s: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(s.trim())
        .ok()
        .map(|d| d.timestamp_millis())
}

fn tag_value<'a>(tags: &'a [String], key: &str) -> Option<&'a str> {
    tags.iter()
        .find_map(|t| t.strip_prefix(key)?.strip_prefix(':'))
        .filter(|v| !v.is_empty())
}

fn split_tags(s: &str) -> Vec<String> {
    s.split(',')
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(String::from)
        .collect()
}

/// The parts of an upload: its metadata and its pprof files.
pub struct Upload {
    pub meta: Meta,
    pub files: Vec<(String, Vec<u8>)>,
}

/// Reads the v4 format (`event` JSON plus attachments) or the older one
/// (`recording-start`, `tags[]`, `data[0]`…).
pub async fn read_upload(headers: &HeaderMap, body: Bytes) -> Result<Upload, String> {
    let ct = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .ok_or("no content type")?;
    let boundary = multer::parse_boundary(ct).map_err(|e| format!("content type: {e}"))?;
    let stream = futures::stream::once(async move { Ok::<Bytes, std::io::Error>(body) });
    let mut form = multer::Multipart::new(stream, boundary);
    let mut fields: Vec<(String, Option<String>, Bytes)> = Vec::new();
    while let Some(field) = form.next_field().await.map_err(|e| e.to_string())? {
        let name = field.name().unwrap_or_default().to_string();
        let file = field.file_name().map(String::from);
        let data = field.bytes().await.map_err(|e| e.to_string())?;
        fields.push((name, file, data));
    }
    let text = |name: &str| {
        fields
            .iter()
            .filter(|(n, ..)| n == name)
            .map(|(_, _, d)| String::from_utf8_lossy(d).into_owned())
            .collect::<Vec<_>>()
    };
    let mut tags: Vec<String> = Vec::new();
    let mut attachments: Vec<String> = Vec::new();
    let (start, end, family) = if let Some((_, _, ev)) = fields.iter().find(|(n, ..)| n == "event")
    {
        let ev: Value = serde_json::from_slice(ev).map_err(|e| format!("event: {e}"))?;
        tags.extend(split_tags(ev["tags_profiler"].as_str().unwrap_or("")));
        attachments = ev["attachments"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        (
            ev["start"].as_str().and_then(rfc3339_ms),
            ev["end"].as_str().and_then(rfc3339_ms),
            ev["family"].as_str().map(String::from),
        )
    } else {
        for t in text("tags[]") {
            tags.extend(split_tags(&t));
        }
        (
            text("recording-start").first().and_then(|s| rfc3339_ms(s)),
            text("recording-end").first().and_then(|s| rfc3339_ms(s)),
            text("family").into_iter().chain(text("runtime")).next(),
        )
    };
    if let Some(extra) = headers
        .get("x-datadog-additional-tags")
        .and_then(|v| v.to_str().ok())
    {
        tags.extend(split_tags(extra));
    }
    let files: Vec<(String, Vec<u8>)> = fields
        .into_iter()
        .filter(|(n, f, _)| {
            n.starts_with("data[")
                || attachments.contains(n)
                || f.as_ref()
                    .is_some_and(|f| attachments.contains(f) || f.ends_with(".pprof"))
                || n.ends_with(".pprof")
        })
        .map(|(n, f, d)| (f.unwrap_or(n), d.to_vec()))
        .collect();
    if files.is_empty() {
        return Err("no pprof file in the upload".into());
    }
    let now = crate::intake::logs::now_ms();
    let end = end.unwrap_or(now);
    let meta = Meta {
        start: start.unwrap_or(end - 60_000),
        end,
        service: tag_value(&tags, "service")
            .unwrap_or("unnamed-service")
            .to_string(),
        env: tag_value(&tags, "env").map(String::from),
        version: tag_value(&tags, "version").map(String::from),
        host: tag_value(&tags, "host").map(String::from),
        language: tag_value(&tags, "language")
            .or(tag_value(&tags, "runtime"))
            .map(String::from)
            .or(family),
        runtime_id: tag_value(&tags, "runtime-id").map(String::from),
        tags,
    };
    Ok(Upload { meta, files })
}

/// Folds every pprof file of an upload, merging types found twice.
pub fn fold_files(files: &[(String, Vec<u8>)]) -> Result<BTreeMap<String, Folded>, String> {
    let mut out: BTreeMap<String, Folded> = BTreeMap::new();
    let mut decoded = 0;
    let mut last_error = None;
    for (name, data) in files {
        let p = match pprof::decode(data) {
            Ok(p) => p,
            Err(e) => {
                last_error = Some(format!("{name}: {e}"));
                continue;
            }
        };
        decoded += 1;
        for (t, f) in pprof::fold(&p) {
            let e = out.entry(t).or_insert_with(|| Folded {
                unit: f.unit.clone(),
                stacks: Vec::new(),
            });
            e.stacks.extend(f.stacks);
        }
    }
    if decoded == 0 {
        return Err(last_error.unwrap_or_else(|| "no profile".into()));
    }
    Ok(out)
}

pub fn encode_folded(f: &BTreeMap<String, Folded>) -> Vec<u8> {
    let json = serde_json::to_vec(f).unwrap_or_default();
    zstd::encode_all(json.as_slice(), 3).unwrap_or_default()
}

pub fn decode_folded(b: &[u8]) -> BTreeMap<String, Folded> {
    zstd::decode_all(b)
        .ok()
        .and_then(|j| serde_json::from_slice(&j).ok())
        .unwrap_or_default()
}

async fn store(st: &AppState, headers: &HeaderMap, body: Bytes) -> Result<i64, String> {
    let up = read_upload(headers, body).await?;
    let folded = fold_files(&up.files)?;
    let types: Vec<String> = folded.keys().cloned().collect();
    st.store
        .pg
        .put_profile(&up.meta, &types, &encode_folded(&folded), &up.files)
        .await
        .map_err(|e| e.to_string())
}

/// `POST /api/v2/profile` (the Agent) and `/profiling/v1/input` (tracers).
pub async fn intake(State(st): State<AppState>, headers: HeaderMap, body: Bytes) -> Response {
    match store(&st, &headers, body).await {
        Ok(id) => {
            tracing::debug!(id, "profile received");
            (StatusCode::OK, Json(json!({}))).into_response()
        }
        Err(e) => {
            tracing::warn!(error = %e, "profile rejected");
            (StatusCode::BAD_REQUEST, Json(json!({"errors": [e]}))).into_response()
        }
    }
}

/// Removes profiles past `ROBINE_DOG_PROFILE_RETENTION_DAYS` (default 8, as
/// Datadog's).
pub fn spawn_purge(st: AppState) {
    let days: i64 = std::env::var("ROBINE_DOG_PROFILE_RETENTION_DAYS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(8);
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(3600));
        loop {
            tick.tick().await;
            let before = crate::intake::logs::now_ms() - days * 86_400_000;
            match st.store.pg.purge_profiles(before).await {
                Ok(n) if n > 0 => tracing::info!(n, "purged profiles"),
                Ok(_) => {}
                Err(e) => tracing::warn!(error = %e, "purging profiles"),
            }
        }
    });
}

// ---------------------------------------------------------------- queries

#[derive(Debug, Deserialize, Default)]
pub struct Filter {
    pub service: Option<String>,
    pub env: Option<String>,
    pub version: Option<String>,
    pub host: Option<String>,
    pub from: i64,
    pub to: i64,
}

/// `GET /api/ui/profiling/profiles`.
pub async fn list(
    State(st): State<AppState>,
    Query(f): Query<Filter>,
) -> ApiResult<Vec<ProfileRow>> {
    Ok(Json(st.store.pg.profiles(&f, 200).await?))
}

/// Profiles, sample types, languages and versions of a service.
type ServiceAcc = (
    usize,
    BTreeMap<String, ()>,
    BTreeMap<String, ()>,
    BTreeMap<String, ()>,
);

/// `GET /api/ui/profiling/services`: services with profiles in the range.
pub async fn services(State(st): State<AppState>, Query(f): Query<Filter>) -> ApiResult<Value> {
    let rows = st
        .store
        .pg
        .profiles(&Filter { service: None, ..f }, 5000)
        .await?;
    let mut by: BTreeMap<String, ServiceAcc> = BTreeMap::new();
    for r in rows {
        let e = by.entry(r.meta.service.clone()).or_default();
        e.0 += 1;
        for t in r.types {
            e.1.insert(t, ());
        }
        if let Some(l) = r.meta.language {
            e.2.insert(l, ());
        }
        if let Some(v) = r.meta.version {
            e.3.insert(v, ());
        }
    }
    Ok(Json(json!(
        by.into_iter()
            .map(|(s, (n, t, l, v))| json!({
                "service": s, "profiles": n,
                "types": t.into_keys().collect::<Vec<_>>(),
                "languages": l.into_keys().collect::<Vec<_>>(),
                "versions": v.into_keys().collect::<Vec<_>>(),
            }))
            .collect::<Vec<_>>()
    )))
}

/// A flame graph node: a frame, its total and self values.
#[derive(Debug, Clone, Serialize, Default, PartialEq)]
pub struct Node {
    pub name: String,
    pub value: i64,
    #[serde(rename = "self")]
    pub self_value: i64,
    pub children: Vec<Node>,
}

impl Node {
    fn add(&mut self, frames: &[&str], v: i64) {
        self.value += v;
        match frames.split_first() {
            None => self.self_value += v,
            Some((f, rest)) => {
                let i = match self.children.iter().position(|c| c.name == *f) {
                    Some(i) => i,
                    None => {
                        self.children.push(Node {
                            name: f.to_string(),
                            ..Default::default()
                        });
                        self.children.len() - 1
                    }
                };
                self.children[i].add(rest, v);
            }
        }
    }

    /// Drops nodes under `min`, children sorted by value.
    fn prune(&mut self, min: i64) {
        self.children.retain(|c| c.value >= min);
        self.children.sort_by_key(|a| std::cmp::Reverse(a.value));
        for c in &mut self.children {
            c.prune(min);
        }
    }
}

/// The tree, top functions `(name, self, total)`, endpoints and unit.
pub type Merged = (Node, Vec<(String, i64, i64)>, Vec<(String, i64)>, String);

/// Merged stacks of one type over profiles: the tree, top functions by self
/// value, and values by endpoint.
pub fn merge(
    profiles: &[BTreeMap<String, Folded>],
    sample_type: &str,
    endpoint: Option<&str>,
) -> Merged {
    let mut root = Node {
        name: "all".into(),
        ..Default::default()
    };
    let mut endpoints: HashMap<String, i64> = HashMap::new();
    let mut unit = String::new();
    let mut with_type = 0;
    let mut stacks: HashMap<&str, i64> = HashMap::new();
    for p in profiles {
        let Some(f) = p.get(sample_type) else {
            continue;
        };
        with_type += 1;
        unit = f.unit.clone();
        for (e, s, v) in &f.stacks {
            *endpoints.entry(e.clone()).or_default() += v;
            if endpoint.is_none_or(|want| want == e) {
                *stacks.entry(s.as_str()).or_default() += v;
            }
        }
    }
    // Snapshots (heap in use) are averaged, deltas summed.
    let div = if pprof::is_snapshot(sample_type) {
        with_type.max(1)
    } else {
        1
    };
    let mut self_by: HashMap<&str, i64> = HashMap::new();
    let mut total_by: HashMap<&str, i64> = HashMap::new();
    for (s, v) in &stacks {
        let v = v / div;
        let frames: Vec<&str> = s.split(';').collect();
        root.add(&frames, v);
        if let Some(leaf) = frames.last() {
            *self_by.entry(leaf).or_default() += v;
        }
        let mut seen: Vec<&str> = Vec::new();
        for f in &frames {
            if !seen.contains(f) {
                *total_by.entry(f).or_default() += v;
                seen.push(f);
            }
        }
    }
    root.prune((root.value / 1000).max(1));
    let mut top: Vec<(String, i64, i64)> = self_by
        .iter()
        .map(|(f, v)| (f.to_string(), *v, total_by.get(f).copied().unwrap_or(*v)))
        .collect();
    top.sort_by_key(|a| std::cmp::Reverse(a.1));
    top.truncate(50);
    let mut endpoints: Vec<(String, i64)> = endpoints
        .into_iter()
        .filter(|(e, _)| !e.is_empty())
        .map(|(e, v)| (e, v / div))
        .collect();
    endpoints.sort_by_key(|a| std::cmp::Reverse(a.1));
    endpoints.truncate(30);
    (root, top, endpoints, unit)
}

#[derive(Debug, Deserialize)]
pub struct FlameReq {
    #[serde(flatten)]
    pub filter: Filter,
    #[serde(rename = "type")]
    pub sample_type: Option<String>,
    pub endpoint: Option<String>,
    pub profile_id: Option<i64>,
}

/// `POST /api/ui/profiling/flamegraph`: the profiles of a period merged, or
/// one profile.
pub async fn flamegraph(State(st): State<AppState>, Json(r): Json<FlameReq>) -> ApiResult<Value> {
    let blobs = match r.profile_id {
        Some(id) => st.store.pg.profile_folded(&[id]).await?,
        None => {
            let ids: Vec<i64> = st
                .store
                .pg
                .profiles(&r.filter, 300)
                .await?
                .iter()
                .map(|p| p.id)
                .collect();
            st.store.pg.profile_folded(&ids).await?
        }
    };
    let profiles: Vec<BTreeMap<String, Folded>> = blobs.iter().map(|b| decode_folded(b)).collect();
    let mut types: BTreeMap<String, String> = BTreeMap::new();
    for p in &profiles {
        for (t, f) in p {
            types.insert(t.clone(), f.unit.clone());
        }
    }
    // CPU first, as Datadog's default view.
    let preferred = ["cpu-time", "cpu", "samples", "wall-time", "wall"];
    let sample_type = r
        .sample_type
        .filter(|t| types.contains_key(t))
        .or_else(|| {
            preferred
                .iter()
                .find(|p| types.contains_key(**p))
                .map(|s| s.to_string())
        })
        .or_else(|| types.keys().next().cloned())
        .unwrap_or_default();
    let (tree, top, endpoints, unit) = merge(&profiles, &sample_type, r.endpoint.as_deref());
    Ok(Json(json!({
        "type": sample_type, "unit": unit, "profiles": profiles.len(),
        "types": types, "tree": tree,
        "top": top.iter().map(|(n, s, t)| json!({"name": n, "self": s, "total": t})).collect::<Vec<_>>(),
        "endpoints": endpoints.iter().map(|(e, v)| json!({"endpoint": e, "value": v})).collect::<Vec<_>>(),
    })))
}

/// `GET /api/ui/profiling/profiles/{id}/files/{name}`: an uploaded file.
pub async fn download(
    State(st): State<AppState>,
    Path((id, name)): Path<(i64, String)>,
) -> Result<Response, ApiError> {
    let data = st
        .store
        .pg
        .profile_file(id, &name)
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok((
        [
            (header::CONTENT_TYPE, "application/octet-stream".to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{}-{}\"", id, name.replace('"', "")),
            ),
        ],
        data,
    )
        .into_response())
}

#[cfg(test)]
mod tests {
    use super::*;
    use prost::Message;

    #[test]
    fn merges_profiles_into_a_tree() {
        let p = pprof::tests::sample_profile();
        let files = vec![("cpu.pprof".to_string(), p.encode_to_vec())];
        let f = fold_files(&files).unwrap();
        let blob = encode_folded(&f);
        let back = decode_folded(&blob);
        assert_eq!(back, f);
        let (tree, top, endpoints, unit) = merge(&[back.clone(), back], "cpu-time", None);
        assert_eq!(unit, "nanoseconds");
        assert_eq!(tree.value, 800);
        let main = &tree.children[0];
        assert_eq!(main.name, "main (app.py)");
        assert_eq!(main.children[0].name, "handle (app.py)");
        assert_eq!(main.children[0].value, 600);
        assert_eq!(top[0], ("query (db.py)".into(), 600, 600));
        assert_eq!(endpoints, vec![("GET /products".into(), 600)]);
        let (only, ..) = merge(&[f], "cpu-time", Some("GET /products"));
        assert_eq!(only.value, 300);
        assert!(fold_files(&[("x".into(), b"nope".to_vec())]).is_err());
    }

    #[tokio::test]
    async fn reads_v4_and_legacy_uploads() {
        let pprof = pprof::tests::sample_profile().encode_to_vec();
        let body = |parts: Vec<(&str, Option<&str>, Vec<u8>)>| {
            let mut b = Vec::new();
            for (name, file, data) in parts {
                b.extend(b"--XYZ\r\n");
                match file {
                    Some(f) => b.extend(
                        format!("Content-Disposition: form-data; name=\"{name}\"; filename=\"{f}\"\r\n\r\n").as_bytes(),
                    ),
                    None => b.extend(format!("Content-Disposition: form-data; name=\"{name}\"\r\n\r\n").as_bytes()),
                }
                b.extend(data);
                b.extend(b"\r\n");
            }
            b.extend(b"--XYZ--\r\n");
            Bytes::from(b)
        };
        let mut h = HeaderMap::new();
        h.insert(
            header::CONTENT_TYPE,
            "multipart/form-data; boundary=XYZ".parse().unwrap(),
        );
        h.insert("x-datadog-additional-tags", "host:web-1".parse().unwrap());
        let event = json!({"attachments": ["auto.pprof"], "tags_profiler": "service:shop,env:prod,version:1.2,runtime-id:abc",
            "start": "2026-11-21T10:00:00Z", "end": "2026-11-21T10:01:00Z", "family": "python", "version": "4"});
        let up = read_upload(
            &h,
            body(vec![
                ("event", Some("event.json"), event.to_string().into_bytes()),
                ("auto.pprof", Some("auto.pprof"), pprof.clone()),
            ]),
        )
        .await
        .unwrap();
        assert_eq!(up.meta.service, "shop");
        assert_eq!(up.meta.env.as_deref(), Some("prod"));
        assert_eq!(up.meta.host.as_deref(), Some("web-1"));
        assert_eq!(up.meta.language.as_deref(), Some("python"));
        assert_eq!(up.meta.end - up.meta.start, 60_000);
        assert_eq!(up.files[0].0, "auto.pprof");

        let legacy = read_upload(
            &h,
            body(vec![
                ("recording-start", None, b"2026-11-21T10:00:00Z".to_vec()),
                ("recording-end", None, b"2026-11-21T10:01:00Z".to_vec()),
                ("tags[]", None, b"service:api".to_vec()),
                ("tags[]", None, b"runtime:go".to_vec()),
                ("data[0]", Some("pprof-data"), pprof),
            ]),
        )
        .await
        .unwrap();
        assert_eq!(legacy.meta.service, "api");
        assert_eq!(legacy.meta.language.as_deref(), Some("go"));
        assert_eq!(legacy.files.len(), 1);
    }
}
