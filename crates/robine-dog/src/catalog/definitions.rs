//! Software Catalog service definitions, as Datadog's
//! `/api/v2/services/definitions`: schema v2, v2.1 or v2.2, posted as JSON
//! or YAML. Kept as posted (one per `dd-service`), read through [`Summary`].

use crate::AppState;
use crate::api::{ApiError, ApiResult};
use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use serde::Serialize;
use serde_json::{Map, Value, json};

pub const KIND: &str = "service_definition";
const VERSIONS: &[&str] = &["v2", "v2.1", "v2.2"];
const CONTACT_TYPES: &[&str] = &["email", "slack", "microsoft-teams", "website", "link"];
const LINK_TYPES: &[&str] = &["doc", "repo", "runbook", "dashboard", "other"];

/// What the rest of robine-dog reads in a definition.
#[derive(Debug, Clone, Serialize, Default, PartialEq)]
pub struct Summary {
    pub service: String,
    pub team: Option<String>,
    pub description: Option<String>,
    pub tier: Option<String>,
    pub lifecycle: Option<String>,
    pub application: Option<String>,
    pub languages: Vec<String>,
    #[serde(rename = "type")]
    pub typ: Option<String>,
    pub contacts: Vec<Value>,
    /// `{name, type, url}`; v2's `repos` and `docs` become `repo` and `doc` links.
    pub links: Vec<Value>,
    pub tags: Vec<String>,
}

fn text(v: &Value) -> Option<String> {
    match v {
        Value::String(s) if !s.trim().is_empty() => Some(s.trim().to_string()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .map(|a| a.iter().filter_map(text).collect())
        .unwrap_or_default()
}

pub fn summary(schema: &Value) -> Summary {
    let mut links: Vec<Value> = schema["links"].as_array().cloned().unwrap_or_default();
    for (key, kind) in [("repos", "repo"), ("docs", "doc")] {
        for l in schema[key].as_array().into_iter().flatten() {
            links.push(json!({"name": l["name"], "type": kind, "url": l["url"]}));
        }
    }
    Summary {
        service: text(&schema["dd-service"]).unwrap_or_default(),
        team: text(&schema["team"]),
        description: text(&schema["description"]),
        tier: text(&schema["tier"]),
        lifecycle: text(&schema["lifecycle"]),
        application: text(&schema["application"]),
        languages: strings(&schema["languages"]),
        typ: text(&schema["type"]),
        contacts: schema["contacts"].as_array().cloned().unwrap_or_default(),
        links,
        tags: strings(&schema["tags"]),
    }
}

/// A definition from JSON or YAML, checked against the schema's rules.
pub fn parse(body: &[u8]) -> Result<Value, String> {
    let text = std::str::from_utf8(body).map_err(|_| "the body is not UTF-8".to_string())?;
    let v: Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(_) => serde_yaml::from_str(text).map_err(|e| format!("neither JSON nor YAML: {e}"))?,
    };
    let v = v.pointer("/data/attributes/schema").cloned().unwrap_or(v);
    validate(&v)?;
    Ok(v)
}

fn validate(v: &Value) -> Result<(), String> {
    let o = v.as_object().ok_or("a definition is an object")?;
    let version = o
        .get("schema-version")
        .and_then(Value::as_str)
        .ok_or("schema-version is missing")?;
    if !VERSIONS.contains(&version) {
        return Err(format!(
            "schema-version {version} is not supported: {}",
            VERSIONS.join(", ")
        ));
    }
    let service = o
        .get("dd-service")
        .and_then(Value::as_str)
        .ok_or("dd-service is missing")?;
    if service.is_empty()
        || service.len() > 100
        || !service
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "-_./:".contains(c))
    {
        return Err("dd-service is lowercase letters, digits and - _ . / :".into());
    }
    for c in o
        .get("contacts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let t = c["type"].as_str().unwrap_or("");
        if !CONTACT_TYPES.contains(&t) {
            return Err(format!(
                "contacts: type {t:?} is not one of {}",
                CONTACT_TYPES.join(", ")
            ));
        }
        if text(&c["contact"]).is_none() {
            return Err("contacts: contact is missing".into());
        }
    }
    if version != "v2" {
        for l in o
            .get("links")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let t = l["type"].as_str().unwrap_or("");
            if !LINK_TYPES.contains(&t) {
                return Err(format!(
                    "links: type {t:?} is not one of {}",
                    LINK_TYPES.join(", ")
                ));
            }
            if text(&l["name"]).is_none() || text(&l["url"]).is_none() {
                return Err("links: name and url are required".into());
            }
        }
    }
    if let Some(l) = o.get("lifecycle").and_then(Value::as_str)
        && l.trim().is_empty()
    {
        return Err("lifecycle is empty".into());
    }
    Ok(())
}

