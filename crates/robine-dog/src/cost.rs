//! Cloud Cost Management: daily costs from FOCUS files (Custom Costs, cloud
//! billing exports, the AWS CUR's columns too) and from a Kubernetes
//! cluster's price split across its pods by usage, as OpenCost does. Rows
//! live in `cost_items`; each is also written as the `all.cost` and
//! `<provider>.cost.amortized` metrics, so dashboards and monitors use them.

use crate::AppState;
use crate::api::{ApiError, ApiResult};
use crate::metrics::Point;
use crate::store::pg::new_id;
use crate::store::writer::Insertable;
use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap};
use std::io::Read;

const DAY: i64 = 86_400_000;
pub const CUSTOM: &str = "cost_custom_file";
pub const CLUSTER: &str = "cost_cluster";

#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    /// Midnight UTC, epoch ms.
    pub day: i64,
    pub provider: String,
    pub service: String,
    pub account: String,
    pub resource: String,
    pub tags: BTreeMap<String, String>,
    pub cost: f64,
    pub currency: String,
    /// Usage figures (Kubernetes), shown next to the cost.
    pub extra: Value,
}

// ---------- Parsing ----------

/// RFC 4180: quoted fields, doubled quotes, CRLF.
fn csv(text: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let (mut row, mut field, mut quoted) = (Vec::new(), String::new(), false);
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match (quoted, c) {
            (true, '"') if chars.peek() == Some(&'"') => {
                field.push('"');
                chars.next();
            }
            (true, '"') => quoted = false,
            (true, c) => field.push(c),
            (false, '"') => quoted = true,
            (false, ',') => row.push(std::mem::take(&mut field)),
            (false, '\r') => {}
            (false, '\n') => {
                row.push(std::mem::take(&mut field));
                rows.push(std::mem::take(&mut row));
            }
            (false, c) => field.push(c),
        }
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    rows.retain(|r| !(r.len() == 1 && r[0].trim().is_empty()));
    rows
}

/// `2026-11-01`, `2026-11-01T00:00:00Z`, `2026-11-01 00:00:00`: that day.
fn day_of(s: &str) -> Option<i64> {
    let d = s.trim().get(..10)?;
    let date = chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").ok()?;
    Some(date.and_hms_opt(0, 0, 0)?.and_utc().timestamp_millis())
}

fn provider_key(p: &str) -> &'static str {
    let p = p.to_ascii_lowercase();
    if p.contains("aws") || p.contains("amazon") {
        "aws"
    } else if p.contains("google") || p == "gcp" {
        "gcp"
    } else if p.contains("azure") || p.contains("microsoft") {
        "azure"
    } else if p == "kubernetes" {
        "kubernetes"
    } else {
        "custom"
    }
}

fn tag_key(k: &str) -> String {
    k.trim()
        .to_ascii_lowercase()
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '_' | '.' | '-' | '/') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// One record (column → value) as an item, FOCUS or AWS CUR names.
fn record(
    get: &dyn Fn(&str) -> Option<String>,
    extra_tags: &[(String, String)],
    default_provider: &str,
) -> Option<Item> {
    let first = |names: &[&str]| {
        names
            .iter()
            .find_map(|n| get(n).filter(|v| !v.trim().is_empty()))
    };
    let day = day_of(&first(&[
        "ChargePeriodStart",
        "BillingPeriodStart",
        "lineItem/UsageStartDate",
        "UsageDate",
        "Date",
    ])?)?;
    let cost: f64 = first(&[
        "EffectiveCost",
        "BilledCost",
        "lineItem/UnblendedCost",
        "Cost",
    ])?
    .trim()
    .parse()
    .ok()?;
    if !cost.is_finite() {
        return None;
    }
    let provider = first(&["ProviderName", "PublisherName"]).unwrap_or_else(|| {
        if get("lineItem/UnblendedCost").is_some() {
            "AWS".into()
        } else {
            default_provider.into()
        }
    });
    let mut tags = BTreeMap::new();
    if let Some(t) = first(&["Tags"])
        && let Ok(Value::Object(m)) = serde_json::from_str::<Value>(&t)
    {
        for (k, v) in m {
            let v = v
                .as_str()
                .map(String::from)
                .unwrap_or_else(|| v.to_string());
            if !v.is_empty() {
                tags.insert(tag_key(&k), v);
            }
        }
    }
    for (k, v) in extra_tags {
        if !v.is_empty() {
            tags.insert(k.clone(), v.clone());
        }
    }
    Some(Item {
        day,
        provider: provider_key(&provider).to_string(),
        service: first(&["ServiceName", "product/ProductName", "ChargeDescription"])
            .unwrap_or_else(|| "unknown".into()),
        account: first(&[
            "SubAccountId",
            "SubAccountName",
            "lineItem/UsageAccountId",
            "BillingAccountId",
        ])
        .unwrap_or_default(),
        resource: first(&["ResourceId", "lineItem/ResourceId", "ResourceName"]).unwrap_or_default(),
        tags,
        cost,
        currency: first(&["BillingCurrency", "lineItem/CurrencyCode", "Currency"])
            .unwrap_or_else(|| "USD".into()),
        extra: Value::Null,
    })
}

