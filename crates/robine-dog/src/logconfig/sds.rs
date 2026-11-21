//! Sensitive Data Scanner, as Datadog's: groups of rules, each group on the
//! logs its filter matches; a rule is a regex (or one of the standard
//! patterns) scanned in the message and the attributes (`namespaces` limits
//! them, `excluded_namespaces` skips some). What it finds is redacted, hashed
//! or partially masked, or only tagged, and the log gets the rule's tags
//! (`sensitive_data:<rule>`, `sensitive_data_category:<category>`). It runs at
//! intake, after the pipelines, so nothing found is stored.

use crate::AppState;
use crate::api::{ApiError, ApiResult};
use crate::intake::logs::LogRow;
use crate::query::matcher::matches;
use crate::query::parser::Expr;
use crate::store::pg::new_id;
use axum::Json;
use axum::extract::{Path, State};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::sync::{Arc, RwLock};

pub const KIND: &str = "sds_config";
const DOC_ID: &str = "config";
const CONFIG_ID: &str = "robine-dog";

pub struct Standard {
    pub id: &'static str,
    pub name: &'static str,
    pub pattern: &'static str,
    pub category: &'static str,
    pub description: &'static str,
    /// A second check of what the regex finds.
    pub check: Option<fn(&str) -> bool>,
}

/// The Luhn checksum of card numbers.
pub fn luhn(s: &str) -> bool {
    let digits: Vec<u32> = s.chars().filter_map(|c| c.to_digit(10)).collect();
    if !(13..=19).contains(&digits.len()) {
        return false;
    }
    let sum: u32 = digits
        .iter()
        .rev()
        .enumerate()
        .map(|(i, &d)| {
            if i % 2 == 1 {
                let x = d * 2;
                if x > 9 { x - 9 } else { x }
            } else {
                d
            }
        })
        .sum();
    sum.is_multiple_of(10)
}

/// The ISO 13616 checksum of IBANs.
pub fn iban(s: &str) -> bool {
    let s: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    if s.len() < 15 || s.len() > 34 {
        return false;
    }
    let moved = format!("{}{}", &s[4..], &s[..4]);
    let mut rem: u64 = 0;
    for c in moved.chars() {
        let v = match c.to_digit(36) {
            Some(v) => v as u64,
            None => return false,
        };
        rem = if v >= 10 {
            (rem * 100 + v) % 97
        } else {
            (rem * 10 + v) % 97
        };
    }
    rem == 1
}

