//! Session Replay: the segments the browser SDK records
//! (`sessionReplaySampleRate`), posted to `/api/v2/replay` (or through the
//! `proxy`) as `multipart/form-data`: a `segment` file, the zlib-compressed
//! JSON of rrweb-like records, and an `event` with its metadata (older SDKs
//! send the metadata as form fields). Segments are kept in PostgreSQL as
//! sent, and read back per session as one list of records for the player.

use crate::AppState;
use crate::api::{ApiError, ApiResult};
use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, header};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use std::io::Read;

/// The most a session's records may weigh, decompressed, when read back.
const MAX_SESSION_BYTES: usize = 64 << 20;
/// The most a segment may weigh, decompressed.
const MAX_SEGMENT_BYTES: u64 = 32 << 20;

/// One segment's metadata.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Meta {
    pub application_id: Option<String>,
    pub session_id: String,
    pub view_id: String,
    pub start: i64,
    pub end: i64,
    pub records_count: i64,
    pub has_full_snapshot: bool,
    pub index_in_view: Option<i64>,
    pub creation_reason: Option<String>,
    pub source: Option<String>,
}

fn text(v: Option<&Value>) -> Option<String> {
    match v? {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn number(v: Option<&Value>) -> Option<i64> {
    match v? {
        Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

impl Meta {
    /// From the `event` JSON, the segment itself, or flat form fields
    /// (`session.id`…): all have the same names.
    pub fn read(v: &Value) -> Option<Meta> {
        let at = |nested: &str, flat: &str| v.pointer(nested).or_else(|| v.get(flat));
        Some(Meta {
            application_id: text(at("/application/id", "application.id")),
            session_id: text(at("/session/id", "session.id"))?,
            view_id: text(at("/view/id", "view.id"))?,
            start: number(v.get("start"))?,
            end: number(v.get("end"))?,
            records_count: number(v.get("records_count")).unwrap_or(0),
            has_full_snapshot: match v.get("has_full_snapshot") {
                Some(Value::Bool(b)) => *b,
                Some(Value::String(s)) => s == "true",
                _ => false,
            },
            index_in_view: number(v.get("index_in_view")),
            creation_reason: text(v.get("creation_reason")),
            source: text(v.get("source")),
        })
    }
}

/// zlib (what the SDK sends), raw deflate, or plain JSON.
pub fn inflate(data: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    if flate2::read::ZlibDecoder::new(data)
        .take(MAX_SEGMENT_BYTES)
        .read_to_end(&mut out)
        .is_ok()
        && !out.is_empty()
    {
        return Ok(out);
    }
    out.clear();
    if flate2::read::DeflateDecoder::new(data)
        .take(MAX_SEGMENT_BYTES)
        .read_to_end(&mut out)
        .is_ok()
        && !out.is_empty()
    {
        return Ok(out);
    }
    if data.first() == Some(&b'{') {
        return Ok(data.to_vec());
    }
    Err("the segment is neither zlib, deflate nor JSON".into())
}

fn deflate(data: &[u8]) -> Vec<u8> {
    use std::io::Write;
    let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
    let _ = e.write_all(data);
    e.finish().unwrap_or_default()
}

/// The parts of the form: the segment's bytes and its metadata.
pub async fn read_form(headers: &HeaderMap, body: Bytes) -> Result<(Bytes, Meta), String> {
    let ct = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .ok_or("no content type")?;
    let boundary = multer::parse_boundary(ct).map_err(|e| format!("content type: {e}"))?;
    let stream = futures::stream::once(async move { Ok::<Bytes, std::io::Error>(body) });
    let mut form = multer::Multipart::new(stream, boundary);
    let (mut segment, mut event, mut fields) = (None, None, Map::new());
    while let Some(field) = form.next_field().await.map_err(|e| e.to_string())? {
        let name = field.name().unwrap_or_default().to_string();
        let data = field.bytes().await.map_err(|e| e.to_string())?;
        match name.as_str() {
            "segment" => segment = Some(data),
            "event" => event = serde_json::from_slice::<Value>(&data).ok(),
            other => {
                fields.insert(
                    other.to_string(),
                    json!(String::from_utf8_lossy(&data).into_owned()),
                );
            }
        }
    }
    let segment = segment.ok_or("no segment")?;
    let meta = event
        .as_ref()
        .and_then(Meta::read)
        .or_else(|| Meta::read(&Value::Object(fields)));
    let meta = match meta {
        Some(m) => m,
        // The segment carries its metadata too.
        None => {
            let plain = inflate(&segment)?;
            let v: Value = serde_json::from_slice(&plain).map_err(|e| e.to_string())?;
            Meta::read(&v).ok_or("the segment has no session, view or times")?
        }
    };
    Ok((segment, meta))
}

/// Stores a posted segment; why it was refused otherwise.
pub async fn ingest(st: &AppState, headers: &HeaderMap, body: Bytes) -> Result<(), String> {
    let (segment, meta) = read_form(headers, body).await?;
    // Checked, and kept compressed with zlib whatever it came as.
    let plain = inflate(&segment)?;
    if serde_json::from_slice::<serde::de::IgnoredAny>(&plain).is_err() {
        return Err("the segment is not JSON".into());
    }
    let stored = if flate2::read::ZlibDecoder::new(&segment[..])
        .read_to_end(&mut Vec::new())
        .is_ok()
    {
        segment.to_vec()
    } else {
        deflate(&plain)
    };
    st.store
        .pg
        .put_replay_segment(&meta, &stored)
        .await
        .map_err(|e| e.to_string())?;
    tracing::debug!(session = %meta.session_id, records = meta.records_count, "replay segment");
    Ok(())
}

#[derive(Deserialize)]
pub struct ReplayQuery {
    #[serde(default)]
    view_id: Option<String>,
}

/// The records of segments, in time order: those of a type the player
/// knows, with Datadog's media interactions turned into rrweb's.
pub fn merge(segments: &[Vec<u8>]) -> Result<Vec<Value>, String> {
    let mut records = Vec::new();
    let mut size = 0;
    for data in segments {
        let plain = inflate(data)?;
        size += plain.len();
        if size > MAX_SESSION_BYTES {
            return Err("the replay is too large to show".into());
        }
        let v: Value = serde_json::from_slice(&plain).map_err(|e| e.to_string())?;
        if let Some(rs) = v.get("records").and_then(Value::as_array) {
            records.extend(rs.iter().cloned());
        }
    }
    records.sort_by_key(|r| r["timestamp"].as_i64().unwrap_or(0));
    for r in &mut records {
        // `play` / `pause` → rrweb's MediaInteractions 0 / 1.
        if r["type"] == 3
            && r["data"]["source"] == 7
            && let Some(t) = r["data"]["type"].as_str()
        {
            r["data"]["type"] = json!(if t == "play" { 0 } else { 1 });
        }
    }
    Ok(records)
}

/// `GET /api/ui/rum/replay/{session_id}`: the session's views with a replay,
/// and the records (of one view with `?view_id=`).
pub async fn get(
    State(st): State<AppState>,
    Path(session_id): Path<String>,
    Query(q): Query<ReplayQuery>,
) -> ApiResult<Value> {
    let segments = st.store.pg.replay_segments(&session_id).await?;
    if segments.is_empty() {
        return Err(ApiError::NotFound);
    }
    let mut views: Vec<Value> = Vec::new();
    for s in &segments {
        match views.iter_mut().find(|v| v["view_id"] == s.meta.view_id) {
            Some(v) => {
                v["start"] = json!(v["start"].as_i64().unwrap_or(0).min(s.meta.start));
                v["end"] = json!(v["end"].as_i64().unwrap_or(0).max(s.meta.end));
                v["records"] = json!(v["records"].as_i64().unwrap_or(0) + s.meta.records_count);
                v["segments"] = json!(v["segments"].as_i64().unwrap_or(0) + 1);
            }
            None => views.push(
                json!({"view_id": s.meta.view_id, "start": s.meta.start, "end": s.meta.end,
                "records": s.meta.records_count, "segments": 1}),
            ),
        }
    }
    views.sort_by_key(|v| v["start"].as_i64().unwrap_or(0));
    let wanted: Vec<Vec<u8>> = segments
        .into_iter()
        .filter(|s| q.view_id.as_ref().is_none_or(|v| *v == s.meta.view_id))
        .map(|s| s.data)
        .collect();
    let records = merge(&wanted).map_err(ApiError::BadQuery)?;
    Ok(Json(
        json!({"session_id": session_id, "views": views, "records": records}),
    ))
}

/// Drops the segments past the retention, every hour.
pub fn spawn_purge(st: AppState) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(3600));
        loop {
            tick.tick().await;
            let before = crate::intake::logs::now_ms() - st.cfg.retention_days * 86_400_000;
            match st.store.pg.purge_replay_segments(before).await {
                Ok(n) if n > 0 => tracing::info!(n, "purged replay segments"),
                Ok(_) => {}
                Err(e) => tracing::warn!(error = %e, "purging replay segments"),
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zlib(s: &str) -> Vec<u8> {
        deflate(s.as_bytes())
    }

    #[test]
    fn metadata_and_records() {
        let event = json!({"application": {"id": "shop"}, "session": {"id": "s1"}, "view": {"id": "v1"},
            "start": 1000, "end": 2000, "records_count": 3, "has_full_snapshot": true, "index_in_view": 0,
            "creation_reason": "init", "source": "browser"});
        let m = Meta::read(&event).unwrap();
        assert_eq!(
            (m.session_id.as_str(), m.view_id.as_str(), m.start, m.end),
            ("s1", "v1", 1000, 2000)
        );
        assert!(m.has_full_snapshot);
        // The older SDKs' form fields.
        let flat = json!({"session.id": "s2", "view.id": "v2", "start": "5", "end": "9", "has_full_snapshot": "false"});
        let m = Meta::read(&flat).unwrap();
        assert_eq!(
            (m.session_id.as_str(), m.start, m.has_full_snapshot),
            ("s2", 5, false)
        );
        assert!(Meta::read(&json!({"start": 1})).is_none());

        let a = zlib(
            r#"{"records": [{"type": 4, "timestamp": 10, "data": {"href": "https://x"}},
            {"type": 3, "timestamp": 30, "data": {"source": 7, "type": "pause", "id": 4}}]}"#,
        );
        let b = zlib(r#"{"records": [{"type": 2, "timestamp": 20, "data": {"node": {}}}]}"#);
        let r = merge(&[a, b]).unwrap();
        assert_eq!(
            r.iter()
                .map(|x| x["timestamp"].as_i64().unwrap())
                .collect::<Vec<_>>(),
            vec![10, 20, 30]
        );
        assert_eq!(r[2]["data"]["type"], 1);
        assert_eq!(inflate(b"{\"records\": []}").unwrap(), b"{\"records\": []}");
        assert!(inflate(b"\x00garbage").is_err());
    }

    #[tokio::test]
    async fn reads_the_form() {
        let seg = zlib(
            r#"{"records": [], "session": {"id": "s9"}, "view": {"id": "v9"}, "start": 1, "end": 2}"#,
        );
        let mut body = Vec::new();
        body.extend_from_slice(b"--XYZ\r\nContent-Disposition: form-data; name=\"segment\"; filename=\"s9-1\"\r\nContent-Type: application/octet-stream\r\n\r\n");
        body.extend_from_slice(&seg);
        body.extend_from_slice(b"\r\n--XYZ\r\nContent-Disposition: form-data; name=\"event\"; filename=\"blob\"\r\nContent-Type: application/json\r\n\r\n");
        body.extend_from_slice(br#"{"session": {"id": "s9"}, "view": {"id": "v9"}, "start": 1, "end": 2, "records_count": 0}"#);
        body.extend_from_slice(b"\r\n--XYZ--\r\n");
        let mut h = HeaderMap::new();
        h.insert(
            header::CONTENT_TYPE,
            "multipart/form-data; boundary=XYZ".parse().unwrap(),
        );
        let (s, m) = read_form(&h, Bytes::from(body.clone())).await.unwrap();
        assert_eq!(s.to_vec(), seg);
        assert_eq!(m.session_id, "s9");
        // Without the event: from the segment.
        let cut = body.windows(7).position(|w| w == b"\r\n--XYZ").unwrap();
        let mut only = body[..cut].to_vec();
        only.extend_from_slice(b"\r\n--XYZ--\r\n");
        let (_, m) = read_form(&h, Bytes::from(only)).await.unwrap();
        assert_eq!(m.view_id, "v9");
    }
}
