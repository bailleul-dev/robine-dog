//! Cloud bills for Cloud Cost: the billing exports a cloud writes to a
//! bucket, read like any file of `cost::parse` (FOCUS, or the AWS CUR):
//! - S3 or S3-compatible (AWS Data Exports and CUR; GCS with HMAC keys;
//!   OVH, MinIO…);
//! - an Azure Storage container, by its SAS URL (Cost Management exports);
//! - one HTTP URL.
//!
//! Every object is a source of its own, re-imported when its ETag changes and
//! forgotten when it disappears. In a billing period's folder
//! (`20261101-20261201`), only the files of the latest run count: the CUR
//! and Azure write a new folder per run.

use crate::AppState;
use crate::api::{ApiError, ApiResult};
use crate::cost;
use crate::store::pg::new_id;
use axum::Json;
use axum::extract::{Path, State};
use futures::TryStreamExt;
use object_store::ObjectStore;
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap};
use std::sync::LazyLock;

pub const KIND: &str = "cost_account";
const SECRETS: [&str; 2] = ["secret_access_key", "header_value"];

/// A file of an account: key, ETag (or a stand-in), last modified (ms).
#[derive(Debug, Clone, PartialEq)]
pub struct Object {
    pub key: String,
    pub etag: String,
    pub modified: i64,
}

static PERIOD: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"(^|/)\d{8}-\d{8}/").unwrap());

/// Bills only (CSV, JSON, gzip), and in a period's folder the latest run.
pub fn current(objects: Vec<Object>) -> Vec<Object> {
    let bills: Vec<Object> = objects
        .into_iter()
        .filter(|o| {
            let k = o.key.to_ascii_lowercase();
            !k.contains("manifest")
                && [".csv", ".csv.gz", ".json", ".json.gz", ".gz"]
                    .iter()
                    .any(|e| k.ends_with(e))
        })
        .collect();
    // period folder → (run, newest file in it)
    let mut runs: HashMap<String, HashMap<String, i64>> = HashMap::new();
    let split = |k: &str| -> Option<(String, String)> {
        let m = PERIOD.find(k)?;
        let period = k[..m.end()].to_string();
        let rest = &k[m.end()..];
        let run = rest
            .rsplit_once('/')
            .map(|(r, _)| r.to_string())
            .unwrap_or_default();
        Some((period, run))
    };
    for o in &bills {
        if let Some((p, r)) = split(&o.key) {
            let e = runs.entry(p).or_default().entry(r).or_insert(0);
            *e = (*e).max(o.modified);
        }
    }
    let latest: HashMap<String, String> = runs
        .into_iter()
        .filter_map(|(p, r)| r.into_iter().max_by_key(|x| x.1).map(|(run, _)| (p, run)))
        .collect();
    bills
        .into_iter()
        .filter(|o| match split(&o.key) {
            Some((p, r)) => latest.get(&p) == Some(&r),
            None => true,
        })
        .collect()
}

fn s(b: &Value, k: &str) -> String {
    b[k].as_str().unwrap_or("").trim().to_string()
}

fn s3(b: &Value) -> Result<std::sync::Arc<dyn ObjectStore>, String> {
    let mut x = object_store::aws::AmazonS3Builder::new()
        .with_bucket_name(s(b, "bucket"))
        .with_access_key_id(s(b, "access_key_id"))
        .with_secret_access_key(s(b, "secret_access_key"))
        .with_region(if s(b, "region").is_empty() {
            "us-east-1".into()
        } else {
            s(b, "region")
        })
        .with_retry(object_store::RetryConfig {
            max_retries: 2,
            retry_timeout: std::time::Duration::from_secs(30),
            ..Default::default()
        });
    let e = s(b, "endpoint");
    if !e.is_empty() {
        x = x
            .with_endpoint(&e)
            .with_allow_http(e.starts_with("http://"))
            .with_virtual_hosted_style_request(false);
    }
    Ok(std::sync::Arc::new(x.build().map_err(|e| e.to_string())?))
}

/// An Azure container's blobs, from its `List Blobs` XML pages.
pub fn azure_page(xml: &str) -> (Vec<Object>, Option<String>) {
    let Ok(doc) = roxmltree::Document::parse(xml) else {
        return (Vec::new(), None);
    };
    let text = |n: roxmltree::Node, tag: &str| {
        n.descendants()
            .find(|c| c.has_tag_name(tag))
            .and_then(|c| c.text())
            .unwrap_or("")
            .to_string()
    };
    let blobs = doc
        .descendants()
        .filter(|n| n.has_tag_name("Blob"))
        .map(|b| Object {
            key: text(b, "Name"),
            etag: text(b, "Etag"),
            modified: chrono::DateTime::parse_from_rfc2822(&text(b, "Last-Modified"))
                .map(|d| d.timestamp_millis())
                .unwrap_or(0),
        })
        .filter(|o| !o.key.is_empty())
        .collect();
    let next = doc
        .descendants()
        .find(|n| n.has_tag_name("NextMarker"))
        .and_then(|n| n.text())
        .map(String::from)
        .filter(|m| !m.is_empty());
    (blobs, next)
}