/// A FOCUS (or AWS CUR) file, CSV or JSON, gzip or not, summed per day.
pub fn parse(data: &[u8], default_provider: &str) -> Result<Vec<Item>, String> {
    let mut text = String::new();
    if data.starts_with(&[0x1f, 0x8b]) {
        flate2::read::GzDecoder::new(data)
            .read_to_string(&mut text)
            .map_err(|e| format!("gzip: {e}"))?;
    } else {
        text = String::from_utf8_lossy(data).into_owned();
    }
    let text = text.trim_start_matches('\u{feff}');
    let mut items = Vec::new();
    let mut skipped = 0;
    if text.trim_start().starts_with('[') || text.trim_start().starts_with('{') {
        let v: Value = serde_json::from_str(text).map_err(|e| format!("JSON: {e}"))?;
        let list = v
            .as_array()
            .cloned()
            .or_else(|| v["data"].as_array().cloned())
            .unwrap_or_default();
        for r in &list {
            let get = |k: &str| match &r[k] {
                Value::Null => None,
                Value::String(s) => Some(s.clone()),
                x => Some(x.to_string()),
            };
            match record(&get, &[], default_provider) {
                Some(i) => items.push(i),
                None => skipped += 1,
            }
        }
    } else {
        let rows = csv(text);
        let Some((head, body)) = rows.split_first() else {
            return Err("empty file".into());
        };
        let idx: HashMap<&str, usize> = head
            .iter()
            .enumerate()
            .map(|(i, h)| (h.trim(), i))
            .collect();
        let user_tags: Vec<(usize, String)> = head
            .iter()
            .enumerate()
            .filter_map(|(i, h)| {
                h.strip_prefix("resourceTags/user:")
                    .map(|k| (i, tag_key(k)))
            })
            .collect();
        for r in body {
            let get = |k: &str| idx.get(k).and_then(|i| r.get(*i)).cloned();
            let extra: Vec<(String, String)> = user_tags
                .iter()
                .map(|(i, k)| (k.clone(), r.get(*i).cloned().unwrap_or_default()))
                .collect();
            match record(&get, &extra, default_provider) {
                Some(i) => items.push(i),
                None => skipped += 1,
            }
        }
    }
    if items.is_empty() {
        return Err(format!(
            "no cost rows ({skipped} skipped): ChargePeriodStart and BilledCost (FOCUS) or lineItem/UsageStartDate and lineItem/UnblendedCost (AWS CUR) are needed"
        ));
    }
    Ok(aggregate(items))
}

type ItemKey = (
    i64,
    String,
    String,
    String,
    String,
    BTreeMap<String, String>,
    String,
);

fn aggregate(items: Vec<Item>) -> Vec<Item> {
    let mut m: BTreeMap<ItemKey, f64> = BTreeMap::new();
    for i in items {
        *m.entry((
            i.day, i.provider, i.service, i.account, i.resource, i.tags, i.currency,
        ))
        .or_default() += i.cost;
    }
    m.into_iter()
        .map(
            |((day, provider, service, account, resource, tags, currency), cost)| Item {
                day,
                provider,
                service,
                account,
                resource,
                tags,
                cost,
                currency,
                extra: Value::Null,
            },
        )
        .collect()
}

// ---------- Storage ----------

fn metric_tags(i: &Item, source: &str) -> Vec<String> {
    let mut t: Vec<String> = vec![
        format!("providername:{}", i.provider),
        format!("servicename:{}", i.service),
        format!("cost_source:{source}"),
    ];
    if !i.account.is_empty() {
        t.push(format!("account:{}", i.account));
    }
    t.extend(i.tags.iter().map(|(k, v)| format!("{k}:{v}")));
    t.sort();
    t.dedup();
    t
}

