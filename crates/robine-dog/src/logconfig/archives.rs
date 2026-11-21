//! Log archives, as Datadog's: every log processed (excluded from the
//! indexes or not) goes to the first archive whose query matches it, written
//! every few minutes as gzipped JSON lines in Datadog's layout,
//! `<path>/dt=YYYYMMDD/hour=HH/archive_HHMMSS.SSSS.<id>.json.gz`, to an
//! S3-compatible bucket (credentials from `AWS_*`, an `endpoint` for MinIO
//! and the like) or a local directory (`"type": "local"`).
//!
//! Rehydration reads an archive back over a time range, keeps the logs a
//! query matches and stores them in a historical view: a table of its own,
//! searched with `index:<view>`, and dropped with the view.

use crate::AppState;
use crate::api::{ApiError, ApiResult};
use crate::intake::logs::{LogRow, now_ms};
use crate::query::matcher::matches;
use crate::query::parser::Expr;
use crate::store::pg::new_id;
use axum::Json;
use axum::extract::{Path, State};
use flate2::Compression;
use flate2::write::GzEncoder;
use futures::StreamExt;
use object_store::ObjectStore;
use object_store::path::Path as ObjPath;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, HashMap};
use std::io::Write;
use std::sync::{Arc, LazyLock, Mutex, RwLock};
use std::time::Duration;

pub const KIND: &str = "log_archive";
pub const ORDER_KIND: &str = "log_archive_order";
pub const VIEW_KIND: &str = "log_rehydration";
const HOUR: i64 = 3_600_000;
/// Buffered bytes past which an archive is written before its time.
const FLUSH_BYTES: usize = 64 << 20;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Destination {
    /// `s3` or `local`.
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bucket: Option<String>,
    #[serde(default)]
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
    /// An S3-compatible endpoint (MinIO, Ceph, Garage…).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    /// Datadog's AWS account and role: kept, unused (credentials come from
    /// the environment).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub integration: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ArchiveDef {
    pub name: String,
    #[serde(default)]
    pub query: String,
    pub destination: Destination,
    #[serde(default = "yes")]
    pub include_tags: bool,
    #[serde(default)]
    pub rehydration_tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rehydration_max_scan_size_in_gb: Option<i64>,
}

fn yes() -> bool {
    true
}

/// The store and the key prefix of a destination.
pub fn open(d: &Destination) -> Result<(Arc<dyn ObjectStore>, String), String> {
    let prefix = d.path.trim_matches('/').to_string();
    match d.kind.as_str() {
        "s3" => {
            let bucket = d
                .bucket
                .as_deref()
                .filter(|b| !b.is_empty())
                .ok_or("destination.bucket is missing")?;
            // Few retries: a wrong destination fails in seconds, not minutes.
            let retry = object_store::RetryConfig {
                max_retries: 2,
                retry_timeout: Duration::from_secs(20),
                ..Default::default()
            };
            let mut b = object_store::aws::AmazonS3Builder::from_env()
                .with_bucket_name(bucket)
                .with_retry(retry);
            if let Some(r) = &d.region {
                b = b.with_region(r);
            }
            if let Some(e) = d.endpoint.as_deref().filter(|e| !e.is_empty()) {
                b = b
                    .with_endpoint(e)
                    .with_allow_http(e.starts_with("http://"))
                    .with_virtual_hosted_style_request(false);
            }
            Ok((Arc::new(b.build().map_err(|e| e.to_string())?), prefix))
        }
        "local" => {
            if !d.path.starts_with('/') {
                return Err("destination.path is an absolute directory".into());
            }
            std::fs::create_dir_all(&d.path).map_err(|e| format!("{}: {e}", d.path))?;
            Ok((
                Arc::new(
                    object_store::local::LocalFileSystem::new_with_prefix(&d.path)
                        .map_err(|e| e.to_string())?,
                ),
                String::new(),
            ))
        }
        "gcs" | "azure" => Err(format!(
            "{} destinations are not supported: s3 (or S3-compatible) and local are",
            d.kind
        )),
        k => Err(format!("unknown destination type {k}")),
    }
}

fn join(prefix: &str, rest: &str) -> ObjPath {
    if prefix.is_empty() {
        ObjPath::from(rest)
    } else {
        ObjPath::from(format!("{prefix}/{rest}"))
    }
}