/// The container URL and its SAS query, from a SAS URL.
fn sas_parts(sas: &str) -> Result<(String, String), String> {
    let (base, q) = sas
        .split_once('?')
        .ok_or("the SAS URL has no query (?sv=…&sig=…)")?;
    Ok((base.trim_end_matches('/').to_string(), q.to_string()))
}

fn http() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .build()
        .unwrap_or_default()
}

async fn list(b: &Value) -> Result<Vec<Object>, String> {
    let prefix = s(b, "prefix").trim_matches('/').to_string();
    match b["type"].as_str().unwrap_or("s3") {
        "s3" => {
            let store = s3(b)?;
            let p = (!prefix.is_empty()).then(|| object_store::path::Path::from(prefix.as_str()));
            let metas: Vec<object_store::ObjectMeta> = store
                .list(p.as_ref())
                .try_collect()
                .await
                .map_err(|e| e.to_string())?;
            Ok(metas
                .into_iter()
                .map(|m| Object {
                    key: m.location.to_string(),
                    etag: m
                        .e_tag
                        .unwrap_or_else(|| format!("{}:{}", m.size, m.last_modified.timestamp())),
                    modified: m.last_modified.timestamp_millis(),
                })
                .collect())
        }
        "azure" => {
            let (base, q) = sas_parts(&s(b, "sas_url"))?;
            let mut out = Vec::new();
            let mut marker: Option<String> = None;
            loop {
                let mut url = format!("{base}?restype=container&comp=list&{q}");
                if !prefix.is_empty() {
                    url.push_str(&format!(
                        "&prefix={}",
                        percent_encoding::utf8_percent_encode(
                            &prefix,
                            percent_encoding::NON_ALPHANUMERIC
                        )
                    ));
                }
                if let Some(m) = &marker {
                    url.push_str(&format!(
                        "&marker={}",
                        percent_encoding::utf8_percent_encode(
                            m,
                            percent_encoding::NON_ALPHANUMERIC
                        )
                    ));
                }
                let r = http()
                    .get(&url)
                    .header("x-ms-version", "2021-08-06")
                    .send()
                    .await
                    .map_err(|e| e.to_string())?;
                if !r.status().is_success() {
                    return Err(format!("Azure list: HTTP {}", r.status()));
                }
                let (blobs, next) = azure_page(&r.text().await.map_err(|e| e.to_string())?);
                out.extend(blobs);
                match next {
                    Some(m) => marker = Some(m),
                    None => break,
                }
            }
            Ok(out)
        }
        _ => {
            let mut req = http().head(s(b, "url"));
            if !s(b, "header_name").is_empty() {
                req = req.header(s(b, "header_name"), s(b, "header_value"));
            }
            // Servers without HEAD (or ETag): re-read once a day.
            let etag = match req.send().await {
                Ok(r) if r.status().is_success() => r
                    .headers()
                    .get("etag")
                    .or(r.headers().get("last-modified"))
                    .and_then(|v| v.to_str().ok())
                    .map(String::from),
                _ => None,
            }
            .unwrap_or_else(|| (crate::intake::logs::now_ms() / 86_400_000).to_string());
            Ok(vec![Object {
                key: s(b, "url"),
                etag,
                modified: 0,
            }])
        }
    }
}

async fn fetch(b: &Value, key: &str) -> Result<Vec<u8>, String> {
    match b["type"].as_str().unwrap_or("s3") {
        "s3" => {
            let r = s3(b)?
                .get(&object_store::path::Path::from(key))
                .await
                .map_err(|e| e.to_string())?;
            Ok(r.bytes().await.map_err(|e| e.to_string())?.to_vec())
        }
        typ => {
            let url = if typ == "azure" {
                let (base, q) = sas_parts(&s(b, "sas_url"))?;
                let path: Vec<String> = key
                    .split('/')
                    .map(|p| {
                        percent_encoding::utf8_percent_encode(p, percent_encoding::NON_ALPHANUMERIC)
                            .to_string()
                    })
                    .collect();
                format!("{base}/{}?{q}", path.join("/"))
            } else {
                key.to_string()
            };
            let mut req = http().get(&url);
            if typ != "azure" && !s(b, "header_name").is_empty() {
                req = req.header(s(b, "header_name"), s(b, "header_value"));
            }
            let r = req.send().await.map_err(|e| e.to_string())?;
            if !r.status().is_success() {
                return Err(format!("{key}: HTTP {}", r.status()));
            }
            Ok(r.bytes().await.map_err(|e| e.to_string())?.to_vec())
        }
    }
}