async fn forget_metrics(st: &AppState, source: &str) {
    let sql = format!(
        "DELETE FROM cost_points WHERE metric IN ('all.cost','aws.cost.amortized','gcp.cost.amortized','azure.cost.amortized','custom.cost.amortized','kubernetes.cost.amortized') AND tags LIKE '%cost_source:{}%'",
        source.replace('\'', "")
    );
    if let Err(e) = st.store.gt.exec(&sql).await {
        tracing::warn!(error = %e, "cost metrics not deleted");
    }
}

/// Replaces a source's rows (and metrics) with `items`.
pub async fn replace(st: &AppState, source: &str, items: &[Item]) -> anyhow::Result<()> {
    forget(st, source).await?;
    let db = st.store.pg.conn().await?;
    for chunk in items.chunks(2000) {
        let col = |f: &dyn Fn(&Item) -> String| chunk.iter().map(f).collect::<Vec<String>>();
        let days: Vec<i64> = chunk.iter().map(|i| i.day).collect();
        let costs: Vec<f64> = chunk.iter().map(|i| i.cost).collect();
        db.execute(
            "INSERT INTO cost_items (day, provider, service, account, resource, tags, cost, currency, source_id, extra)
             SELECT d, p, s, a, r, t::jsonb, c, cur, $9, x::jsonb
             FROM unnest($1::bigint[], $2::text[], $3::text[], $4::text[], $5::text[], $6::text[], $7::float8[], $8::text[], $10::text[])
               AS u(d, p, s, a, r, t, c, cur, x)",
            &[
                &days,
                &col(&|i| i.provider.clone()),
                &col(&|i| i.service.clone()),
                &col(&|i| i.account.clone()),
                &col(&|i| i.resource.clone()),
                &col(&|i| serde_json::to_string(&i.tags).unwrap_or_default()),
                &costs,
                &col(&|i| i.currency.clone()),
                &source,
                &col(&|i| i.extra.to_string()),
            ],
        )
        .await?;
    }
    let mut rows = Vec::with_capacity(items.len() * 2);
    for i in items {
        let tags = metric_tags(i, source);
        for metric in [
            "all.cost".to_string(),
            format!("{}.cost.amortized", i.provider),
        ] {
            rows.push(Point {
                ts: i.day,
                metric,
                host: None,
                tags: tags.clone(),
                value: i.cost,
                weight: 1.0,
            });
        }
    }
    let kinds: Vec<(String, String)> = ["all", "aws", "gcp", "azure", "custom", "kubernetes"]
        .iter()
        .map(|p| {
            (
                if *p == "all" {
                    "all.cost".into()
                } else {
                    format!("{p}.cost.amortized")
                },
                "gauge".into(),
            )
        })
        .collect();
    st.store.pg.touch_metrics(&kinds).await?;
    for chunk in rows.chunks(1000) {
        let mut sql = String::from(
            r#"INSERT INTO cost_points ("ts", "metric", "host", "tags", "value", "weight") VALUES "#,
        );
        for (n, p) in chunk.iter().enumerate() {
            if n > 0 {
                sql.push(',');
            }
            p.values(&mut sql);
        }
        st.store.gt.exec(&sql).await?;
    }
    Ok(())
}

pub async fn forget(st: &AppState, source: &str) -> anyhow::Result<()> {
    st.store
        .pg
        .conn()
        .await?
        .execute("DELETE FROM cost_items WHERE source_id = $1", &[&source])
        .await?;
    forget_metrics(st, source).await;
    Ok(())
}

// ---------- Custom Costs ----------

fn file_view(id: &str, b: &Value) -> Value {
    json!({"type": "custom_costs_file", "id": id, "attributes": {
        "name": b["name"], "status": b["status"], "error": b["error"], "rows": b["rows"],
        "billed_cost": b["total"], "currency": b["currency"], "providers": b["providers"],
        "uploaded_at": b["uploaded_at"].as_i64().map(crate::logconfig::iso),
        "charge_period": {"start": b["start"].as_i64().map(crate::logconfig::iso), "end": b["end"].as_i64().map(crate::logconfig::iso)}}})
}

/// The file in a request: multipart (`file`), or the body (JSON or CSV).
async fn file_body(headers: &HeaderMap, body: Bytes) -> anyhow::Result<(String, Bytes)> {
    let ct = headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if ct.starts_with("multipart/") {
        let boundary = multer::parse_boundary(ct)?;
        let stream = futures::stream::once(async move { Ok::<Bytes, std::io::Error>(body) });
        let mut form = multer::Multipart::new(stream, boundary);
        let f = form
            .next_field()
            .await?
            .ok_or_else(|| anyhow::anyhow!("no file in the form"))?;
        let name = f.file_name().unwrap_or("upload").to_string();
        return Ok((name, f.bytes().await?));
    }
    let name = headers
        .get("x-file-name")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("upload")
        .to_string();
    Ok((name, body))
}