fn out(schema: &Value, updated: i64) -> Value {
    json!({
        "type": "service-definition",
        "id": schema["dd-service"],
        "attributes": {
            "schema": schema,
            "meta": {
                "ingestion-source": "api",
                "ingested-schema-version": schema["schema-version"],
                "last-modified-time": crate::logconfig::iso(updated),
                "origin": "robine-dog",
                "origin-detail": "",
                "warnings": [],
                "github-html-url": "",
            }
        }
    })
}

/// Every definition, by service.
pub async fn all(st: &AppState) -> anyhow::Result<Vec<(Value, i64)>> {
    let mut v: Vec<(Value, i64)> = st
        .store
        .pg
        .docs(KIND)
        .await?
        .into_iter()
        .map(|d| (d.body, d.updated_at))
        .collect();
    v.sort_by(|a, b| a.0["dd-service"].as_str().cmp(&b.0["dd-service"].as_str()));
    Ok(v)
}

pub async fn summaries(st: &AppState) -> anyhow::Result<Vec<Summary>> {
    Ok(all(st).await?.iter().map(|(v, _)| summary(v)).collect())
}

/// `GET /api/v2/services/definitions`.
pub async fn list(State(st): State<AppState>) -> ApiResult<Value> {
    let data: Vec<Value> = all(&st).await?.iter().map(|(v, u)| out(v, *u)).collect();
    Ok(Json(json!({"data": data, "meta": {}})))
}

/// `GET /api/v2/services/definitions/{service_name}`.
pub async fn get(State(st): State<AppState>, Path(name): Path<String>) -> ApiResult<Value> {
    let d = st
        .store
        .pg
        .doc(KIND, &name)
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(json!({"data": out(&d.body, d.updated_at)})))
}

/// `POST /api/v2/services/definitions`: creates or replaces.
pub async fn upsert(State(st): State<AppState>, body: Bytes) -> ApiResult<Value> {
    let v = parse(&body).map_err(ApiError::BadQuery)?;
    let name = v["dd-service"].as_str().unwrap_or_default().to_string();
    let d = st.store.pg.put_doc(KIND, &name, &v).await?;
    Ok(Json(json!({"data": [out(&d.body, d.updated_at)]})))
}

/// `DELETE /api/v2/services/definitions/{service_name}`.
pub async fn delete(State(st): State<AppState>, Path(name): Path<String>) -> ApiResult<Value> {
    if !st.store.pg.delete_doc(KIND, &name).await? {
        return Err(ApiError::NotFound);
    }
    Ok(Json(json!({})))
}

/// A starting definition for a service, in YAML, for the UI's editor.
pub fn template(service: &str) -> String {
    let mut m = Map::new();
    m.insert("schema-version".into(), json!("v2.2"));
    m.insert("dd-service".into(), json!(service));
    m.insert("team".into(), json!(""));
    m.insert(
        "contacts".into(),
        json!([{"name": "Support", "type": "email", "contact": "team@example.com"}]),
    );
    m.insert(
        "links".into(),
        json!([{"name": "Repository", "type": "repo", "provider": "github",
                "url": format!("https://github.com/example/{service}")},
               {"name": "Runbook", "type": "runbook",
                "url": format!("https://wiki.example.com/{service}/runbook")}]),
    );
    m.insert("tier".into(), json!("1"));
    m.insert("lifecycle".into(), json!("production"));
    m.insert("description".into(), json!(""));
    serde_yaml::to_string(&Value::Object(m)).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn yaml_and_json_definitions() {
        let yaml = b"schema-version: v2.2
dd-service: shop-api
team: storefront
tier: 1
lifecycle: production
contacts:
  - name: Support
    type: email
    contact: shop@example.com
links:
  - name: Source
    type: repo
    provider: github
    url: https://github.com/example/shop
languages: [rust]
";
        let v = parse(yaml).unwrap();
        let s = summary(&v);
        assert_eq!(s.service, "shop-api");
        assert_eq!(s.team.as_deref(), Some("storefront"));
        assert_eq!(s.tier.as_deref(), Some("1"));
        assert_eq!(s.links[0]["type"], "repo");
        assert_eq!(s.languages, vec!["rust"]);

        let v2 = parse(br#"{"schema-version":"v2","dd-service":"web","repos":[{"name":"src","url":"https://x"}],"docs":[{"name":"d","url":"https://y"}]}"#).unwrap();
        let s = summary(&v2);
        assert_eq!(s.links.len(), 2);
        assert_eq!(s.links[1]["type"], "doc");

        assert!(
            parse(b"dd-service: x")
                .unwrap_err()
                .contains("schema-version")
        );
        assert!(parse(b"schema-version: v3\ndd-service: x").is_err());
        assert!(parse(b"schema-version: v2.2\ndd-service: Shop").is_err());
        assert!(
            parse(b"schema-version: v2.2\ndd-service: x\ncontacts: [{type: pager, contact: a}]")
                .unwrap_err()
                .contains("contacts")
        );
        assert!(parse(template("shop").as_bytes()).is_ok());
    }
}