fn hour_dir(ts: i64) -> String {
    let d = chrono::DateTime::from_timestamp_millis(ts).unwrap_or_default();
    format!("dt={}/hour={}", d.format("%Y%m%d"), d.format("%H"))
}

fn iso(ms: i64) -> String {
    super::iso(ms)
}

/// A log as a line of a Datadog archive.
pub fn to_line(l: &LogRow, include_tags: bool) -> String {
    let mut o = Map::new();
    o.insert("_id".into(), json!(new_id()));
    o.insert("date".into(), json!(iso(l.ts)));
    o.insert("host".into(), json!(l.host));
    o.insert("service".into(), json!(l.service));
    o.insert("source".into(), json!(l.source));
    o.insert("status".into(), json!(l.status));
    o.insert("message".into(), json!(l.message));
    if include_tags {
        o.insert(
            "tags".into(),
            json!(
                l.tags
                    .iter()
                    .filter(|t| !t.starts_with("index:"))
                    .collect::<Vec<_>>()
            ),
        );
    }
    if let Some(t) = &l.trace_id {
        o.insert("trace_id".into(), json!(t));
    }
    if let Some(s) = &l.span_id {
        o.insert("span_id".into(), json!(s));
    }
    o.insert("attributes".into(), Value::Object(l.attributes.clone()));
    Value::Object(o).to_string()
}