/// `PUT /api/v2/cost/custom_costs`.
pub async fn upload(
    State(st): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<Value> {
    let (name, data) = file_body(&headers, body)
        .await
        .map_err(|e| ApiError::BadQuery(e.to_string()))?;
    let items = parse(&data, "Custom").map_err(ApiError::BadQuery)?;
    let id = new_id();
    let source = format!("custom:{id}");
    replace(&st, &source, &items).await?;
    let mut providers: Vec<&str> = items.iter().map(|i| i.provider.as_str()).collect();
    providers.sort();
    providers.dedup();
    let b = json!({"name": name, "status": "active", "rows": items.len(),
        "total": items.iter().map(|i| i.cost).sum::<f64>(), "currency": items[0].currency,
        "providers": providers, "uploaded_at": crate::intake::logs::now_ms(),
        "start": items.iter().map(|i| i.day).min(), "end": items.iter().map(|i| i.day + DAY).max()});
    st.store.pg.put_doc(CUSTOM, &id, &b).await?;
    Ok(Json(json!({"data": file_view(&id, &b)})))
}

/// `GET /api/v2/cost/custom_costs`.
pub async fn files(State(st): State<AppState>) -> ApiResult<Value> {
    let mut docs = st.store.pg.docs(CUSTOM).await?;
    docs.sort_by_key(|d| -d.body["uploaded_at"].as_i64().unwrap_or(0));
    Ok(Json(
        json!({"data": docs.iter().map(|d| file_view(&d.id, &d.body)).collect::<Vec<_>>()}),
    ))
}

/// `GET /api/v2/cost/custom_costs/{id}`.
pub async fn file(State(st): State<AppState>, Path(id): Path<String>) -> ApiResult<Value> {
    let d = st
        .store
        .pg
        .doc(CUSTOM, &id)
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(json!({"data": file_view(&id, &d.body)})))
}

/// `DELETE /api/v2/cost/custom_costs/{id}`: the file and its costs.
pub async fn delete_file(State(st): State<AppState>, Path(id): Path<String>) -> ApiResult<Value> {
    forget(&st, &format!("custom:{id}")).await?;
    st.store.pg.delete_doc(CUSTOM, &id).await?;
    Ok(Json(json!({})))
}

// ---------- Kubernetes ----------

const WORKLOADS: [&str; 3] = ["kube_deployment", "kube_stateful_set", "kube_daemon_set"];

/// Per group of `by`, the hourly values over `[from, to)`: (sum, hours seen).
async fn hourly(
    st: &AppState,
    q: &str,
    from: i64,
    to: i64,
) -> anyhow::Result<Vec<(BTreeMap<String, String>, f64, f64)>> {
    let req: crate::api::query::Request =
        serde_json::from_value(json!({"from": from, "to": to, "interval": 3_600_000,
        "queries": [{"data_source": "metrics", "name": "a", "query": q}]}))?;
    let out = crate::api::query::execute(st, &req, false)
        .await
        .map_err(|e| anyhow::anyhow!("{q}: {e:?}"))?;
    Ok(out
        .series
        .into_iter()
        .map(|(_, _, s)| {
            let vals: Vec<f64> = s.values.iter().flatten().copied().collect();
            (s.tags, vals.iter().sum(), vals.len() as f64)
        })
        .collect())
}

#[derive(Debug, Clone, Deserialize)]
pub struct Cluster {
    pub name: String,
    #[serde(default = "star")]
    pub filter: String,
    pub monthly_cost: f64,
    #[serde(default = "eur")]
    pub currency: String,
    /// The part of the price that pays for CPU (the rest, memory).
    #[serde(default = "half")]
    pub cpu_share: f64,
}
fn star() -> String {
    "*".into()
}
fn eur() -> String {
    "EUR".into()
}
fn half() -> f64 {
    0.5
}

/// One pod's usage over the period: average cores and bytes.
#[derive(Debug, Clone, Default)]
pub struct PodUsage {
    pub tags: BTreeMap<String, String>,
    pub cpu: f64,
    pub memory: f64,
}