pub const STANDARD: &[Standard] = &[
    Standard {
        id: "credit_card",
        name: "Credit card number",
        pattern: r"\b(?:\d[ -]?){12,18}\d\b",
        category: "credit_card",
        description: "Visa, Mastercard, Amex and other card numbers, checked with Luhn",
        check: Some(luhn),
    },
    Standard {
        id: "email_address",
        name: "Email address",
        pattern: r"\b[A-Za-z0-9._%+\-]+@[A-Za-z0-9.\-]+\.[A-Za-z]{2,}\b",
        category: "pii",
        description: "Email addresses",
        check: None,
    },
    Standard {
        id: "iban",
        name: "IBAN",
        pattern: r"\b[A-Z]{2}\d{2}(?: ?[A-Z0-9]{4}){2,7}(?: ?[A-Z0-9]{1,4})?\b",
        category: "financial",
        description: "International bank account numbers, with their checksum",
        check: Some(iban),
    },
    Standard {
        id: "us_ssn",
        name: "US Social Security Number",
        pattern: r"\b\d{3}-\d{2}-\d{4}\b",
        category: "pii",
        description: "US social security numbers (123-45-6789)",
        check: None,
    },
    Standard {
        id: "phone_number",
        name: "International phone number",
        pattern: r"\+\d{1,3}(?:[ .\-]?\d{1,4}){2,5}\b",
        category: "pii",
        description: "Phone numbers with a country code",
        check: None,
    },
    Standard {
        id: "ipv4_address",
        name: "IPv4 address",
        pattern: r"\b(?:(?:25[0-5]|2[0-4]\d|1?\d?\d)\.){3}(?:25[0-5]|2[0-4]\d|1?\d?\d)\b",
        category: "network",
        description: "IPv4 addresses",
        check: None,
    },
    Standard {
        id: "aws_access_key_id",
        name: "AWS Access Key ID",
        pattern: r"\b(?:AKIA|ASIA)[0-9A-Z]{16}\b",
        category: "credentials",
        description: "AWS access key IDs",
        check: None,
    },
    Standard {
        id: "json_web_token",
        name: "JSON Web Token",
        pattern: r"\beyJ[A-Za-z0-9_\-]+\.eyJ[A-Za-z0-9_\-]+\.[A-Za-z0-9_\-]+",
        category: "credentials",
        description: "JWTs",
        check: None,
    },
    Standard {
        id: "bearer_token",
        name: "Bearer token",
        pattern: r"(?i)\bbearer\s+[A-Za-z0-9\-._~+/]+=*",
        category: "credentials",
        description: "Authorization: Bearer tokens",
        check: None,
    },
    Standard {
        id: "github_token",
        name: "GitHub token",
        pattern: r"\bgh[pousr]_[A-Za-z0-9]{36,}\b",
        category: "credentials",
        description: "GitHub personal, OAuth and app tokens",
        check: None,
    },
    Standard {
        id: "slack_token",
        name: "Slack token",
        pattern: r"\bxox[abposr]-[A-Za-z0-9\-]{10,}\b",
        category: "credentials",
        description: "Slack bot, user and app tokens",
        check: None,
    },
    Standard {
        id: "stripe_secret_key",
        name: "Stripe secret key",
        pattern: r"\b[rs]k_live_[0-9A-Za-z]{20,}\b",
        category: "credentials",
        description: "Stripe live secret and restricted keys",
        check: None,
    },
    Standard {
        id: "private_key",
        name: "Private key",
        pattern: r"-----BEGIN (?:[A-Z]+ )*PRIVATE KEY-----[\s\S]*?(?:-----END (?:[A-Z]+ )*PRIVATE KEY-----|$)",
        category: "credentials",
        description: "PEM private keys",
        check: None,
    },
    Standard {
        id: "password_in_url",
        name: "Password in URL",
        pattern: r"(?i)\b[a-z][a-z0-9+.\-]*://[^:/\s@]+:[^@/\s]+@",
        category: "credentials",
        description: "user:password@ in URLs",
        check: None,
    },
];

pub fn standard(id: &str) -> Option<&'static Standard> {
    STANDARD.iter().find(|s| s.id == id)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Replacement {
    /// `none`, `hash`, `replacement_string`,
    /// `partial_replacement_from_beginning` or `partial_replacement_from_end`.
    #[serde(rename = "type", default = "none")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replacement_string: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub number_of_chars: Option<usize>,
}

fn none() -> String {
    "none".into()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Rule {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub pattern: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub standard_pattern_id: Option<String>,
    #[serde(default)]
    pub namespaces: Vec<String>,
    #[serde(default)]
    pub excluded_namespaces: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub text_replacement: Replacement,
    #[serde(default = "yes")]
    pub is_enabled: bool,
    #[serde(default = "priority")]
    pub priority: i64,
}

fn yes() -> bool {
    true
}

fn priority() -> i64 {
    1
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Group {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "all")]
    pub filter: Value,
    #[serde(default = "yes")]
    pub is_enabled: bool,
    #[serde(default = "logs")]
    pub product_list: Vec<String>,
    #[serde(default)]
    pub rules: Vec<Rule>,
}

fn all() -> Value {
    json!({"query": "*"})
}

fn logs() -> Vec<String> {
    vec!["logs".into()]
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub version: i64,
    #[serde(default)]
    pub groups: Vec<Group>,
}

impl Group {
    fn query(&self) -> &str {
        self.filter["query"].as_str().unwrap_or("*")
    }
}

struct CompiledRule {
    name: String,
    regex: Regex,
    check: Option<fn(&str) -> bool>,
    namespaces: Vec<String>,
    excluded: Vec<String>,
    tags: Vec<String>,
    replacement: Replacement,
}

struct CompiledGroup {
    filter: Expr,
    rules: Vec<CompiledRule>,
}