/// A line of a Datadog archive as a log.
pub fn from_line(line: &str) -> Option<LogRow> {
    let v: Value = serde_json::from_str(line).ok()?;
    let ts = v["date"]
        .as_str()
        .and_then(|d| chrono::DateTime::parse_from_rfc3339(d).ok())
        .map(|d| d.timestamp_millis())
        .or_else(|| v["date"].as_i64())?;
    let s = |k: &str| v[k].as_str().map(String::from);
    Some(LogRow {
        ts,
        status: s("status").unwrap_or_else(|| "info".into()),
        service: s("service"),
        host: s("host"),
        source: s("source"),
        message: s("message").unwrap_or_default(),
        tags: v["tags"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|t| t.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default(),
        attributes: v["attributes"].as_object().cloned().unwrap_or_default(),
        trace_id: s("trace_id").or_else(|| {
            v.pointer("/attributes/dd/trace_id")
                .and_then(|x| x.as_str())
                .map(String::from)
        }),
        span_id: s("span_id"),
    })
}

struct Compiled {
    id: String,
    filter: Expr,
    include_tags: bool,
    store: Arc<dyn ObjectStore>,
    prefix: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Status {
    /// `WORKING`, `FAILING` or `DISABLED`, as Datadog's `state`.
    pub state: String,
    pub archived: u64,
    pub files: u64,
    pub bytes: u64,
    pub last_flush: Option<i64>,
    pub last_error: Option<String>,
}

/// An archive's lines waiting to be written, by hour directory, and their size.
type Buffer = (BTreeMap<String, Vec<String>>, usize);

#[derive(Default)]
pub struct Archives {
    list: RwLock<Arc<Vec<Compiled>>>,
    /// Per archive, lines by hour directory.
    buffers: Mutex<HashMap<String, Buffer>>,
    status: Mutex<HashMap<String, Status>>,
    flushing: tokio::sync::Mutex<()>,
}

impl Archives {
    fn load(&self, defs: &[(String, ArchiveDef)]) {
        let mut list = Vec::new();
        let mut status = self.status.lock().unwrap_or_else(|e| e.into_inner());
        for (id, d) in defs {
            let filter = match crate::query::parser::parse(&d.query) {
                Ok(f) => f,
                Err(_) => continue,
            };
            match open(&d.destination) {
                Ok((store, prefix)) => {
                    let s = status.entry(id.clone()).or_default();
                    if s.state != "FAILING" {
                        s.state = "WORKING".into();
                    }
                    list.push(Compiled {
                        id: id.clone(),
                        filter,
                        include_tags: d.include_tags,
                        store,
                        prefix,
                    });
                }
                Err(e) => {
                    let s = status.entry(id.clone()).or_default();
                    s.state = "FAILING".into();
                    s.last_error = Some(e);
                }
            }
        }
        *self.list.write().unwrap_or_else(|e| e.into_inner()) = Arc::new(list);
    }

    pub fn status(&self, id: &str) -> Status {
        self.status
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(id)
            .cloned()
            .unwrap_or_else(|| Status {
                state: "WORKING".into(),
                ..Default::default()
            })
    }

    /// Buffers a log for the first archive matching it; true when the
    /// buffers are big enough to write.
    pub fn add(&self, l: &LogRow) -> bool {
        let list = self.list.read().unwrap_or_else(|e| e.into_inner()).clone();
        let Some(a) = list.iter().find(|a| matches(&a.filter, l)) else {
            return false;
        };
        let line = to_line(l, a.include_tags);
        let mut b = self.buffers.lock().unwrap_or_else(|e| e.into_inner());
        let (hours, bytes) = b.entry(a.id.clone()).or_default();
        *bytes += line.len() + 1;
        hours.entry(hour_dir(l.ts)).or_default().push(line);
        *bytes >= FLUSH_BYTES
    }

    /// Writes what is buffered: a file per archive and hour.
    pub async fn flush(&self) {
        let _one = self.flushing.lock().await;
        let taken = std::mem::take(&mut *self.buffers.lock().unwrap_or_else(|e| e.into_inner()));
        let list = self.list.read().unwrap_or_else(|e| e.into_inner()).clone();
        let now = now_ms();
        for (id, (hours, _)) in taken {
            let Some(a) = list.iter().find(|a| a.id == id) else {
                continue;
            };
            for (dir, lines) in hours {
                let mut gz = GzEncoder::new(Vec::new(), Compression::default());
                for l in &lines {
                    let _ = gz.write_all(l.as_bytes());
                    let _ = gz.write_all(b"\n");
                }
                let Ok(body) = gz.finish() else { continue };
                let t = chrono::DateTime::from_timestamp_millis(now).unwrap_or_default();
                let name = format!(
                    "{dir}/archive_{}.{:04}.{}.json.gz",
                    t.format("%H%M%S"),
                    (now % 1000) * 10,
                    new_id()
                );
                let size = body.len();
                let res = a.store.put(&join(&a.prefix, &name), body.into()).await;
                let mut st = self.status.lock().unwrap_or_else(|e| e.into_inner());
                let s = st.entry(id.clone()).or_default();
                match res {
                    Ok(_) => {
                        s.state = "WORKING".into();
                        s.archived += lines.len() as u64;
                        s.files += 1;
                        s.bytes += size as u64;
                        s.last_flush = Some(now);
                        s.last_error = None;
                    }
                    Err(e) => {
                        tracing::warn!(archive = %id, error = %e, "archive write failed");
                        s.state = "FAILING".into();
                        s.last_error = Some(e.to_string());
                    }
                }
            }
        }
    }
}

/// Writes the archives every `ROBINE_DOG_ARCHIVE_FLUSH_SECONDS` (300).
pub fn spawn(st: AppState) {
    let every = std::env::var("ROBINE_DOG_ARCHIVE_FLUSH_SECONDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(300u64)
        .max(1);
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(every));
        tick.tick().await;
        loop {
            tick.tick().await;
            st.logconfig.archives.flush().await;
        }
    });
}

// --- API: `/api/v2/logs/config/archives` -------------------------------------

async fn stored(st: &AppState) -> anyhow::Result<Vec<(String, ArchiveDef)>> {
    let mut defs: Vec<(String, ArchiveDef)> = st
        .store
        .pg
        .docs(KIND)
        .await?
        .into_iter()
        .filter_map(|d| Some((d.id, serde_json::from_value(d.body).ok()?)))
        .collect();
    let order = order(st).await?;
    defs.sort_by_key(|(id, _)| order.iter().position(|o| o == id).unwrap_or(usize::MAX));
    Ok(defs)
}

async fn order(st: &AppState) -> anyhow::Result<Vec<String>> {
    Ok(st
        .store
        .pg
        .doc(ORDER_KIND, "order")
        .await?
        .and_then(|d| serde_json::from_value(d.body["archive_ids"].clone()).ok())
        .unwrap_or_default())
}

async fn set_order_ids(st: &AppState, ids: &[String]) -> anyhow::Result<()> {
    st.store
        .pg
        .put_doc(ORDER_KIND, "order", &json!({"archive_ids": ids}))
        .await?;
    Ok(())
}