/// The price of `hours` split across pods by CPU and memory, the unused
/// capacity in an `__idle__` row.
pub fn allocate(
    c: &Cluster,
    day: i64,
    hours: f64,
    pods: &[PodUsage],
    cpu_cap: f64,
    mem_cap: f64,
) -> Vec<Item> {
    let cost = c.monthly_cost * 12.0 / 365.0 * hours / 24.0;
    let cpu_used: f64 = pods.iter().map(|p| p.cpu).sum();
    let mem_used: f64 = pods.iter().map(|p| p.memory).sum();
    // Without the kube-state capacity, the pods share everything.
    let cpu_cap = if cpu_cap > 0.0 {
        cpu_cap.max(cpu_used)
    } else {
        cpu_used
    };
    let mem_cap = if mem_cap > 0.0 {
        mem_cap.max(mem_used)
    } else {
        mem_used
    };
    let share = |used: f64, cap: f64| if cap > 0.0 { used / cap } else { 0.0 };
    let mut out = Vec::new();
    let mut spent = 0.0;
    for p in pods {
        let v = cost
            * (c.cpu_share * share(p.cpu, cpu_cap)
                + (1.0 - c.cpu_share) * share(p.memory, mem_cap));
        if v <= 0.0 {
            continue;
        }
        spent += v;
        let ns = p
            .tags
            .get("kube_namespace")
            .cloned()
            .unwrap_or_else(|| "none".into());
        let mut tags = BTreeMap::from([
            ("kube_namespace".to_string(), ns.clone()),
            ("kube_cluster_name".to_string(), c.name.clone()),
        ]);
        for w in WORKLOADS {
            if let Some(v) = p.tags.get(w) {
                tags.insert(w.to_string(), v.clone());
            }
        }
        out.push(Item {
            day,
            provider: "kubernetes".into(),
            service: ns,
            account: c.name.clone(),
            resource: p.tags.get("pod_name").cloned().unwrap_or_default(),
            tags,
            cost: v,
            currency: c.currency.clone(),
            extra: json!({"cpu_cores": p.cpu, "memory_bytes": p.memory}),
        });
    }
    out.push(Item {
        day,
        provider: "kubernetes".into(),
        service: "__idle__".into(),
        account: c.name.clone(),
        resource: String::new(),
        tags: BTreeMap::from([("kube_cluster_name".to_string(), c.name.clone())]),
        cost: (cost - spent).max(0.0),
        currency: c.currency.clone(),
        extra: json!({"cpu_cores": cpu_cap - cpu_used, "memory_bytes": mem_cap - mem_used,
            "cpu_capacity": cpu_cap, "memory_capacity": mem_cap}),
    });
    out
}

/// A cluster's costs for the day starting at `day`, up to now: the hours
/// with metrics only, so a cluster watched since noon costs half a day.
async fn cluster_day(st: &AppState, id: &str, c: &Cluster, day: i64) -> anyhow::Result<usize> {
    let to = (day + DAY).min(crate::intake::logs::now_ms() / 3_600_000 * 3_600_000);
    let source = format!("k8s:{id}:{day}");
    if to <= day {
        return Ok(0);
    }
    let by = format!("kube_namespace,pod_name,{}", WORKLOADS.join(","));
    let f = if c.filter.trim().is_empty() {
        "*"
    } else {
        c.filter.trim()
    };
    let cpu = hourly(
        st,
        &format!("sum:kubernetes.cpu.usage.total{{{f}}} by {{{by}}}"),
        day,
        to,
    )
    .await?;
    let mem = hourly(
        st,
        &format!("sum:kubernetes.memory.usage{{{f}}} by {{{by}}}"),
        day,
        to,
    )
    .await?;
    let cpu_cap = hourly(
        st,
        &format!("sum:kubernetes_state.node.cpu_allocatable{{{f}}}"),
        day,
        to,
    )
    .await?;
    let mem_cap = hourly(
        st,
        &format!("sum:kubernetes_state.node.memory_allocatable{{{f}}}"),
        day,
        to,
    )
    .await?;
    // Hours watched: the nodes' capacity, else the busiest pod.
    let hours = cpu_cap
        .first()
        .map(|r| r.2)
        .filter(|n| *n > 0.0)
        .unwrap_or_else(|| cpu.iter().chain(&mem).map(|r| r.2).fold(0.0, f64::max));
    if hours < 1.0 {
        forget(st, &source).await?;
        return Ok(0);
    }
    let clean = |t: BTreeMap<String, String>| -> BTreeMap<String, String> {
        t.into_iter()
            .filter(|(_, v)| v != "N/A" && !v.is_empty())
            .collect()
    };
    let mut pods: BTreeMap<Vec<(String, String)>, PodUsage> = BTreeMap::new();
    for (tags, sum, _) in cpu {
        let tags = clean(tags);
        let p = pods.entry(tags.clone().into_iter().collect()).or_default();
        p.tags = tags;
        p.cpu = sum / hours / 1e9;
    }
    for (tags, sum, _) in mem {
        let tags = clean(tags);
        let p = pods.entry(tags.clone().into_iter().collect()).or_default();
        p.tags = tags;
        p.memory = sum / hours;
    }
    let avg = |r: &[(BTreeMap<String, String>, f64, f64)]| {
        r.first()
            .map(|(_, s, n)| if *n > 0.0 { s / n } else { 0.0 })
            .unwrap_or(0.0)
    };
    let pods: Vec<PodUsage> = pods
        .into_values()
        .filter(|p| p.tags.contains_key("pod_name"))
        .collect();
    let items = allocate(c, day, hours, &pods, avg(&cpu_cap), avg(&mem_cap));
    replace(st, &source, &items).await?;
    Ok(items.len())
}

