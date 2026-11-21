//! Reference Tables: CSV files of rows keyed by a primary key, as Datadog's.
//! The lookup processor enriches logs with a row (`lookup_enrichment_table`),
//! and queries filter on them: `@usr.id:reference_table(customers)` keeps
//! the logs whose attribute is one of the table's keys, and
//! `@usr.id:reference_table(customers, tier:gold)` those of the rows matching.

use crate::AppState;
use crate::api::{ApiError, ApiResult};
use crate::store::pg::new_id;
use axum::Json;
use axum::extract::{Path, Query, State};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::collections::HashMap;
use std::sync::{Arc, LazyLock, RwLock};

pub const KIND: &str = "reference_table";
const MAX_ROWS: usize = 200_000;
/// How many keys a query term expands to, at most.
const MAX_QUERY_KEYS: usize = 2_000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Field {
    pub name: String,
    /// `STRING` or `INT32`.
    #[serde(rename = "type", default = "string_type")]
    pub kind: String,
}

fn string_type() -> String {
    "STRING".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Table {
    pub table_name: String,
    #[serde(default)]
    pub description: String,
    pub primary_keys: Vec<String>,
    pub fields: Vec<Field>,
    #[serde(default)]
    pub tags: Vec<String>,
    /// The rows, in the order of `fields`.
    #[serde(default)]
    pub rows: Vec<Vec<String>>,
}

impl Table {
    fn row_object(&self, row: &[String]) -> Map<String, Value> {
        self.fields
            .iter()
            .zip(row)
            .map(|(f, v)| {
                let value = match f.kind.as_str() {
                    "INT32" => v
                        .trim()
                        .parse::<i64>()
                        .map(Value::from)
                        .unwrap_or(Value::Null),
                    _ => Value::String(v.clone()),
                };
                (f.name.clone(), value)
            })
            .collect()
    }

    fn key_index(&self) -> usize {
        let k = self.primary_keys.first().map(String::as_str).unwrap_or("");
        self.fields.iter().position(|f| f.name == k).unwrap_or(0)
    }
}

/// The tables in memory, by name: rows by primary key.
#[derive(Default)]
pub struct Loaded {
    tables: HashMap<String, (Arc<Table>, HashMap<String, usize>)>,
}

static TABLES: LazyLock<RwLock<Loaded>> = LazyLock::new(Default::default);

pub fn install(tables: Vec<Table>) {
    let mut l = Loaded::default();
    for t in tables {
        let k = t.key_index();
        let index = t
            .rows
            .iter()
            .enumerate()
            .filter_map(|(i, r)| r.get(k).map(|v| (v.clone(), i)))
            .collect();
        l.tables.insert(t.table_name.clone(), (Arc::new(t), index));
    }
    *TABLES.write().unwrap_or_else(|e| e.into_inner()) = l;
}

/// The row of a table with this key, as an object of its fields.
pub fn lookup(table: &str, key: &str) -> Option<Map<String, Value>> {
    let l = TABLES.read().unwrap_or_else(|e| e.into_inner());
    let (t, index) = l.tables.get(table)?;
    index.get(key).map(|&i| t.row_object(&t.rows[i]))
}

/// The keys of a table's rows, those whose `column` is `value` (`*` globs)
/// when given.
pub fn keys(table: &str, filter: Option<(&str, &str)>) -> Option<Vec<String>> {
    let l = TABLES.read().unwrap_or_else(|e| e.into_inner());
    let (t, _) = l.tables.get(table)?;
    let k = t.key_index();
    let col = match filter {
        Some((c, _)) => Some(t.fields.iter().position(|f| f.name == c)?),
        None => None,
    };
    Some(
        t.rows
            .iter()
            .filter(|r| match (col, filter) {
                (Some(c), Some((_, v))) => {
                    r.get(c).is_some_and(|x| crate::query::matcher::glob(v, x))
                }
                _ => true,
            })
            .filter_map(|r| r.get(k).cloned())
            .take(MAX_QUERY_KEYS)
            .collect(),
    )
}

static TERM: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(-?@?[\w.\-]+):reference_table\(\s*([\w.\-]+)\s*(?:,\s*([\w.\-]+)\s*:\s*("[^"]*"|[^)\s]+)\s*)?\)"#)
        .expect("regex")
});

fn quote(v: &str) -> String {
    format!("\"{}\"", v.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Query terms on reference tables, replaced by the keys they stand for.
pub fn expand(query: &str) -> Result<String, String> {
    if !query.contains("reference_table(") {
        return Ok(query.to_string());
    }
    let mut err = None;
    let out = TERM.replace_all(query, |c: &regex::Captures| {
        let attr = &c[1];
        let table = &c[2];
        let filter = c.get(3).map(|col| {
            let v = c[4].trim_matches('"').to_string();
            (col.as_str().to_string(), v)
        });
        match keys(
            table,
            filter.as_ref().map(|(a, b)| (a.as_str(), b.as_str())),
        ) {
            None => {
                err = Some(format!("no reference table {table} with these columns"));
                String::new()
            }
            // No key: nothing can match.
            Some(k) if k.is_empty() => format!("{attr}:\"\u{0}none\""),
            Some(k) => format!(
                "{attr}:({})",
                k.iter().map(|v| quote(v)).collect::<Vec<_>>().join(" OR ")
            ),
        }
    });
    match err {
        Some(e) => Err(e),
        None => Ok(out.into_owned()),
    }
}

/// A CSV file: its header and rows (quotes, `""` escapes, CRLF).
pub fn parse_csv(text: &str) -> Result<(Vec<String>, Vec<Vec<String>>), String> {
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.trim_start_matches('\u{feff}').chars().peekable();
    let mut any = false;
    while let Some(c) = chars.next() {
        any = true;
        if quoted {
            match c {
                '"' if chars.peek() == Some(&'"') => {
                    chars.next();
                    field.push('"');
                }
                '"' => quoted = false,
                c => field.push(c),
            }
            continue;
        }
        match c {
            '"' if field.is_empty() => quoted = true,
            ',' => row.push(std::mem::take(&mut field)),
            '\r' => {}
            '\n' => {
                row.push(std::mem::take(&mut field));
                if !(row.len() == 1 && row[0].is_empty()) {
                    rows.push(std::mem::take(&mut row));
                }
                row.clear();
            }
            c => field.push(c),
        }
    }
    if quoted {
        return Err("unterminated quote".into());
    }
    if any && (!field.is_empty() || !row.is_empty()) {
        row.push(field);
        rows.push(row);
    }
    if rows.is_empty() {
        return Err("the file is empty".into());
    }
    let header: Vec<String> = rows
        .remove(0)
        .into_iter()
        .map(|h| h.trim().to_string())
        .collect();
    if header.iter().any(|h| h.is_empty()) {
        return Err("a column has no name".into());
    }
    for (i, r) in rows.iter().enumerate() {
        if r.len() != header.len() {
            return Err(format!(
                "row {} has {} fields, the header {}",
                i + 2,
                r.len(),
                header.len()
            ));
        }
    }
    Ok((header, rows))
}

fn valid_name(n: &str) -> bool {
    !n.is_empty()
        && n.len() <= 56
        && n.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        && n.starts_with(|c: char| c.is_ascii_lowercase())
}

/// A table from a CSV file, keyed by `primary_key` (the first column by default).
pub fn from_csv(
    name: &str,
    description: &str,
    primary_key: Option<&str>,
    csv: &str,
    tags: Vec<String>,
) -> Result<Table, String> {
    if !valid_name(name) {
        return Err(
            "the name is lowercase letters, digits and underscores, starting with a letter".into(),
        );
    }
    let (header, rows) = parse_csv(csv)?;
    if rows.len() > MAX_ROWS {
        return Err(format!("at most {MAX_ROWS} rows"));
    }
    let key = primary_key
        .map(str::trim)
        .filter(|k| !k.is_empty())
        .unwrap_or(&header[0]);
    if !header.iter().any(|h| h == key) {
        return Err(format!("no column {key}"));
    }
    let k = header.iter().position(|h| h == key).unwrap_or(0);
    let mut seen = std::collections::HashSet::new();
    for r in &rows {
        if !seen.insert(&r[k]) {
            return Err(format!("the key {} is on several rows", r[k]));
        }
    }
    // Columns of integers only are INT32, as Datadog infers them.
    let fields = header
        .iter()
        .enumerate()
        .map(|(i, h)| Field {
            name: h.clone(),
            kind: if !rows.is_empty()
                && rows.iter().all(|r| r[i].trim().parse::<i32>().is_ok())
                && i != k
            {
                "INT32".into()
            } else {
                "STRING".into()
            },
        })
        .collect();
    Ok(Table {
        table_name: name.into(),
        description: description.into(),
        primary_keys: vec![key.to_string()],
        fields,
        tags,
        rows,
    })
}

async fn stored(st: &AppState) -> anyhow::Result<Vec<(String, i64, Table)>> {
    Ok(st
        .store
        .pg
        .docs(KIND)
        .await?
        .into_iter()
        .filter_map(|d| Some((d.id, d.updated_at, serde_json::from_value(d.body).ok()?)))
        .collect())
}

pub async fn reload(st: &AppState) -> anyhow::Result<()> {
    install(stored(st).await?.into_iter().map(|(_, _, t)| t).collect());
    Ok(())
}

fn summary(id: &str, updated: i64, t: &Table) -> Value {
    json!({
        "id": id,
        "type": "reference_table",
        "attributes": {
            "table_name": t.table_name,
            "description": t.description,
            "source": "LOCAL_FILE",
            "status": "DONE",
            "row_count": t.rows.len(),
            "schema": {"primary_keys": t.primary_keys, "fields": t.fields},
            "tags": t.tags,
            "updated_at": super::iso(updated),
        }
    })
}

/// `GET /api/v2/reference-tables/tables`.
pub async fn list(State(st): State<AppState>) -> ApiResult<Value> {
    let mut v: Vec<Value> = stored(&st)
        .await?
        .iter()
        .map(|(id, u, t)| summary(id, *u, t))
        .collect();
    v.sort_by(|a, b| {
        a["attributes"]["table_name"]
            .as_str()
            .cmp(&b["attributes"]["table_name"].as_str())
    });
    Ok(Json(json!({"data": v})))
}

async fn find(st: &AppState, id: &str) -> Result<(String, i64, Table), ApiError> {
    stored(st)
        .await?
        .into_iter()
        .find(|(i, _, t)| i == id || t.table_name == id)
        .ok_or(ApiError::NotFound)
}

/// `GET /api/v2/reference-tables/tables/{id}`.
pub async fn get(State(st): State<AppState>, Path(id): Path<String>) -> ApiResult<Value> {
    let (id, u, t) = find(&st, &id).await?;
    Ok(Json(json!({"data": summary(&id, u, &t)})))
}

#[derive(Deserialize)]
pub struct RowsQuery {
    #[serde(default, rename = "filter[row_id]")]
    row_id: Option<String>,
    #[serde(default)]
    search: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
}

/// `GET /api/v2/reference-tables/tables/{id}/rows`: rows by key
/// (`filter[row_id]=a,b`), or the first ones (with `search`, those holding it).
pub async fn rows(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<RowsQuery>,
) -> ApiResult<Value> {
    let (_, _, t) = find(&st, &id).await?;
    let k = t.key_index();
    let wanted: Option<Vec<&str>> = q
        .row_id
        .as_deref()
        .map(|r| r.split(',').map(str::trim).collect());
    let needle = q
        .search
        .as_deref()
        .map(str::to_lowercase)
        .filter(|s| !s.is_empty());
    let limit = q.limit.unwrap_or(100).min(1000);
    let rows: Vec<Value> = t
        .rows
        .iter()
        .filter(|r| wanted.as_ref().is_none_or(|w| w.contains(&r[k].as_str())))
        .filter(|r| {
            needle
                .as_ref()
                .is_none_or(|n| r.iter().any(|x| x.to_lowercase().contains(n)))
        })
        .take(limit)
        .map(|r| json!({"id": r[k], "type": "row", "attributes": {"values": t.row_object(r)}}))
        .collect();
    Ok(Json(json!({"data": rows, "meta": {"total": t.rows.len()}})))
}

#[derive(Deserialize)]
struct Upload {
    table_name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    primary_key: Option<String>,
    #[serde(default)]
    tags: Vec<String>,
    /// The CSV file's content.
    csv: String,
}

/// The body: robine-dog's `{table_name, primary_key, csv}`, or Datadog's
/// JSON:API with the file inline (`attributes.file_metadata.content`).
fn read_upload(b: Value) -> Result<Upload, ApiError> {
    if let Some(a) = b.pointer("/data/attributes") {
        let csv = a
            .pointer("/file_metadata/content")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                ApiError::BadQuery("attributes.file_metadata.content: the CSV file".into())
            })?;
        return Ok(Upload {
            table_name: a["table_name"].as_str().unwrap_or_default().into(),
            description: a["description"].as_str().unwrap_or_default().into(),
            primary_key: a
                .pointer("/schema/primary_keys/0")
                .and_then(Value::as_str)
                .map(Into::into),
            tags: serde_json::from_value(a["tags"].clone()).unwrap_or_default(),
            csv: csv.into(),
        });
    }
    serde_json::from_value(b).map_err(|e| ApiError::BadQuery(e.to_string()))
}

/// `POST /api/v2/reference-tables/tables`.
pub async fn create(State(st): State<AppState>, Json(b): Json<Value>) -> ApiResult<Value> {
    let u = read_upload(b)?;
    if find(&st, &u.table_name).await.is_ok() {
        return Err(ApiError::BadQuery(format!(
            "a table {} exists",
            u.table_name
        )));
    }
    let t = from_csv(
        &u.table_name,
        &u.description,
        u.primary_key.as_deref(),
        &u.csv,
        u.tags,
    )
    .map_err(ApiError::BadQuery)?;
    let id = new_id();
    let d = st
        .store
        .pg
        .put_doc(
            KIND,
            &id,
            &serde_json::to_value(&t).map_err(anyhow::Error::from)?,
        )
        .await?;
    reload(&st).await?;
    Ok(Json(json!({"data": summary(&id, d.updated_at, &t)})))
}

/// `PATCH /api/v2/reference-tables/tables/{id}`: a new file, or a new description.
pub async fn update(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Json(b): Json<Value>,
) -> ApiResult<Value> {
    let (id, _, old) = find(&st, &id).await?;
    let a = b.pointer("/data/attributes").cloned().unwrap_or(b);
    let csv = a
        .pointer("/file_metadata/content")
        .or_else(|| a.get("csv"))
        .and_then(Value::as_str);
    let description = a["description"]
        .as_str()
        .unwrap_or(&old.description)
        .to_string();
    let t = match csv {
        Some(csv) => from_csv(
            &old.table_name,
            &description,
            Some(old.primary_keys.first().map(String::as_str).unwrap_or("")),
            csv,
            old.tags.clone(),
        )
        .map_err(ApiError::BadQuery)?,
        None => Table { description, ..old },
    };
    let d = st
        .store
        .pg
        .put_doc(
            KIND,
            &id,
            &serde_json::to_value(&t).map_err(anyhow::Error::from)?,
        )
        .await?;
    reload(&st).await?;
    Ok(Json(json!({"data": summary(&id, d.updated_at, &t)})))
}

/// `DELETE /api/v2/reference-tables/tables/{id}`.
pub async fn delete(State(st): State<AppState>, Path(id): Path<String>) -> ApiResult<Value> {
    let (id, _, _) = find(&st, &id).await?;
    st.store.pg.delete_doc(KIND, &id).await?;
    reload(&st).await?;
    Ok(Json(json!({})))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_and_queries() {
        let t = from_csv(
            "customers",
            "",
            Some("id"),
            "id,name,tier,seats\r\nc1,\"Acme, Inc.\",gold,12\nc2,\"Say \"\"hi\"\"\",silver,3\nc3,Globex,gold,40\n",
            vec![],
        )
        .unwrap();
        assert_eq!(t.rows[0][1], "Acme, Inc.");
        assert_eq!(t.rows[1][1], "Say \"hi\"");
        assert_eq!(t.fields[3].kind, "INT32");
        assert_eq!(t.fields[0].kind, "STRING");
        install(vec![t]);
        assert_eq!(lookup("customers", "c3").unwrap()["seats"], json!(40));
        assert!(lookup("customers", "c9").is_none());
        assert_eq!(
            expand("status:error @usr.id:reference_table(customers, tier:gold)").unwrap(),
            r#"status:error @usr.id:("c1" OR "c3")"#
        );
        assert_eq!(
            expand("-@org:reference_table(customers)").unwrap(),
            r#"-@org:("c1" OR "c2" OR "c3")"#
        );
        assert!(expand("@a:reference_table(nope)").is_err());
        assert!(from_csv("Bad-Name", "", None, "a\n1", vec![]).is_err());
        assert!(from_csv("t", "", None, "a,b\n1", vec![]).is_err());
        assert!(from_csv("t", "", None, "a\n1\n1", vec![]).is_err());
        crate::api::parse_query(
            &expand("@usr.id:reference_table(customers, tier:silver)").unwrap(),
        )
        .unwrap();
    }
}