pub async fn reload(st: &AppState) -> anyhow::Result<()> {
    st.logconfig.archives.load(&stored(st).await?);
    Ok(())
}

fn out(st: &AppState, id: &str, d: &ArchiveDef) -> Value {
    let s = st.logconfig.archives.status(id);
    let mut attrs = serde_json::to_value(d).unwrap_or_default();
    attrs["state"] = json!(s.state);
    attrs["status"] = json!(s);
    json!({"type": "archives", "id": id, "attributes": attrs})
}

/// `GET /api/v2/logs/config/archives`.
pub async fn list(State(st): State<AppState>) -> ApiResult<Value> {
    let defs = stored(&st).await?;
    Ok(Json(
        json!({"data": defs.iter().map(|(id, d)| out(&st, id, d)).collect::<Vec<_>>()}),
    ))
}

/// `GET /api/v2/logs/config/archives/{id}`.
pub async fn get(State(st): State<AppState>, Path(id): Path<String>) -> ApiResult<Value> {
    let (id, d) = stored(&st)
        .await?
        .into_iter()
        .find(|(i, _)| *i == id)
        .ok_or(ApiError::NotFound)?;
    Ok(Json(json!({"data": out(&st, &id, &d)})))
}

/// Checks the destination by writing a small object to it, as Datadog does.
async fn check(d: &ArchiveDef) -> Result<(), ApiError> {
    if d.name.trim().is_empty() {
        return Err(ApiError::BadQuery("the archive has no name".into()));
    }
    crate::api::parse_query(&d.query)?;
    let (store, prefix) = open(&d.destination).map_err(ApiError::BadQuery)?;
    store
        .put(&join(&prefix, ".robine-dog-check"), b"ok".to_vec().into())
        .await
        .map_err(|e| ApiError::BadQuery(format!("cannot write to the destination: {e}")))?;
    Ok(())
}

fn read(b: Value) -> Result<ArchiveDef, ApiError> {
    serde_json::from_value(b["data"]["attributes"].clone())
        .map_err(|e| ApiError::BadQuery(format!("data.attributes: {e}")))
}

/// `POST /api/v2/logs/config/archives`: added last in the order.
pub async fn create(State(st): State<AppState>, Json(b): Json<Value>) -> ApiResult<Value> {
    let d = read(b)?;
    check(&d).await?;
    let id = new_id();
    st.store
        .pg
        .put_doc(
            KIND,
            &id,
            &serde_json::to_value(&d).map_err(anyhow::Error::from)?,
        )
        .await?;
    let mut o = order(&st).await?;
    o.push(id.clone());
    set_order_ids(&st, &o).await?;
    reload(&st).await?;
    Ok(Json(json!({"data": out(&st, &id, &d)})))
}

/// `PUT /api/v2/logs/config/archives/{id}`.
pub async fn update(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Json(b): Json<Value>,
) -> ApiResult<Value> {
    if st.store.pg.doc(KIND, &id).await?.is_none() {
        return Err(ApiError::NotFound);
    }
    let d = read(b)?;
    check(&d).await?;
    // What is buffered goes to the old destination first.
    st.logconfig.archives.flush().await;
    st.store
        .pg
        .put_doc(
            KIND,
            &id,
            &serde_json::to_value(&d).map_err(anyhow::Error::from)?,
        )
        .await?;
    reload(&st).await?;
    Ok(Json(json!({"data": out(&st, &id, &d)})))
}

/// `DELETE /api/v2/logs/config/archives/{id}`: the files stay.
pub async fn delete(State(st): State<AppState>, Path(id): Path<String>) -> ApiResult<Value> {
    st.logconfig.archives.flush().await;
    if !st.store.pg.delete_doc(KIND, &id).await? {
        return Err(ApiError::NotFound);
    }
    let mut o = order(&st).await?;
    o.retain(|x| *x != id);
    set_order_ids(&st, &o).await?;
    reload(&st).await?;
    Ok(Json(json!({})))
}

/// `GET /api/v2/logs/config/archive-order`.
pub async fn get_order(State(st): State<AppState>) -> ApiResult<Value> {
    let ids: Vec<String> = stored(&st).await?.into_iter().map(|(id, _)| id).collect();
    Ok(Json(
        json!({"data": {"type": "archive_order", "attributes": {"archive_ids": ids}}}),
    ))
}