async fn cluster_days(st: &AppState, id: &str, body: &Value, days: i64) -> anyhow::Result<usize> {
    let c: Cluster = serde_json::from_value(body.clone())?;
    let today = crate::intake::logs::now_ms() / DAY * DAY;
    let mut n = 0;
    for k in (0..days).rev() {
        n += cluster_day(st, id, &c, today - k * DAY).await?;
    }
    Ok(n)
}

/// Every hour: yesterday and today of every cluster.
pub fn spawn(st: AppState) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(120)).await;
            if let Ok(docs) = st.store.pg.docs(CLUSTER).await {
                for d in docs {
                    if let Err(e) = cluster_days(&st, &d.id, &d.body, 2).await {
                        tracing::warn!(cluster = %d.id, error = %format!("{e:#}"), "Kubernetes costs");
                    }
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(3480)).await;
        }
    });
}

pub async fn clusters(State(st): State<AppState>) -> ApiResult<Value> {
    let docs = st.store.pg.docs(CLUSTER).await?;
    Ok(Json(
        json!({"data": docs.iter().map(|d| { let mut b = d.body.clone(); b["id"] = json!(d.id); b }).collect::<Vec<_>>()}),
    ))
}

pub async fn put_cluster(
    State(st): State<AppState>,
    id: Option<Path<String>>,
    Json(v): Json<Value>,
) -> ApiResult<Value> {
    let c: Cluster =
        serde_json::from_value(v.clone()).map_err(|e| ApiError::BadQuery(e.to_string()))?;
    if c.name.trim().is_empty() || c.monthly_cost < 0.0 || !(0.0..=1.0).contains(&c.cpu_share) {
        return Err(ApiError::BadQuery(
            "name, monthly_cost ≥ 0 and cpu_share in [0, 1] are needed".into(),
        ));
    }
    let id = id.map(|p| p.0).unwrap_or_else(new_id);
    let b = json!({"name": c.name.trim(), "filter": c.filter, "monthly_cost": c.monthly_cost, "currency": c.currency, "cpu_share": c.cpu_share});
    st.store.pg.put_doc(CLUSTER, &id, &b).await?;
    let st2 = st.clone();
    let (id2, b2) = (id.clone(), b.clone());
    tokio::spawn(async move {
        if let Err(e) = cluster_days(&st2, &id2, &b2, 7).await {
            tracing::warn!(error = %format!("{e:#}"), "Kubernetes costs");
        }
    });
    let mut out = b;
    out["id"] = json!(id);
    Ok(Json(out))
}

pub async fn create_cluster(st: State<AppState>, v: Json<Value>) -> ApiResult<Value> {
    put_cluster(st, None, v).await
}
pub async fn update_cluster(
    st: State<AppState>,
    id: Path<String>,
    v: Json<Value>,
) -> ApiResult<Value> {
    put_cluster(st, Some(id), v).await
}