/// The enabled groups and rules, ready to scan.
#[derive(Default)]
pub struct Scanner {
    groups: RwLock<Arc<Vec<CompiledGroup>>>,
}

pub fn slug(s: &str) -> String {
    let mut out = String::new();
    for c in s.to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('_') {
            out.push('_');
        }
    }
    out.trim_matches('_').to_string()
}

/// A rule's regex, its checksum (Luhn…) and its standard category.
type RuleRegex = (Regex, Option<fn(&str) -> bool>, Option<&'static str>);

fn rule_regex(r: &Rule) -> Result<RuleRegex, String> {
    if let Some(sp) = r.standard_pattern_id.as_deref().filter(|s| !s.is_empty()) {
        let s = standard(sp).ok_or_else(|| format!("no standard pattern {sp}"))?;
        return Ok((
            Regex::new(s.pattern).map_err(|e| e.to_string())?,
            s.check,
            Some(s.category),
        ));
    }
    if r.pattern.is_empty() {
        return Err("a rule has a pattern or a standard pattern".into());
    }
    if r.pattern.len() > 1000 {
        return Err("the pattern is longer than 1000 characters".into());
    }
    let re = regex::RegexBuilder::new(&r.pattern)
        .size_limit(1 << 20)
        .build()
        .map_err(|e| format!("pattern: {e}"))?;
    Ok((re, None, None))
}

pub fn validate_rule(r: &Rule) -> Result<(), String> {
    if r.name.trim().is_empty() {
        return Err("the rule has no name".into());
    }
    rule_regex(r)?;
    match r.text_replacement.kind.as_str() {
        "none" | "hash" => {}
        "replacement_string" => {
            if r.text_replacement.replacement_string.is_none() {
                return Err("text_replacement.replacement_string is missing".into());
            }
        }
        "partial_replacement_from_beginning" | "partial_replacement_from_end" => {
            if r.text_replacement.number_of_chars.unwrap_or(0) == 0 {
                return Err("text_replacement.number_of_chars is missing".into());
            }
        }
        k => return Err(format!("unknown text_replacement.type {k}")),
    }
    Ok(())
}

impl Scanner {
    pub fn load(&self, cfg: &Config) {
        let mut groups = Vec::new();
        for g in cfg
            .groups
            .iter()
            .filter(|g| g.is_enabled && g.product_list.iter().any(|p| p == "logs"))
        {
            let Ok(filter) = crate::query::parser::parse(g.query()) else {
                continue;
            };
            let rules = g
                .rules
                .iter()
                .filter(|r| r.is_enabled)
                .filter_map(|r| {
                    let (regex, check, category) = rule_regex(r).ok()?;
                    let mut tags = r.tags.clone();
                    if !tags.iter().any(|t| t.starts_with("sensitive_data:")) {
                        tags.push(format!("sensitive_data:{}", slug(&r.name)));
                    }
                    if let Some(c) = category
                        && !tags
                            .iter()
                            .any(|t| t.starts_with("sensitive_data_category:"))
                    {
                        tags.push(format!("sensitive_data_category:{c}"));
                    }
                    Some(CompiledRule {
                        name: r.name.clone(),
                        regex,
                        check,
                        namespaces: r.namespaces.clone(),
                        excluded: r.excluded_namespaces.clone(),
                        tags,
                        replacement: r.text_replacement.clone(),
                    })
                })
                .collect();
            groups.push(CompiledGroup { filter, rules });
        }
        *self.groups.write().unwrap_or_else(|e| e.into_inner()) = Arc::new(groups);
    }

    /// Scans a log, redacting it; returns the names of the rules that matched.
    pub fn scan(&self, log: &mut LogRow) -> Vec<String> {
        let groups = self
            .groups
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let mut hits = Vec::new();
        for g in groups.iter() {
            if g.rules.is_empty() || !matches(&g.filter, log) {
                continue;
            }
            for r in &g.rules {
                let mut found = false;
                if r.namespaces.is_empty() && !r.excluded.iter().any(|e| e == "message") {
                    if let Some(s) = redact(r, &log.message) {
                        log.message = s;
                        found = true;
                    } else if r.replacement.kind == "none" && finds(r, &log.message) {
                        found = true;
                    }
                }
                found |= scan_map(r, &mut log.attributes, "");
                if found {
                    hits.push(r.name.clone());
                    for t in &r.tags {
                        if !log.tags.contains(t) {
                            log.tags.push(t.clone());
                        }
                    }
                }
            }
        }
        hits
    }
}

fn in_scope(r: &CompiledRule, path: &str) -> bool {
    let under = |ns: &String| path == ns || path.starts_with(&format!("{ns}."));
    (r.namespaces.is_empty() || r.namespaces.iter().any(under)) && !r.excluded.iter().any(under)
}

fn scan_map(r: &CompiledRule, m: &mut Map<String, Value>, prefix: &str) -> bool {
    let mut found = false;
    for (k, v) in m.iter_mut() {
        let path = if prefix.is_empty() {
            k.clone()
        } else {
            format!("{prefix}.{k}")
        };
        found |= scan_value(r, v, &path);
    }
    found
}

fn scan_value(r: &CompiledRule, v: &mut Value, path: &str) -> bool {
    match v {
        Value::String(s) => {
            // A namespace can be the parent of the value: scope checks the path.
            if !in_scope(r, path) {
                return false;
            }
            if let Some(n) = redact(r, s) {
                *s = n;
                true
            } else {
                finds(r, s)
            }
        }
        Value::Number(n) => {
            if !in_scope(r, path) {
                return false;
            }
            let s = n.to_string();
            match redact(r, &s) {
                Some(x) => {
                    *v = Value::String(x);
                    true
                }
                None => finds(r, &s),
            }
        }
        Value::Object(m) => scan_map(r, m, path),
        Value::Array(a) => a.iter_mut().fold(false, |f, x| scan_value(r, x, path) | f),
        _ => false,
    }
}

fn finds(r: &CompiledRule, text: &str) -> bool {
    r.regex
        .find_iter(text)
        .any(|m| r.check.is_none_or(|c| c(m.as_str())))
}

fn hash(s: &str) -> String {
    let d = Sha256::digest(s.as_bytes());
    d[..8].iter().map(|b| format!("{b:02x}")).collect()
}

/// The text with what the rule finds replaced, if it replaces and finds.
fn redact(r: &CompiledRule, text: &str) -> Option<String> {
    if r.replacement.kind == "none" {
        return None;
    }
    let mut changed = false;
    let out = r.regex.replace_all(text, |c: &regex::Captures| {
        let m = c.get(0).map(|m| m.as_str()).unwrap_or("");
        if r.check.is_some_and(|f| !f(m)) {
            return m.to_string();
        }
        changed = true;
        replacement(&r.replacement, m)
    });
    changed.then(|| out.into_owned())
}

pub fn replacement(rep: &Replacement, m: &str) -> String {
    // Partial masks: a one-character replacement string, or `*`.
    let mask = rep
        .replacement_string
        .as_deref()
        .filter(|s| s.chars().count() == 1)
        .and_then(|s| s.chars().next())
        .unwrap_or('*');
    let n = rep.number_of_chars.unwrap_or(0);
    let chars: Vec<char> = m.chars().collect();
    match rep.kind.as_str() {
        "hash" => hash(m),
        "replacement_string" => rep.replacement_string.clone().unwrap_or_default(),
        "partial_replacement_from_beginning" => {
            let k = n.min(chars.len());
            std::iter::repeat_n(mask, k)
                .chain(chars[k..].iter().copied())
                .collect()
        }
        "partial_replacement_from_end" => {
            let k = n.min(chars.len());
            chars[..chars.len() - k]
                .iter()
                .copied()
                .chain(std::iter::repeat_n(mask, k))
                .collect()
        }
        _ => m.to_string(),
    }
}

// --- API: Datadog's JSON:API shapes ----------------------------------------

async fn load(st: &AppState) -> anyhow::Result<Config> {
    Ok(st
        .store
        .pg
        .doc(KIND, DOC_ID)
        .await?
        .and_then(|d| serde_json::from_value(d.body).ok())
        .unwrap_or_default())
}

async fn save(st: &AppState, mut cfg: Config) -> Result<i64, ApiError> {
    cfg.version += 1;
    st.store
        .pg
        .put_doc(
            KIND,
            DOC_ID,
            &serde_json::to_value(&cfg).map_err(anyhow::Error::from)?,
        )
        .await?;
    st.logconfig.sds.load(&cfg);
    Ok(cfg.version)
}

pub async fn reload(st: &AppState) -> anyhow::Result<()> {
    st.logconfig.sds.load(&load(st).await?);
    Ok(())
}

fn group_json(g: &Group) -> Value {
    json!({
        "id": g.id,
        "type": "sensitive_data_scanner_group",
        "attributes": {
            "name": g.name, "description": g.description, "filter": g.filter,
            "is_enabled": g.is_enabled, "product_list": g.product_list, "samplings": [],
        },
        "relationships": {
            "configuration": {"data": {"id": CONFIG_ID, "type": "sensitive_data_scanner_configuration"}},
            "rules": {"data": g.rules.iter().map(|r| json!({"id": r.id, "type": "sensitive_data_scanner_rule"})).collect::<Vec<_>>()},
        }
    })
}

fn rule_json(g: &Group, r: &Rule) -> Value {
    let mut rel = json!({"group": {"data": {"id": g.id, "type": "sensitive_data_scanner_group"}}});
    if let Some(sp) = &r.standard_pattern_id {
        rel["standard_pattern"] =
            json!({"data": {"id": sp, "type": "sensitive_data_scanner_standard_pattern"}});
    }
    json!({
        "id": r.id,
        "type": "sensitive_data_scanner_rule",
        "attributes": {
            "name": r.name, "description": r.description,
            "pattern": r.standard_pattern_id.as_deref().and_then(standard).map(|s| s.pattern).unwrap_or(&r.pattern),
            "namespaces": r.namespaces, "excluded_namespaces": r.excluded_namespaces,
            "tags": r.tags, "text_replacement": r.text_replacement,
            "is_enabled": r.is_enabled, "priority": r.priority,
        },
        "relationships": rel,
    })
}

fn meta(version: i64) -> Value {
    json!({"version": version, "count_limit": 100, "group_count_limit": 20,
        "is_pci_compliant": false, "has_highlight_enabled": true,
        "has_multi_pass_enabled": false, "is_float_sampling_rate_enabled": false})
}

/// `GET /api/v2/sensitive-data-scanner/config`.
pub async fn config(State(st): State<AppState>) -> ApiResult<Value> {
    let cfg = load(&st).await?;
    let mut included: Vec<Value> = cfg.groups.iter().map(group_json).collect();
    for g in &cfg.groups {
        included.extend(g.rules.iter().map(|r| rule_json(g, r)));
    }
    Ok(Json(json!({
        "data": {
            "id": CONFIG_ID,
            "type": "sensitive_data_scanner_configuration",
            "attributes": {},
            "relationships": {"groups": {"data": cfg.groups.iter().map(|g| json!({"id": g.id, "type": "sensitive_data_scanner_group"})).collect::<Vec<_>>()}},
        },
        "included": included,
        "meta": meta(cfg.version),
    })))
}

fn ids(v: &Value) -> Option<Vec<String>> {
    v.as_array().map(|a| {
        a.iter()
            .filter_map(|x| x["id"].as_str().map(String::from))
            .collect()
    })
}

/// Reorders a list by ids; those not named stay at the end, in their order.
fn reorder<T: Clone>(items: &[T], order: &[String], id: impl Fn(&T) -> &str) -> Vec<T> {
    let mut out: Vec<T> = order
        .iter()
        .filter_map(|o| items.iter().find(|x| id(x) == o).cloned())
        .collect();
    out.extend(
        items
            .iter()
            .filter(|x| !order.iter().any(|o| o == id(x)))
            .cloned(),
    );
    out
}

/// `PATCH /api/v2/sensitive-data-scanner/config`: the order of the groups.
pub async fn reorder_groups(State(st): State<AppState>, Json(b): Json<Value>) -> ApiResult<Value> {
    let mut cfg = load(&st).await?;
    let order = ids(&b["data"]["relationships"]["groups"]["data"]).unwrap_or_default();
    cfg.groups = reorder(&cfg.groups, &order, |g| &g.id);
    let v = save(&st, cfg).await?;
    Ok(Json(json!({"meta": meta(v)})))
}

fn group_attrs(g: &mut Group, a: &Value) -> Result<(), ApiError> {
    if let Some(n) = a["name"].as_str() {
        g.name = n.into();
    }
    if let Some(d) = a["description"].as_str() {
        g.description = d.into();
    }
    if a["filter"].is_object() {
        g.filter = a["filter"].clone();
    }
    if let Some(e) = a["is_enabled"].as_bool() {
        g.is_enabled = e;
    }
    if let Some(p) = a["product_list"].as_array() {
        g.product_list = p
            .iter()
            .filter_map(|x| x.as_str().map(String::from))
            .collect();
    }
    if g.name.trim().is_empty() {
        return Err(ApiError::BadQuery("the group has no name".into()));
    }
    crate::api::parse_query(g.query())?;
    Ok(())
}

/// `POST /api/v2/sensitive-data-scanner/config/groups`.
pub async fn create_group(State(st): State<AppState>, Json(b): Json<Value>) -> ApiResult<Value> {
    let mut cfg = load(&st).await?;
    if cfg.groups.len() >= 20 {
        return Err(ApiError::BadQuery("at most 20 groups".into()));
    }
    let mut g = Group {
        id: new_id(),
        name: String::new(),
        description: String::new(),
        filter: all(),
        is_enabled: true,
        product_list: logs(),
        rules: vec![],
    };
    group_attrs(&mut g, &b["data"]["attributes"])?;
    let out = group_json(&g);
    cfg.groups.push(g);
    let v = save(&st, cfg).await?;
    Ok(Json(json!({"data": out, "meta": meta(v)})))
}

/// `PATCH /api/v2/sensitive-data-scanner/config/groups/{id}`: its
/// attributes, and the order of its rules.
pub async fn update_group(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Json(b): Json<Value>,
) -> ApiResult<Value> {
    let mut cfg = load(&st).await?;
    let g = cfg
        .groups
        .iter_mut()
        .find(|g| g.id == id)
        .ok_or(ApiError::NotFound)?;
    group_attrs(g, &b["data"]["attributes"])?;
    if let Some(order) = ids(&b["data"]["relationships"]["rules"]["data"]) {
        g.rules = reorder(&g.rules, &order, |r| &r.id);
    }
    let v = save(&st, cfg).await?;
    Ok(Json(json!({"meta": meta(v)})))
}

/// `DELETE /api/v2/sensitive-data-scanner/config/groups/{id}`.
pub async fn delete_group(State(st): State<AppState>, Path(id): Path<String>) -> ApiResult<Value> {
    let mut cfg = load(&st).await?;
    let n = cfg.groups.len();
    cfg.groups.retain(|g| g.id != id);
    if cfg.groups.len() == n {
        return Err(ApiError::NotFound);
    }
    let v = save(&st, cfg).await?;
    Ok(Json(json!({"meta": meta(v)})))
}

fn rule_attrs(r: &mut Rule, a: &Value, rel: &Value) -> Result<(), ApiError> {
    let mut v = serde_json::to_value(&*r).map_err(anyhow::Error::from)?;
    if let Some(m) = a.as_object() {
        for (k, x) in m {
            v[k] = x.clone();
        }
    }
    if let Some(sp) = rel["standard_pattern"]["data"]["id"].as_str() {
        v["standard_pattern_id"] = json!(sp);
        v["pattern"] = json!("");
    } else if a
        .get("pattern")
        .is_some_and(|p| p.as_str().is_some_and(|p| !p.is_empty()))
    {
        v["standard_pattern_id"] = Value::Null;
    }
    *r = serde_json::from_value(v).map_err(|e| ApiError::BadQuery(e.to_string()))?;
    validate_rule(r).map_err(ApiError::BadQuery)
}

/// `POST /api/v2/sensitive-data-scanner/config/rules`.
pub async fn create_rule(State(st): State<AppState>, Json(b): Json<Value>) -> ApiResult<Value> {
    let mut cfg = load(&st).await?;
    let gid = b["data"]["relationships"]["group"]["data"]["id"]
        .as_str()
        .ok_or_else(|| ApiError::BadQuery("relationships.group: the rule's group".into()))?
        .to_string();
    let g = cfg
        .groups
        .iter_mut()
        .find(|g| g.id == gid)
        .ok_or(ApiError::NotFound)?;
    if g.rules.len() >= 50 {
        return Err(ApiError::BadQuery("at most 50 rules in a group".into()));
    }
    let mut r = Rule {
        id: new_id(),
        name: String::new(),
        description: String::new(),
        pattern: String::new(),
        standard_pattern_id: None,
        namespaces: vec![],
        excluded_namespaces: vec![],
        tags: vec![],
        text_replacement: Replacement {
            kind: "none".into(),
            ..Default::default()
        },
        is_enabled: true,
        priority: 1,
    };
    rule_attrs(
        &mut r,
        &b["data"]["attributes"],
        &b["data"]["relationships"],
    )?;
    g.rules.push(r.clone());
    let out = rule_json(g, &r);
    let v = save(&st, cfg).await?;
    Ok(Json(json!({"data": out, "meta": meta(v)})))
}

/// `PATCH /api/v2/sensitive-data-scanner/config/rules/{id}`.
pub async fn update_rule(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Json(b): Json<Value>,
) -> ApiResult<Value> {
    let mut cfg = load(&st).await?;
    let r = cfg
        .groups
        .iter_mut()
        .flat_map(|g| g.rules.iter_mut())
        .find(|r| r.id == id)
        .ok_or(ApiError::NotFound)?;
    rule_attrs(r, &b["data"]["attributes"], &b["data"]["relationships"])?;
    let v = save(&st, cfg).await?;
    Ok(Json(json!({"meta": meta(v)})))
}

/// `DELETE /api/v2/sensitive-data-scanner/config/rules/{id}`.
pub async fn delete_rule(State(st): State<AppState>, Path(id): Path<String>) -> ApiResult<Value> {
    let mut cfg = load(&st).await?;
    let mut found = false;
    for g in &mut cfg.groups {
        let n = g.rules.len();
        g.rules.retain(|r| r.id != id);
        found |= g.rules.len() != n;
    }
    if !found {
        return Err(ApiError::NotFound);
    }
    let v = save(&st, cfg).await?;
    Ok(Json(json!({"meta": meta(v)})))
}

/// `GET /api/v2/sensitive-data-scanner/config/standard-patterns`.
pub async fn standard_patterns() -> Json<Value> {
    Json(json!({"data": STANDARD.iter().map(|s| json!({
        "id": s.id,
        "type": "sensitive_data_scanner_standard_pattern",
        "attributes": {"name": s.name, "pattern": s.pattern, "description": s.description,
            "tags": [format!("sensitive_data:{}", s.id), format!("sensitive_data_category:{}", s.category)],
            "priority": 1},
    })).collect::<Vec<_>>()}))
}

#[derive(Deserialize)]
pub struct TestReq {
    #[serde(default)]
    pattern: String,
    #[serde(default)]
    standard_pattern_id: Option<String>,
    #[serde(default)]
    text_replacement: Option<Replacement>,
    text: String,
}

/// `POST /api/ui/sensitive-data-scanner/test`: what a rule finds in a sample.
pub async fn test(Json(t): Json<TestReq>) -> ApiResult<Value> {
    let rule = Rule {
        id: String::new(),
        name: "test".into(),
        description: String::new(),
        pattern: t.pattern,
        standard_pattern_id: t.standard_pattern_id.filter(|s| !s.is_empty()),
        namespaces: vec![],
        excluded_namespaces: vec![],
        tags: vec![],
        text_replacement: t.text_replacement.unwrap_or_default(),
        is_enabled: true,
        priority: 1,
    };
    let (regex, check, _) = rule_regex(&rule).map_err(ApiError::BadQuery)?;
    let c = CompiledRule {
        name: String::new(),
        regex,
        check,
        namespaces: vec![],
        excluded: vec![],
        tags: vec![],
        replacement: rule.text_replacement.clone(),
    };
    let found: Vec<Value> = c
        .regex
        .find_iter(&t.text)
        .filter(|m| c.check.is_none_or(|f| f(m.as_str())))
        .map(|m| json!({"start": m.start(), "end": m.end(), "text": m.as_str()}))
        .collect();
    let _ = &c.name;
    Ok(Json(json!({
        "matches": found,
        "result": redact(&c, &t.text).unwrap_or(t.text),
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(name: &str, sp: Option<&str>, pattern: &str, kind: &str) -> Rule {
        Rule {
            id: name.into(),
            name: name.into(),
            description: String::new(),
            pattern: pattern.into(),
            standard_pattern_id: sp.map(Into::into),
            namespaces: vec![],
            excluded_namespaces: vec![],
            tags: vec![],
            text_replacement: Replacement {
                kind: kind.into(),
                replacement_string: Some("[redacted]".into()),
                number_of_chars: Some(12),
            },
            is_enabled: true,
            priority: 1,
        }
    }

    #[test]
    fn checksums() {
        assert!(luhn("4111 1111 1111 1111"));
        assert!(!luhn("4111 1111 1111 1112"));
        assert!(iban("FR76 3000 6000 0112 3456 7890 189"));
        assert!(!iban("FR76 3000 6000 0112 3456 7890 188"));
    }

    #[test]
    fn scans_and_redacts() {
        let mut ns = rule("Emails", Some("email_address"), "", "replacement_string");
        ns.excluded_namespaces = vec!["usr".into()];
        let cfg = Config {
            version: 1,
            groups: vec![
                Group {
                    id: "g1".into(),
                    name: "PCI".into(),
                    description: String::new(),
                    filter: json!({"query": "service:shop"}),
                    is_enabled: true,
                    product_list: logs(),
                    rules: vec![
                        rule(
                            "Cards",
                            Some("credit_card"),
                            "",
                            "partial_replacement_from_beginning",
                        ),
                        ns,
                        rule("Order ids", None, r"ORD-\d+", "none"),
                        rule("Tokens", Some("json_web_token"), "", "hash"),
                    ],
                },
                Group {
                    id: "g2".into(),
                    name: "Off".into(),
                    description: String::new(),
                    filter: all(),
                    is_enabled: false,
                    product_list: logs(),
                    rules: vec![rule("All", None, ".+", "replacement_string")],
                },
            ],
        };
        let s = Scanner::default();
        s.load(&cfg);
        let mut log = LogRow {
            ts: 0,
            status: "info".into(),
            service: Some("shop".into()),
            host: None,
            source: None,
            message: "paid with 4111-1111-1111-1111 (not 4111-1111-1111-1112) by bob@example.com for ORD-12"
                .into(),
            tags: vec![],
            attributes: json!({"usr": {"email": "bob@example.com"}, "req": {"auth": "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxIn0.c2ln", "to": "ann@example.org"}})
                .as_object()
                .cloned()
                .unwrap(),
            trace_id: None,
            span_id: None,
        };
        let hits = s.scan(&mut log);
        assert_eq!(hits, vec!["Cards", "Emails", "Order ids", "Tokens"]);
        assert_eq!(
            log.message,
            "paid with ************11-1111 (not 4111-1111-1111-1112) by [redacted] for ORD-12"
        );
        // Excluded namespace: kept; others redacted.
        assert_eq!(log.attributes["usr"]["email"], "bob@example.com");
        assert_eq!(log.attributes["req"]["to"], "[redacted]");
        assert_eq!(log.attributes["req"]["auth"].as_str().unwrap().len(), 16);
        assert!(log.tags.contains(&"sensitive_data:cards".to_string()));
        assert!(
            log.tags
                .contains(&"sensitive_data_category:credit_card".to_string())
        );
        assert!(log.tags.contains(&"sensitive_data:order_ids".to_string()));
        // Another service: not in the group's filter.
        let mut other = log.clone();
        other.service = Some("api".into());
        other.message = "4111 1111 1111 1111".into();
        assert!(s.scan(&mut other).is_empty());
        assert!(validate_rule(&rule("x", None, "(", "none")).is_err());
        assert!(validate_rule(&rule("x", Some("nope"), "", "none")).is_err());
    }
}