/// `PUT /api/v2/logs/config/archive-order`.
pub async fn set_order(State(st): State<AppState>, Json(b): Json<Value>) -> ApiResult<Value> {
    let ids: Vec<String> =
        serde_json::from_value(b["data"]["attributes"]["archive_ids"].clone())
            .map_err(|e| ApiError::BadQuery(format!("data.attributes.archive_ids: {e}")))?;
    let mut have: Vec<String> = stored(&st).await?.into_iter().map(|(id, _)| id).collect();
    let mut want = ids.clone();
    have.sort();
    want.sort();
    if have != want {
        return Err(ApiError::BadQuery(
            "archive_ids lists every archive once".into(),
        ));
    }
    set_order_ids(&st, &ids).await?;
    reload(&st).await?;
    Ok(Json(
        json!({"data": {"type": "archive_order", "attributes": {"archive_ids": ids}}}),
    ))
}

// --- Rehydration -------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct View {
    pub id: String,
    pub name: String,
    pub archive_id: String,
    #[serde(default)]
    pub query: String,
    pub from: i64,
    pub to: i64,
    #[serde(default)]
    pub max_logs: Option<u64>,
    /// `running`, `done` or `failed`.
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub files: u64,
    #[serde(default)]
    pub scanned_bytes: u64,
    #[serde(default)]
    pub scanned_logs: u64,
    #[serde(default)]
    pub rehydrated: u64,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub finished_at: Option<i64>,
}

impl View {
    pub fn table(&self) -> String {
        format!(
            "logs_rh_{}",
            self.id
                .to_lowercase()
                .replace(|c: char| !c.is_ascii_alphanumeric(), "")
        )
    }
}

/// Historical views by name, for the log searches.
static VIEWS: LazyLock<RwLock<HashMap<String, String>>> = LazyLock::new(Default::default);
static INDEX_TERM: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:^|[\s(])index:([\w\-]+)").expect("regex"));

/// The table of the historical view a query names with `index:<view>`.
pub fn view_table(query: &str) -> Option<String> {
    let views = VIEWS.read().unwrap_or_else(|e| e.into_inner());
    if views.is_empty() {
        return None;
    }
    INDEX_TERM
        .captures_iter(query)
        .find_map(|c| views.get(&c[1]).cloned())
}

async fn views(st: &AppState) -> anyhow::Result<Vec<View>> {
    let mut v: Vec<View> = st
        .store
        .pg
        .docs(VIEW_KIND)
        .await?
        .into_iter()
        .filter_map(|d| serde_json::from_value(d.body).ok())
        .collect();
    v.sort_by_key(|x| std::cmp::Reverse(x.created_at));
    Ok(v)
}

pub async fn reload_views(st: &AppState) -> anyhow::Result<()> {
    let v = views(st).await?;
    *VIEWS.write().unwrap_or_else(|e| e.into_inner()) =
        v.iter().map(|x| (x.name.clone(), x.table())).collect();
    // Views interrupted by a restart.
    for mut x in v.into_iter().filter(|x| x.status == "running") {
        x.status = "failed".into();
        x.error = Some("interrupted by a restart".into());
        st.store
            .pg
            .put_doc(VIEW_KIND, &x.id, &serde_json::to_value(&x)?)
            .await?;
    }
    Ok(())
}

/// `GET /api/ui/logs/rehydrations`.
pub async fn list_views(State(st): State<AppState>) -> ApiResult<Vec<View>> {
    Ok(Json(views(&st).await?))
}

#[derive(Deserialize)]
pub struct NewView {
    name: String,
    archive_id: String,
    #[serde(default)]
    query: String,
    from: i64,
    to: i64,
    #[serde(default)]
    max_logs: Option<u64>,
}