pub async fn delete_cluster(
    State(st): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Value> {
    let db = st.store.pg.conn().await?;
    let sources: Vec<String> = db
        .query(
            "SELECT DISTINCT source_id FROM cost_items WHERE source_id LIKE $1",
            &[&format!("k8s:{id}:%")],
        )
        .await
        .map_err(anyhow::Error::from)?
        .iter()
        .map(|r| r.get(0))
        .collect();
    for s in sources {
        forget(&st, &s).await?;
    }
    st.store.pg.delete_doc(CLUSTER, &id).await?;
    Ok(Json(json!({})))
}

#[derive(Deserialize)]
pub struct Recompute {
    #[serde(default)]
    days: Option<i64>,
}

/// `POST /api/ui/cost/clusters/{id}/compute?days=30`.
pub async fn compute(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<Recompute>,
) -> ApiResult<Value> {
    let d = st
        .store
        .pg
        .doc(CLUSTER, &id)
        .await?
        .ok_or(ApiError::NotFound)?;
    let n = cluster_days(&st, &id, &d.body, q.days.unwrap_or(7).clamp(1, 90)).await?;
    Ok(Json(json!({"rows": n})))
}

// ---------- Explorer ----------

#[derive(Deserialize)]
pub struct ExploreReq {
    from: i64,
    to: i64,
    #[serde(default)]
    group: Option<String>,
    #[serde(default)]
    provider: Option<String>,
}

fn group_expr(g: &str) -> (String, Option<String>) {
    match g {
        "service" | "account" | "resource" | "currency" => (g.to_string(), None),
        "workload" => (
            format!(
                "COALESCE({}, NULLIF(resource, ''), service)",
                WORKLOADS.map(|w| format!("tags->>'{w}'")).join(", ")
            ),
            None,
        ),
        g if g.starts_with("tag:") => (
            "COALESCE(tags->>$4, 'N/A')".into(),
            Some(g[4..].to_string()),
        ),
        _ => ("provider".into(), None),
    }
}

/// `GET /api/ui/cost`: daily costs grouped, the top 12 groups.
pub async fn explore(State(st): State<AppState>, Query(q): Query<ExploreReq>) -> ApiResult<Value> {
    let (from, to) = (q.from / DAY * DAY, q.to);
    let (expr, tag) = group_expr(q.group.as_deref().unwrap_or("provider"));
    let provider = q.provider.unwrap_or_default();
    let db = st.store.pg.conn().await?;
    let sql = format!(
        "SELECT day, {expr} AS k, SUM(cost) FROM cost_items WHERE day >= $1 AND day < $2 AND ($3 = '' OR provider = $3) GROUP BY day, k"
    );
    let tag = tag.unwrap_or_default();
    let rows = if sql.contains("$4") {
        db.query(&sql, &[&from, &to, &provider, &tag]).await
    } else {
        db.query(&sql, &[&from, &to, &provider]).await
    }
    .map_err(anyhow::Error::from)?;
    let mut totals: HashMap<String, f64> = HashMap::new();
    let mut cells: Vec<(i64, String, f64)> = Vec::new();
    for r in &rows {
        let (d, k, c): (i64, Option<String>, f64) = (r.get(0), r.get(1), r.get(2));
        let k = k.unwrap_or_else(|| "N/A".into());
        *totals.entry(k.clone()).or_default() += c;
        cells.push((d, k, c));
    }
    let mut keys: Vec<(String, f64)> = totals.into_iter().collect();
    keys.sort_by(|a, b| b.1.total_cmp(&a.1));
    let top: Vec<String> = keys.iter().take(12).map(|k| k.0.clone()).collect();
    let days: Vec<i64> = (0..((to - from + DAY - 1) / DAY))
        .map(|i| from + i * DAY)
        .collect();
    let mut series: BTreeMap<String, Vec<f64>> = BTreeMap::new();
    for (d, k, c) in cells {
        let k = if top.contains(&k) { k } else { "other".into() };
        let v = series.entry(k).or_insert_with(|| vec![0.0; days.len()]);
        if let Some(i) = days.iter().position(|x| *x == d) {
            v[i] += c;
        }
    }
    let total: f64 = keys.iter().map(|k| k.1).sum::<f64>() + 0.0;
    let prev: f64 = db
        .query_one(
            "SELECT COALESCE(SUM(cost), 0) FROM cost_items WHERE day >= $1 AND day < $2 AND ($3 = '' OR provider = $3)",
            &[&(2 * from - to), &from, &provider],
        )
        .await
        .map_err(anyhow::Error::from)?
        .get(0);
    let currencies: Vec<String> = db
        .query(
            "SELECT DISTINCT currency FROM cost_items WHERE day >= $1 AND day < $2",
            &[&from, &to],
        )
        .await
        .map_err(anyhow::Error::from)?
        .iter()
        .map(|r| r.get(0))
        .collect();
    let mut groups: Vec<Value> = series
        .into_iter()
        .map(|(k, v)| json!({"key": k, "total": v.iter().sum::<f64>(), "values": v}))
        .collect();
    groups.sort_by(|a, b| {
        b["total"]
            .as_f64()
            .unwrap_or(0.0)
            .total_cmp(&a["total"].as_f64().unwrap_or(0.0))
    });
    Ok(Json(
        json!({"days": days, "groups": groups, "total": total, "previous_total": prev, "currencies": currencies}),
    ))
}

/// `GET /api/ui/cost/kubernetes`: per cluster, namespace and workload.
pub async fn kubernetes(
    State(st): State<AppState>,
    Query(q): Query<ExploreReq>,
) -> ApiResult<Value> {
    let db = st.store.pg.conn().await?;
    let (expr, _) = group_expr("workload");
    let rows = db
        .query(
            &format!(
                "SELECT account, service, {expr} AS w, SUM(cost), MAX(currency), AVG((extra->>'cpu_cores')::float8), AVG((extra->>'memory_bytes')::float8), COUNT(DISTINCT resource)
                 FROM cost_items WHERE provider = 'kubernetes' AND day >= $1 AND day < $2 GROUP BY account, service, w ORDER BY 4 DESC"
            ),
            &[&(q.from / DAY * DAY), &q.to],
        )
        .await
        .map_err(anyhow::Error::from)?;
    let data: Vec<Value> = rows
        .iter()
        .map(|r| {
            let service: String = r.get(1);
            json!({"cluster": r.get::<_, String>(0), "namespace": service, "workload": if service == "__idle__" { None } else { r.get::<_, Option<String>>(2) },
                "cost": r.get::<_, f64>(3), "currency": r.get::<_, Option<String>>(4), "cpu_cores": r.get::<_, Option<f64>>(5),
                "memory_bytes": r.get::<_, Option<f64>>(6), "pods": r.get::<_, i64>(7)})
        })
        .collect();
    Ok(Json(json!({"data": data})))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn focus_and_cur_files() {
        let focus = "ChargePeriodStart,ChargePeriodEnd,BilledCost,BillingCurrency,ProviderName,ServiceName,SubAccountId,ResourceId,Tags\n\
            2026-11-01T00:00:00Z,2026-11-02T00:00:00Z,1.5,EUR,OVHcloud,Object Storage,acc,b1,\"{\"\"team\"\":\"\"core\"\"}\"\n\
            2026-11-01T12:00:00Z,2026-11-01T13:00:00Z,2.5,EUR,OVHcloud,Object Storage,acc,b1,\"{\"\"team\"\":\"\"core\"\"}\"\n";
        let items = parse(focus.as_bytes(), "Custom").unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].cost, 4.0);
        assert_eq!(items[0].provider, "custom");
        assert_eq!(items[0].service, "Object Storage");
        assert_eq!(items[0].tags["team"], "core");
        let cur = "identity/LineItemId,lineItem/UsageStartDate,lineItem/UnblendedCost,lineItem/CurrencyCode,product/ProductName,lineItem/UsageAccountId,lineItem/ResourceId,resourceTags/user:env\n\
            x,2026-11-02T00:00:00Z,0.25,USD,Amazon Simple Storage Service,123,arn:s3,prod\n";
        let items = parse(cur.as_bytes(), "Custom").unwrap();
        assert_eq!(items[0].provider, "aws");
        assert_eq!(items[0].tags["env"], "prod");
        assert_eq!(items[0].day, day_of("2026-11-02").unwrap());
        let json = r#"[{"ProviderName":"GitHub","ChargeDescription":"Copilot","ChargePeriodStart":"2026-11-03","BilledCost":10,"BillingCurrency":"USD"}]"#;
        let items = parse(json.as_bytes(), "Custom").unwrap();
        assert_eq!(
            (items[0].service.as_str(), items[0].cost),
            ("Copilot", 10.0)
        );
        assert!(parse(b"a,b\n1,2\n", "Custom").is_err());
    }

    #[test]
    fn cluster_price_is_split_by_usage() {
        let c = Cluster {
            name: "silo01".into(),
            filter: "*".into(),
            monthly_cost: 365.0 / 12.0 * 10.0,
            currency: "EUR".into(),
            cpu_share: 0.5,
        };
        let pod = |ns: &str, cpu, mem| PodUsage {
            tags: BTreeMap::from([
                ("kube_namespace".into(), ns.into()),
                ("pod_name".into(), format!("{ns}-0")),
            ]),
            cpu,
            memory: mem,
        };
        // 10 a day; 8 cores and 100 GB.
        let items = allocate(
            &c,
            0,
            24.0,
            &[pod("a", 4.0, 50.0), pod("b", 0.0, 25.0)],
            8.0,
            100.0,
        );
        let get = |s: &str| items.iter().find(|i| i.service == s).unwrap().cost;
        assert!((get("a") - 5.0).abs() < 1e-9); // 0.5×4/8 + 0.5×50/100 = 0.5
        assert!((get("b") - 1.25).abs() < 1e-9);
        assert!((get("__idle__") - 3.75).abs() < 1e-9);
    }
}