fn default_provider(b: &Value) -> String {
    match b["provider"].as_str().unwrap_or("") {
        "" => "Custom".into(),
        p => p.into(),
    }
}

/// Imports what changed; returns (files imported, cost rows).
pub async fn import(st: &AppState, id: &str) -> anyhow::Result<(usize, usize)> {
    let d = st
        .store
        .pg
        .doc(KIND, id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("no such account"))?;
    let mut b = d.body;
    let res: Result<(usize, usize), String> = async {
        let objects = current(list(&b).await?);
        let mut seen: BTreeMap<String, String> =
            serde_json::from_value(b["imported"].clone()).unwrap_or_default();
        let (mut files, mut rows) = (0, 0);
        for o in &objects {
            if seen.get(&o.key) == Some(&o.etag) {
                continue;
            }
            let data = fetch(&b, &o.key).await?;
            let items =
                cost::parse(&data, &default_provider(&b)).map_err(|e| format!("{}: {e}", o.key))?;
            cost::replace(st, &format!("acct:{id}:{}", o.key), &items)
                .await
                .map_err(|e| e.to_string())?;
            seen.insert(o.key.clone(), o.etag.clone());
            files += 1;
            rows += items.len();
        }
        let gone: Vec<String> = seen
            .keys()
            .filter(|k| !objects.iter().any(|o| &o.key == *k))
            .cloned()
            .collect();
        for k in gone {
            cost::forget(st, &format!("acct:{id}:{k}"))
                .await
                .map_err(|e| e.to_string())?;
            seen.remove(&k);
        }
        b["imported"] = json!(seen);
        Ok((files, rows))
    }
    .await;
    b["last_import"] = json!(crate::intake::logs::now_ms());
    b["last_error"] = match &res {
        Ok(_) => Value::Null,
        Err(e) => json!(e),
    };
    st.store.pg.put_doc(KIND, id, &b).await?;
    res.map_err(|e| anyhow::anyhow!(e))
}

/// Every 6 hours, every account.
pub fn spawn(st: AppState) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(300)).await;
            if let Ok(docs) = st.store.pg.docs(KIND).await {
                for d in docs {
                    if let Err(e) = import(&st, &d.id).await {
                        tracing::warn!(account = %d.id, error = %format!("{e:#}"), "cloud bills");
                    }
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(6 * 3600 - 300)).await;
        }
    });
}

fn view(id: &str, b: &Value) -> Value {
    let mut v = b.clone();
    for k in SECRETS {
        if !s(b, k).is_empty() {
            v[k] = json!("********");
        }
    }
    if let Ok((base, _)) = sas_parts(&s(b, "sas_url")) {
        v["sas_url"] = json!(format!("{base}?********"));
    }
    v["files"] = json!(b["imported"].as_object().map(|m| m.len()).unwrap_or(0));
    v.as_object_mut().map(|m| m.remove("imported"));
    v["id"] = json!(id);
    v
}

pub async fn accounts(State(st): State<AppState>) -> ApiResult<Value> {
    let docs = st.store.pg.docs(KIND).await?;
    Ok(Json(
        json!({"data": docs.iter().map(|d| view(&d.id, &d.body)).collect::<Vec<_>>()}),
    ))
}

async fn save(st: &AppState, id: String, v: Value) -> ApiResult<Value> {
    let old = st
        .store
        .pg
        .doc(KIND, &id)
        .await?
        .map(|d| d.body)
        .unwrap_or(Value::Null);
    let typ = v["type"].as_str().unwrap_or("s3");
    if !["s3", "azure", "http"].contains(&typ) {
        return Err(ApiError::BadQuery("type: s3, azure or http".into()));
    }
    let mut b = json!({"name": s(&v, "name"), "type": typ, "provider": s(&v, "provider"), "endpoint": s(&v, "endpoint"),
        "region": s(&v, "region"), "bucket": s(&v, "bucket"), "prefix": s(&v, "prefix"), "access_key_id": s(&v, "access_key_id"),
        "secret_access_key": s(&v, "secret_access_key"), "sas_url": s(&v, "sas_url"), "url": s(&v, "url"),
        "header_name": s(&v, "header_name"), "header_value": s(&v, "header_value"), "imported": old["imported"]});
    // Masked or left empty: the stored secret stays.
    for k in SECRETS {
        let n = s(&b, k);
        if n.is_empty() || n.chars().all(|c| c == '*') {
            b[k] = old[k].clone();
        }
    }
    if s(&b, "sas_url").ends_with("?********") || s(&b, "sas_url").is_empty() {
        b["sas_url"] = old["sas_url"].clone();
    }
    let missing = match typ {
        "s3" => ["bucket", "access_key_id", "secret_access_key"]
            .iter()
            .find(|k| s(&b, k).is_empty()),
        "azure" => ["sas_url"].iter().find(|k| s(&b, k).is_empty()),
        _ => ["url"].iter().find(|k| s(&b, k).is_empty()),
    };
    if s(&b, "name").is_empty() {
        return Err(ApiError::BadQuery("name is required".into()));
    }
    if let Some(k) = missing {
        return Err(ApiError::BadQuery(format!("{k} is required")));
    }
    if typ == "azure" {
        sas_parts(&s(&b, "sas_url")).map_err(ApiError::BadQuery)?;
    }
    st.store.pg.put_doc(KIND, &id, &b).await?;
    let (st2, id2) = (st.clone(), id.clone());
    tokio::spawn(async move {
        if let Err(e) = import(&st2, &id2).await {
            tracing::warn!(error = %format!("{e:#}"), "cloud bills");
        }
    });
    Ok(Json(view(&id, &b)))
}