/// `POST /api/ui/logs/rehydrations`: starts reading the archive.
pub async fn create_view(State(st): State<AppState>, Json(n): Json<NewView>) -> ApiResult<View> {
    if n.name.is_empty()
        || !n
            .name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(ApiError::BadQuery(
            "the name is letters, digits, - and _".into(),
        ));
    }
    if views(&st).await?.iter().any(|v| v.name == n.name) {
        return Err(ApiError::BadQuery(format!(
            "a historical view {} exists",
            n.name
        )));
    }
    if n.to <= n.from {
        return Err(ApiError::BadQuery("the range is empty".into()));
    }
    if (n.to - n.from) / HOUR > 24 * 31 {
        return Err(ApiError::BadQuery("at most 31 days at once".into()));
    }
    let filter = crate::api::parse_query(&n.query)?;
    let (_, def) = stored(&st)
        .await?
        .into_iter()
        .find(|(id, _)| *id == n.archive_id)
        .ok_or_else(|| ApiError::BadQuery("no such archive".into()))?;
    let (store, prefix) = open(&def.destination).map_err(ApiError::BadQuery)?;
    let v = View {
        id: new_id(),
        name: n.name,
        archive_id: n.archive_id,
        query: n.query,
        from: n.from,
        to: n.to,
        max_logs: n.max_logs,
        status: "running".into(),
        files: 0,
        scanned_bytes: 0,
        scanned_logs: 0,
        rehydrated: 0,
        error: None,
        created_at: now_ms(),
        finished_at: None,
    };
    let table = v.table();
    st.store
        .gt
        .exec(&crate::store::greptime::logs_like(&table))
        .await?;
    st.store
        .pg
        .put_doc(
            VIEW_KIND,
            &v.id,
            &serde_json::to_value(&v).map_err(anyhow::Error::from)?,
        )
        .await?;
    VIEWS
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .insert(v.name.clone(), table.clone());
    let max_bytes = def
        .rehydration_max_scan_size_in_gb
        .map(|g| (g as u64) << 30);
    let tags = def.rehydration_tags.clone();
    let (st2, v2) = (st.clone(), v.clone());
    tokio::spawn(async move {
        let mut v = v2;
        let res = rehydrate(
            &st2, &mut v, store, &prefix, &filter, &table, max_bytes, &tags,
        )
        .await;
        v.status = if res.is_ok() {
            "done".into()
        } else {
            "failed".into()
        };
        v.error = res.err().map(|e| e.to_string());
        v.finished_at = Some(now_ms());
        if let Ok(body) = serde_json::to_value(&v) {
            let _ = st2.store.pg.put_doc(VIEW_KIND, &v.id, &body).await;
        }
    });
    Ok(Json(v))
}

#[allow(clippy::too_many_arguments)]
async fn rehydrate(
    st: &AppState,
    v: &mut View,
    store: Arc<dyn ObjectStore>,
    prefix: &str,
    filter: &Expr,
    table: &str,
    max_bytes: Option<u64>,
    tags: &[String],
) -> anyhow::Result<()> {
    use std::io::Read;
    let max_logs = v.max_logs.unwrap_or(10_000_000);
    let mut hour = v.from.div_euclid(HOUR) * HOUR;
    let mut batch: Vec<LogRow> = Vec::new();
    let mut last_save = now_ms();
    while hour <= v.to {
        let dir = join(prefix, &hour_dir(hour));
        let mut files = store.list(Some(&dir));
        let mut names = Vec::new();
        while let Some(m) = files.next().await {
            names.push(m?.location);
        }
        names.sort();
        for path in names
            .into_iter()
            .filter(|p| p.as_ref().ends_with(".json.gz"))
        {
            let bytes = store.get(&path).await?.bytes().await?;
            v.files += 1;
            v.scanned_bytes += bytes.len() as u64;
            if max_bytes.is_some_and(|m| v.scanned_bytes > m) {
                anyhow::bail!("stopped at the archive's maximum scan size");
            }
            let mut text = String::new();
            flate2::read::MultiGzDecoder::new(&bytes[..]).read_to_string(&mut text)?;
            for line in text.lines() {
                v.scanned_logs += 1;
                let Some(mut l) = from_line(line) else {
                    continue;
                };
                if l.ts < v.from || l.ts > v.to || !matches(filter, &l) {
                    continue;
                }
                l.tags.retain(|t| !t.starts_with("index:"));
                l.tags.push(format!("index:{}", v.name));
                l.tags.extend(tags.iter().cloned());
                batch.push(l);
                v.rehydrated += 1;
                if batch.len() >= 5_000 {
                    insert(st, table, std::mem::take(&mut batch)).await?;
                }
                if v.rehydrated >= max_logs {
                    insert(st, table, batch).await?;
                    return Ok(());
                }
            }
            if now_ms() - last_save > 2_000 {
                last_save = now_ms();
                st.store
                    .pg
                    .put_doc(VIEW_KIND, &v.id, &serde_json::to_value(&*v)?)
                    .await?;
            }
        }
        hour += HOUR;
    }
    insert(st, table, batch).await
}