pub async fn create(State(st): State<AppState>, Json(v): Json<Value>) -> ApiResult<Value> {
    save(&st, new_id(), v).await
}
pub async fn update(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Json(v): Json<Value>,
) -> ApiResult<Value> {
    save(&st, id, v).await
}

pub async fn delete(State(st): State<AppState>, Path(id): Path<String>) -> ApiResult<Value> {
    if let Some(d) = st.store.pg.doc(KIND, &id).await? {
        for k in d.body["imported"]
            .as_object()
            .map(|m| m.keys().cloned().collect::<Vec<_>>())
            .unwrap_or_default()
        {
            cost::forget(&st, &format!("acct:{id}:{k}")).await?;
        }
    }
    st.store.pg.delete_doc(KIND, &id).await?;
    Ok(Json(json!({})))
}

/// `POST /api/ui/cost/accounts/{id}/import`.
pub async fn import_now(State(st): State<AppState>, Path(id): Path<String>) -> ApiResult<Value> {
    match import(&st, &id).await {
        Ok((files, rows)) => Ok(Json(json!({"files": files, "rows": rows}))),
        Err(e) => Err(ApiError::BadQuery(format!("{e:#}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn o(key: &str, modified: i64) -> Object {
        Object {
            key: key.into(),
            etag: "e".into(),
            modified,
        }
    }

    #[test]
    fn only_the_latest_run_of_a_period_counts() {
        let keep = current(vec![
            o("cur/report/20261101-20261201/run-a/report-1.csv.gz", 1),
            o("cur/report/20261101-20261201/run-b/report-1.csv.gz", 5),
            o("cur/report/20261101-20261201/run-b/report-2.csv.gz", 4),
            o("cur/report/20261101-20261201/report-Manifest.json", 9),
            o("cur/report/20261001-20261101/run-z/report-1.csv.gz", 2),
            o("focus/data/BILLING_PERIOD=2026-11/part-0.csv.gz", 3),
            o("focus/data/BILLING_PERIOD=2026-11/part-0.parquet", 3),
        ]);
        let keys: Vec<&str> = keep.iter().map(|o| o.key.as_str()).collect();
        assert_eq!(
            keys,
            vec![
                "cur/report/20261101-20261201/run-b/report-1.csv.gz",
                "cur/report/20261101-20261201/run-b/report-2.csv.gz",
                "cur/report/20261001-20261101/run-z/report-1.csv.gz",
                "focus/data/BILLING_PERIOD=2026-11/part-0.csv.gz",
            ]
        );
    }

    #[test]
    fn azure_blob_listing() {
        let xml = r#"<?xml version="1.0" encoding="utf-8"?><EnumerationResults ContainerName="exports"><Blobs>
            <Blob><Name>focus/20261101-20261130/r1/part_0_0001.csv.gz</Name><Properties><Last-Modified>Sun, 22 Nov 2026 10:00:00 GMT</Last-Modified><Etag>0x8DC</Etag></Properties></Blob>
            </Blobs><NextMarker>abc</NextMarker></EnumerationResults>"#;
        let (blobs, next) = azure_page(xml);
        assert_eq!(blobs.len(), 1);
        assert_eq!(blobs[0].etag, "0x8DC");
        assert!(blobs[0].modified > 0);
        assert_eq!(next.as_deref(), Some("abc"));
        assert_eq!(
            sas_parts("https://a.blob.core.windows.net/c?sv=1&sig=x")
                .unwrap()
                .0,
            "https://a.blob.core.windows.net/c"
        );
    }
}