async fn insert(st: &AppState, table: &str, rows: Vec<LogRow>) -> anyhow::Result<()> {
    use crate::store::writer::Insertable;
    for chunk in rows.chunks(2_000) {
        let mut sql = format!("INSERT INTO {table} {} VALUES ", LogRow::COLUMNS);
        for (i, r) in chunk.iter().enumerate() {
            if i > 0 {
                sql.push(',');
            }
            r.values(&mut sql);
        }
        st.store.gt.exec(&sql).await?;
    }
    Ok(())
}

/// `DELETE /api/ui/logs/rehydrations/{id}`: the view and its logs.
pub async fn delete_view(State(st): State<AppState>, Path(id): Path<String>) -> ApiResult<Value> {
    let v = views(&st)
        .await?
        .into_iter()
        .find(|v| v.id == id)
        .ok_or(ApiError::NotFound)?;
    if v.status == "running" {
        return Err(ApiError::BadQuery("the view is still rehydrating".into()));
    }
    st.store
        .gt
        .exec(&format!("DROP TABLE IF EXISTS {}", v.table()))
        .await?;
    st.store.pg.delete_doc(VIEW_KIND, &id).await?;
    VIEWS
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&v.name);
    Ok(Json(json!({})))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_round_trip() {
        let l = LogRow {
            ts: 1_795_000_000_123,
            status: "error".into(),
            service: Some("api".into()),
            host: Some("h1".into()),
            source: Some("python".into()),
            message: "boom".into(),
            tags: vec!["env:prod".into(), "index:main".into()],
            attributes: json!({"http": {"status_code": 500}})
                .as_object()
                .cloned()
                .unwrap(),
            trace_id: Some("123".into()),
            span_id: None,
        };
        let line = to_line(&l, true);
        let v: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(v["date"], "2026-11-18T11:06:40.123Z");
        assert_eq!(v["tags"], json!(["env:prod"]));
        let back = from_line(&line).unwrap();
        assert_eq!(back.ts, l.ts);
        assert_eq!(back.attributes, l.attributes);
        assert_eq!(back.trace_id.as_deref(), Some("123"));
        assert_eq!(hour_dir(l.ts), "dt=20261118/hour=11");
        assert!(to_line(&l, false).find("\"tags\"").is_none());
    }

    #[tokio::test]
    async fn writes_and_reads_local_archives() {
        let dir = std::env::temp_dir().join(format!("rd-archive-{}", new_id()));
        let d = Destination {
            kind: "local".into(),
            bucket: None,
            path: dir.to_string_lossy().into(),
            region: None,
            endpoint: None,
            integration: None,
        };
        let a = Archives::default();
        a.load(&[(
            "a1".into(),
            ArchiveDef {
                name: "all".into(),
                query: "service:api".into(),
                destination: d.clone(),
                include_tags: true,
                rehydration_tags: vec![],
                rehydration_max_scan_size_in_gb: None,
            },
        )]);
        let mut l =
            from_line(r#"{"date":"2026-11-18T07:06:40.123Z","service":"api","message":"one"}"#)
                .unwrap();
        a.add(&l);
        l.service = Some("web".into());
        a.add(&l);
        a.flush().await;
        let s = a.status("a1");
        assert_eq!((s.state.as_str(), s.archived, s.files), ("WORKING", 1, 1));
        let files: Vec<_> = walk(&dir);
        assert_eq!(files.len(), 1);
        assert!(
            files[0].contains("dt=20261118/hour=07/archive_"),
            "{files:?}"
        );
        assert!(files[0].ends_with(".json.gz"));
        std::fs::remove_dir_all(&dir).ok();
        assert!(
            open(&Destination {
                kind: "gcs".into(),
                ..d
            })
            .is_err()
        );
    }

    fn walk(d: &std::path::Path) -> Vec<String> {
        let mut out = Vec::new();
        for e in std::fs::read_dir(d).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                out.extend(walk(&p));
            } else {
                out.push(p.to_string_lossy().into());
            }
        }
        out
    }
}
