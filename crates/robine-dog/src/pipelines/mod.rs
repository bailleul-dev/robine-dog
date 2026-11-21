//! Log pipelines, as Datadog's Log Configuration has them: an ordered list
//! of pipelines, each with a filter query and processors run in order on
//! every matching log at ingestion (after the JSON and reserved-attribute
//! preprocessing). Definitions use the JSON of Datadog's API
//! (`/api/v1/logs/config/pipelines`), so they can be pasted from or to it.

pub mod grok;
pub mod useragent;

use crate::intake::logs::{LogRow, normalize_status, normalize_trace_id, parse_timestamp_ms};
use crate::query::matcher::{attribute, matches};
use crate::query::parser::{self, Expr};
use grok::{Grok, set_path};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct FilterDef {
    #[serde(default)]
    pub query: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PipelineDef {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(rename = "type", default = "pipeline_type")]
    pub typ: String,
    pub name: String,
    #[serde(default = "yes")]
    pub is_enabled: bool,
    #[serde(default)]
    pub filter: FilterDef,
    #[serde(default)]
    pub processors: Vec<ProcessorDef>,
    #[serde(default)]
    pub is_read_only: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

fn pipeline_type() -> String {
    "pipeline".into()
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProcessorDef {
    #[serde(default)]
    pub name: String,
    #[serde(default = "yes")]
    pub is_enabled: bool,
    #[serde(flatten)]
    pub kind: Kind,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct GrokRules {
    #[serde(default)]
    pub support_rules: String,
    #[serde(default)]
    pub match_rules: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Category {
    pub filter: FilterDef,
    pub name: String,
}

fn message() -> String {
    "message".into()
}

fn attribute_type() -> String {
    "attribute".into()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum Kind {
    GrokParser {
        #[serde(default = "message")]
        source: String,
        #[serde(default)]
        samples: Vec<String>,
        grok: GrokRules,
    },
    DateRemapper {
        sources: Vec<String>,
    },
    StatusRemapper {
        sources: Vec<String>,
    },
    ServiceRemapper {
        sources: Vec<String>,
    },
    MessageRemapper {
        sources: Vec<String>,
    },
    AttributeRemapper {
        sources: Vec<String>,
        #[serde(default = "attribute_type")]
        source_type: String,
        target: String,
        #[serde(default = "attribute_type")]
        target_type: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target_format: Option<String>,
        #[serde(default)]
        preserve_source: bool,
        #[serde(default)]
        override_on_conflict: bool,
    },
    UrlParser {
        sources: Vec<String>,
        target: String,
        #[serde(default = "yes")]
        normalize_ending_slashes: bool,
    },
    UserAgentParser {
        sources: Vec<String>,
        target: String,
        #[serde(default)]
        is_encoded: bool,
    },
    CategoryProcessor {
        target: String,
        categories: Vec<Category>,
    },
    ArithmeticProcessor {
        expression: String,
        target: String,
        #[serde(default)]
        is_replace_missing: bool,
    },
    StringBuilderProcessor {
        template: String,
        target: String,
        #[serde(default)]
        is_replace_missing: bool,
    },
    LookupProcessor {
        source: String,
        target: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        lookup_table: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        default_lookup: Option<String>,
        /// A Reference Table: the target gets the row of the source's key.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        lookup_enrichment_table: Option<String>,
    },
    TraceIdRemapper {
        sources: Vec<String>,
    },
    SpanIdRemapper {
        sources: Vec<String>,
    },
    /// A nested pipeline (one level, as in Datadog).
    Pipeline {
        #[serde(default)]
        filter: FilterDef,
        #[serde(default)]
        processors: Vec<ProcessorDef>,
    },
}

// --- compiled form -----------------------------------------------------------

enum Step {
    Grok(String, Grok),
    Date(Vec<String>),
    Status(Vec<String>),
    Service(Vec<String>),
    Message(Vec<String>),
    Remap {
        sources: Vec<String>,
        from_tag: bool,
        target: String,
        to_tag: bool,
        format: Option<String>,
        preserve: bool,
        overwrite: bool,
    },
    Url(Vec<String>, String, bool),
    UserAgent(Vec<String>, String, bool),
    Category(String, Vec<(Expr, String)>),
    Arithmetic(Arith, String, bool),
    Builder(Vec<Part>, String, bool),
    Lookup(String, String, HashMap<String, String>, Option<String>),
    /// Source, target and Reference Table.
    RefLookup(String, String, String),
    TraceId(Vec<String>),
    SpanId(Vec<String>),
    Nested(Compiled),
}

struct Compiled {
    filter: Expr,
    steps: Vec<Step>,
}

fn query(q: &str) -> Result<Expr, String> {
    parser::parse(q).map_err(|e| format!("filter `{q}`: {}", e.0))
}

fn compile_steps(ps: &[ProcessorDef], nested: bool) -> Result<Vec<Step>, String> {
    let mut out = Vec::new();
    for p in ps.iter().filter(|p| p.is_enabled) {
        let label = if p.name.is_empty() {
            "processor"
        } else {
            &p.name
        };
        let step = match &p.kind {
            Kind::GrokParser { source, grok, .. } => Step::Grok(
                source.clone(),
                Grok::new(&grok.match_rules, &grok.support_rules)
                    .map_err(|e| format!("{label}: {e}"))?,
            ),
            Kind::DateRemapper { sources } => Step::Date(sources.clone()),
            Kind::StatusRemapper { sources } => Step::Status(sources.clone()),
            Kind::ServiceRemapper { sources } => Step::Service(sources.clone()),
            Kind::MessageRemapper { sources } => Step::Message(sources.clone()),
            Kind::AttributeRemapper {
                sources,
                source_type,
                target,
                target_type,
                target_format,
                preserve_source,
                override_on_conflict,
            } => Step::Remap {
                sources: sources.clone(),
                from_tag: source_type == "tag",
                target: target.clone(),
                to_tag: target_type == "tag",
                format: target_format.clone().filter(|f| f != "auto"),
                preserve: *preserve_source,
                overwrite: *override_on_conflict,
            },
            Kind::UrlParser {
                sources,
                target,
                normalize_ending_slashes,
            } => Step::Url(sources.clone(), target.clone(), *normalize_ending_slashes),
            Kind::UserAgentParser {
                sources,
                target,
                is_encoded,
            } => Step::UserAgent(sources.clone(), target.clone(), *is_encoded),
            Kind::CategoryProcessor { target, categories } => Step::Category(
                target.clone(),
                categories
                    .iter()
                    .map(|c| Ok((query(&c.filter.query)?, c.name.clone())))
                    .collect::<Result<_, String>>()
                    .map_err(|e| format!("{label}: {e}"))?,
            ),
            Kind::ArithmeticProcessor {
                expression,
                target,
                is_replace_missing,
            } => Step::Arithmetic(
                Arith::parse(expression).map_err(|e| format!("{label}: {e}"))?,
                target.clone(),
                *is_replace_missing,
            ),
            Kind::StringBuilderProcessor {
                template,
                target,
                is_replace_missing,
            } => Step::Builder(
                template_parts(template),
                target.clone(),
                *is_replace_missing,
            ),
            Kind::LookupProcessor {
                source,
                target,
                lookup_enrichment_table: Some(table),
                ..
            } => Step::RefLookup(source.clone(), target.clone(), table.clone()),
            Kind::LookupProcessor {
                source,
                target,
                lookup_table,
                default_lookup,
                lookup_enrichment_table: None,
            } => Step::Lookup(
                source.clone(),
                target.clone(),
                lookup_table
                    .iter()
                    .filter_map(|l| {
                        let (k, v) = l.split_once(',')?;
                        Some((k.trim().to_string(), v.trim().to_string()))
                    })
                    .collect(),
                default_lookup.clone(),
            ),
            Kind::TraceIdRemapper { sources } => Step::TraceId(sources.clone()),
            Kind::SpanIdRemapper { sources } => Step::SpanId(sources.clone()),
            Kind::Pipeline { filter, processors } => {
                if nested {
                    return Err("pipelines nest one level only".into());
                }
                Step::Nested(Compiled {
                    filter: query(&filter.query).map_err(|e| format!("{label}: {e}"))?,
                    steps: compile_steps(processors, true)?,
                })
            }
        };
        out.push(step);
    }
    Ok(out)
}

/// Checks a definition: its filter and every processor must compile.
pub fn validate(p: &PipelineDef) -> Result<(), String> {
    query(&p.filter.query)?;
    compile_steps(&p.processors, false).map(|_| ())
}

// --- applying --------------------------------------------------------------

/// The value of a source: reserved fields by name, otherwise an attribute.
fn read(log: &LogRow, path: &str) -> Option<Value> {
    match path {
        "message" => Some(json!(log.message)),
        "status" if !log.attributes.contains_key("status") => Some(json!(log.status)),
        "service" if !log.attributes.contains_key("service") => {
            log.service.as_ref().map(|s| json!(s))
        }
        "host" if !log.attributes.contains_key("host") => log.host.as_ref().map(|s| json!(s)),
        p => attribute(&log.attributes, p)
            .filter(|v| !v.is_null())
            .cloned(),
    }
}

fn remove_path(m: &mut Map<String, Value>, path: &str) -> bool {
    if m.remove(path).is_some() {
        return true;
    }
    let Some((head, rest)) = path.split_once('.') else {
        return false;
    };
    let Some(Value::Object(inner)) = m.get_mut(head) else {
        return false;
    };
    let removed = remove_path(inner, rest);
    if removed && inner.is_empty() {
        m.remove(head);
    }
    removed
}

fn first(log: &LogRow, sources: &[String]) -> Option<(String, Value)> {
    sources
        .iter()
        .find_map(|s| read(log, s).map(|v| (s.clone(), v)))
}

fn text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn tag_value(log: &LogRow, key: &str) -> Option<String> {
    log.tags.iter().find_map(|t| {
        let (k, v) = t.split_once(':')?;
        (k == key).then(|| v.to_string())
    })
}

fn format_as(v: Value, f: Option<&str>) -> Option<Value> {
    match f {
        Some("integer") => text(&v).trim().parse::<f64>().ok().map(|n| json!(n as i64)),
        Some("double") => text(&v).trim().parse::<f64>().ok().map(|n| json!(n)),
        Some("string") => Some(json!(text(&v))),
        _ => Some(v),
    }
}

impl Step {
    fn apply(&self, log: &mut LogRow) {
        match self {
            Step::Grok(source, g) => {
                let Some(t) = read(log, source) else { return };
                if let Some(out) = g.parse(&text(&t)) {
                    for (k, v) in out.attributes {
                        merge(&mut log.attributes, k, v);
                    }
                }
            }
            Step::Date(sources) => {
                if let Some(ts) = sources
                    .iter()
                    .find_map(|s| read(log, s).and_then(|v| parse_timestamp_ms(&v)))
                {
                    log.ts = ts;
                }
            }
            Step::Status(sources) => {
                if let Some((_, v)) = first(log, sources) {
                    log.status = normalize_status(&text(&v)).to_string();
                }
            }
            Step::Service(sources) => {
                if let Some((_, v)) = first(log, sources) {
                    log.service = Some(text(&v));
                }
            }
            Step::Message(sources) => {
                if let Some((_, v)) = first(log, sources) {
                    log.message = text(&v);
                }
            }
            Step::Remap {
                sources,
                from_tag,
                target,
                to_tag,
                format,
                preserve,
                overwrite,
            } => {
                let found = if *from_tag {
                    sources
                        .iter()
                        .find_map(|s| tag_value(log, s).map(|v| (s.clone(), json!(v))))
                } else {
                    first(log, sources)
                };
                let Some((source, v)) = found else { return };
                let Some(v) = format_as(v, format.as_deref()) else {
                    return;
                };
                if *to_tag {
                    let prefix = format!("{target}:");
                    if *overwrite {
                        log.tags.retain(|t| !t.starts_with(&prefix));
                    }
                    if *overwrite || !log.tags.iter().any(|t| t.starts_with(&prefix)) {
                        log.tags.push(format!("{target}:{}", text(&v)));
                    }
                } else {
                    if attribute(&log.attributes, target).is_some() && !overwrite {
                        return;
                    }
                    set_path(&mut log.attributes, target, v);
                }
                if !preserve && source != *target {
                    if *from_tag {
                        let prefix = format!("{source}:");
                        log.tags.retain(|t| !t.starts_with(&prefix));
                    } else {
                        remove_path(&mut log.attributes, &source);
                    }
                }
            }
            Step::Url(sources, target, normalize) => {
                let Some((_, v)) = first(log, sources) else {
                    return;
                };
                if let Some(mut d) = grok::url_details(&text(&v)) {
                    if *normalize
                        && let Some(Value::String(p)) = d.get_mut("path")
                        && p.len() > 1
                        && p.ends_with('/')
                    {
                        p.pop();
                    }
                    set_path(&mut log.attributes, target, d);
                }
            }
            Step::UserAgent(sources, target, encoded) => {
                let Some((_, v)) = first(log, sources) else {
                    return;
                };
                let mut ua = text(&v);
                if *encoded {
                    ua = grok::decode_uri(&ua);
                }
                set_path(&mut log.attributes, target, useragent::parse(&ua));
            }
            Step::Category(target, cats) => {
                if let Some((_, name)) = cats.iter().find(|(q, _)| matches(q, log)) {
                    set_path(&mut log.attributes, target, json!(name));
                }
            }
            Step::Arithmetic(expr, target, replace) => {
                if let Some(v) = expr.eval(log, *replace)
                    && v.is_finite()
                {
                    let v = if v.fract() == 0.0 && v.abs() < 9e15 {
                        json!(v as i64)
                    } else {
                        json!(v)
                    };
                    set_path(&mut log.attributes, target, v);
                }
            }
            Step::Builder(parts, target, replace) => {
                let mut out = String::new();
                for p in parts {
                    match p {
                        Part::Text(t) => out.push_str(t),
                        Part::Attr(a) => match read(log, a) {
                            Some(v) => out.push_str(&text(&v)),
                            None if *replace => {}
                            None => return,
                        },
                    }
                }
                set_path(&mut log.attributes, target, json!(out));
            }
            Step::Lookup(source, target, table, default) => {
                let Some(v) = read(log, source) else { return };
                if let Some(x) = table.get(&text(&v)).or(default.as_ref()) {
                    set_path(&mut log.attributes, target, json!(x));
                }
            }
            Step::RefLookup(source, target, table) => {
                let Some(v) = read(log, source) else { return };
                if let Some(row) = crate::logconfig::reftables::lookup(table, &text(&v)) {
                    set_path(&mut log.attributes, target, Value::Object(row));
                }
            }
            Step::TraceId(sources) => {
                if let Some(id) = sources
                    .iter()
                    .find_map(|s| read(log, s).and_then(|v| normalize_trace_id(&v)))
                {
                    log.trace_id = Some(id);
                }
            }
            Step::SpanId(sources) => {
                if let Some(id) = sources
                    .iter()
                    .find_map(|s| read(log, s).and_then(|v| normalize_trace_id(&v)))
                {
                    log.span_id = Some(id);
                }
            }
            Step::Nested(p) => p.apply(log),
        }
    }
}

/// Grok output merges into existing objects instead of replacing them.
fn merge(m: &mut Map<String, Value>, k: String, v: Value) {
    match (m.get_mut(&k), v) {
        (Some(Value::Object(existing)), Value::Object(new)) => {
            for (k2, v2) in new {
                merge(existing, k2, v2);
            }
        }
        (_, v) => {
            m.insert(k, v);
        }
    }
}

impl Compiled {
    fn apply(&self, log: &mut LogRow) {
        if matches(&self.filter, log) {
            for s in &self.steps {
                s.apply(log);
            }
        }
    }
}

// --- arithmetic --------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum Arith {
    Num(f64),
    Attr(String),
    Neg(Box<Arith>),
    Bin(char, Box<Arith>, Box<Arith>),
    Call(String, Box<Arith>),
}

impl Arith {
    pub fn parse(s: &str) -> Result<Arith, String> {
        let toks = Self::lex(s)?;
        let mut i = 0;
        let e = Self::sum(&toks, &mut i)?;
        if i != toks.len() {
            return Err(format!("unexpected `{}`", toks[i]));
        }
        Ok(e)
    }

    fn lex(s: &str) -> Result<Vec<String>, String> {
        let mut out = Vec::new();
        let chars: Vec<char> = s.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];
            if c.is_whitespace() {
                i += 1;
            } else if "+-*/%()".contains(c) {
                out.push(c.to_string());
                i += 1;
            } else if c.is_ascii_digit() || c == '.' {
                let st = i;
                while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                    i += 1;
                }
                out.push(chars[st..i].iter().collect());
            } else if c.is_alphanumeric() || c == '_' || c == '@' {
                let st = i;
                while i < chars.len()
                    && (chars[i].is_alphanumeric() || matches!(chars[i], '_' | '.' | '@' | '-'))
                {
                    i += 1;
                }
                out.push(chars[st..i].iter().collect());
            } else {
                return Err(format!("unexpected `{c}`"));
            }
        }
        Ok(out)
    }

    fn sum(t: &[String], i: &mut usize) -> Result<Arith, String> {
        let mut l = Self::product(t, i)?;
        while *i < t.len() && (t[*i] == "+" || t[*i] == "-") {
            let op = t[*i].chars().next().unwrap();
            *i += 1;
            l = Arith::Bin(op, Box::new(l), Box::new(Self::product(t, i)?));
        }
        Ok(l)
    }

    fn product(t: &[String], i: &mut usize) -> Result<Arith, String> {
        let mut l = Self::unary(t, i)?;
        while *i < t.len() && matches!(t[*i].as_str(), "*" | "/" | "%") {
            let op = t[*i].chars().next().unwrap();
            *i += 1;
            l = Arith::Bin(op, Box::new(l), Box::new(Self::unary(t, i)?));
        }
        Ok(l)
    }

    fn unary(t: &[String], i: &mut usize) -> Result<Arith, String> {
        let tok = t.get(*i).ok_or("unexpected end of expression")?.clone();
        *i += 1;
        match tok.as_str() {
            "-" => Ok(Arith::Neg(Box::new(Self::unary(t, i)?))),
            "(" => {
                let e = Self::sum(t, i)?;
                if t.get(*i).map(String::as_str) != Some(")") {
                    return Err("missing `)`".into());
                }
                *i += 1;
                Ok(e)
            }
            f @ ("abs" | "ceil" | "floor" | "round")
                if t.get(*i).map(String::as_str) == Some("(") =>
            {
                *i += 1;
                let e = Self::sum(t, i)?;
                if t.get(*i).map(String::as_str) != Some(")") {
                    return Err("missing `)`".into());
                }
                *i += 1;
                Ok(Arith::Call(f.to_string(), Box::new(e)))
            }
            n if n
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_digit() || c == '.') =>
            {
                n.parse()
                    .map(Arith::Num)
                    .map_err(|_| format!("bad number `{n}`"))
            }
            a if !"+*/%)".contains(a) => Ok(Arith::Attr(a.trim_start_matches('@').to_string())),
            other => Err(format!("unexpected `{other}`")),
        }
    }

    fn eval(&self, log: &LogRow, replace_missing: bool) -> Option<f64> {
        Some(match self {
            Arith::Num(n) => *n,
            Arith::Attr(a) => {
                match read(log, a).and_then(|v| text(&v).trim().parse::<f64>().ok()) {
                    Some(v) => v,
                    None if replace_missing => 0.0,
                    None => return None,
                }
            }
            Arith::Neg(e) => -e.eval(log, replace_missing)?,
            Arith::Bin(op, l, r) => {
                let (l, r) = (l.eval(log, replace_missing)?, r.eval(log, replace_missing)?);
                match op {
                    '+' => l + r,
                    '-' => l - r,
                    '*' => l * r,
                    '/' => l / r,
                    _ => l % r,
                }
            }
            Arith::Call(f, e) => {
                let v = e.eval(log, replace_missing)?;
                match f.as_str() {
                    "abs" => v.abs(),
                    "ceil" => v.ceil(),
                    "floor" => v.floor(),
                    _ => v.round(),
                }
            }
        })
    }
}

#[derive(Debug, Clone)]
pub enum Part {
    Text(String),
    Attr(String),
}

fn template_parts(t: &str) -> Vec<Part> {
    let mut out = Vec::new();
    let mut rest = t;
    while let Some(i) = rest.find("%{") {
        if i > 0 {
            out.push(Part::Text(rest[..i].to_string()));
        }
        match rest[i..].find('}') {
            Some(j) => {
                out.push(Part::Attr(rest[i + 2..i + j].trim().to_string()));
                rest = &rest[i + j + 1..];
            }
            None => {
                out.push(Part::Text(rest[i..].to_string()));
                rest = "";
            }
        }
    }
    if !rest.is_empty() {
        out.push(Part::Text(rest.to_string()));
    }
    out
}

// --- the set in use ------------------------------------------------------------

/// The enabled pipelines, compiled, swapped atomically on every change.
#[derive(Default)]
pub struct Pipelines {
    compiled: RwLock<Arc<Vec<Compiled>>>,
}

impl Pipelines {
    pub fn load(&self, defs: &[PipelineDef]) {
        let mut out = Vec::new();
        for d in defs.iter().filter(|d| d.is_enabled) {
            let compiled =
                query(&d.filter.query).and_then(|f| Ok((f, compile_steps(&d.processors, false)?)));
            match compiled {
                Ok((filter, steps)) => out.push(Compiled { filter, steps }),
                Err(e) => tracing::warn!(pipeline = %d.name, error = %e, "pipeline skipped"),
            }
        }
        *self.compiled.write().unwrap_or_else(|e| e.into_inner()) = Arc::new(out);
    }

    pub fn apply(&self, log: &mut LogRow) {
        let set = self
            .compiled
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        for p in set.iter() {
            p.apply(log);
        }
    }
}

/// What a set of definitions does to one log.
#[cfg(test)]
pub fn preview(defs: &[PipelineDef], log: &mut LogRow) {
    let p = Pipelines::default();
    p.load(defs);
    p.apply(log);
}

// --- integration pipelines ---------------------------------------------------------

fn processor(name: &str, kind: Kind) -> ProcessorDef {
    ProcessorDef {
        name: name.into(),
        is_enabled: true,
        kind,
    }
}

fn s(v: &[&str]) -> Vec<String> {
    v.iter().map(|x| x.to_string()).collect()
}

fn remap(source: &str, target: &str) -> ProcessorDef {
    processor(
        &format!("Map `{source}` to `{target}`"),
        Kind::AttributeRemapper {
            sources: s(&[source]),
            source_type: attribute_type(),
            target: target.into(),
            target_type: attribute_type(),
            target_format: None,
            preserve_source: false,
            override_on_conflict: false,
        },
    )
}

fn status_category() -> ProcessorDef {
    processor(
        "Categorise status code",
        Kind::CategoryProcessor {
            target: "http.status_category".into(),
            categories: [
                ("@http.status_code:[200 TO 299]", "OK"),
                ("@http.status_code:[300 TO 399]", "notice"),
                ("@http.status_code:[400 TO 499]", "warning"),
                ("@http.status_code:[500 TO 599]", "error"),
            ]
            .iter()
            .map(|(q, n)| Category {
                filter: FilterDef {
                    query: q.to_string(),
                },
                name: n.to_string(),
            })
            .collect(),
        },
    )
}

/// Read-only pipelines installed the first time, like Datadog's
/// integration pipelines (clone one to change it).
pub fn integration_pipelines() -> Vec<PipelineDef> {
    let pipeline = |name: &str, query: &str, processors: Vec<ProcessorDef>| PipelineDef {
        id: None,
        typ: "pipeline".into(),
        name: name.into(),
        is_enabled: true,
        filter: FilterDef {
            query: query.into(),
        },
        processors,
        is_read_only: true,
        description: None,
    };
    vec![
        pipeline(
            "Nginx",
            "source:nginx",
            vec![
                processor(
                    "Parsing Nginx logs",
                    Kind::GrokParser {
                        source: "message".into(),
                        samples: s(&[
                            r#"10.0.3.7 - - [16/Nov/2026:16:31:00 +0000] "GET /api/products?limit=24 HTTP/1.1" 200 48211 "-" "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36""#,
                            "2026/11/16 16:31:00 [error] 31#31: *4210 connect() failed (111: Connection refused) while connecting to upstream, client: 10.0.3.7, server: shop, request: \"GET /api/cart HTTP/1.1\", upstream: \"http://10.42.0.17:8080/api/cart\"",
                        ]),
                        grok: GrokRules {
                            support_rules: [
                                r#"_auth %{notSpace:http.auth:nullIf("-")}"#,
                                r#"_bytes_written %{integer:network.bytes_written}"#,
                                r#"_client_ip %{ipOrHost:network.client.ip}"#,
                                r#"_version HTTP\/%{regex("\\d+\\.\\d+"):http.version}"#,
                                r#"_url %{notSpace:http.url}"#,
                                r#"_ident %{notSpace:http.ident:nullIf("-")}"#,
                                r#"_user_agent %{regex("[^\\\"]*"):http.useragent}"#,
                                r#"_referer %{notSpace:http.referer}"#,
                                r#"_status_code %{integer:http.status_code}"#,
                                r#"_method %{word:http.method}"#,
                                r#"_date_access %{date("dd/MMM/yyyy:HH:mm:ss Z"):date_access}"#,
                                r#"_date_error %{date("yyyy/MM/dd HH:mm:ss"):date_access}"#,
                            ]
                            .join("\n"),
                            match_rules: [
                                r#"access.common %{_client_ip} %{_ident} %{_auth} \[%{_date_access}\] "(?:%{_method} |)%{_url}(?: %{_version}|)" %{_status_code} (?:%{_bytes_written}|-)"#,
                                r#"access.combined %{access.common} "%{_referer}" "%{_user_agent}".*"#,
                                r#"error.format %{_date_error} \[%{word:level}\] %{data:error.message}(?:, %{data::keyvalue(": ", "/:")})?"#,
                            ]
                            .join("\n"),
                        },
                    },
                ),
                processor("Define `date_access` as the official date of the log", Kind::DateRemapper { sources: s(&["date_access"]) }),
                status_category(),
                processor("Define `http.status_category`, `level` as the official status of the log", Kind::StatusRemapper { sources: s(&["http.status_category", "level"]) }),
                processor("URL parser", Kind::UrlParser { sources: s(&["http.url"]), target: "http.url_details".into(), normalize_ending_slashes: false }),
                processor("User-Agent parser", Kind::UserAgentParser { sources: s(&["http.useragent"]), target: "http.useragent_details".into(), is_encoded: false }),
            ],
        ),
        pipeline(
            "PostgreSQL",
            "source:postgresql",
            vec![
                processor(
                    "Parsing PostgreSQL logs",
                    Kind::GrokParser {
                        source: "message".into(),
                        samples: s(&[
                            r#"2026-11-17 10:14:23.259 UTC [78] ERROR:  relation "schema_version" does not exist at character 21"#,
                            "2026-11-17 10:14:15.027 UTC [1] LOG:  duration: 12.345 ms  statement: SELECT count(*) FROM orders",
                            "FATAL:  remaining connection slots are reserved for non-replication superuser connections",
                        ]),
                        grok: GrokRules {
                            // `log_line_prefix`: the image's default (`%m [%p] `), or none.
                            support_rules: [
                                r#"_prefix (?:%{regex("\\d{4}-\\d{2}-\\d{2} \\d{2}:\\d{2}:\\d{2}(?:\\.\\d+)? \\S+"):db.timestamp} \[%{integer:db.pid}\]\s+)?"#,
                                r#"_severity %{regex("[A-Z]+[0-9]?"):db.severity}:\s+"#,
                            ]
                            .join("\n"),
                            match_rules: [
                                r#"postgresql.duration %{_prefix}(?:%{_severity})?duration: %{number:db.duration_ms} ms\s+(?:statement|parse \S+|bind \S+|execute \S+): %{data:db.statement}"#,
                                r#"postgresql.severity %{_prefix}%{_severity}%{data:db.message}"#,
                            ]
                            .join("\n"),
                        },
                    },
                ),
                processor("Duration in nanoseconds", Kind::ArithmeticProcessor { expression: "db.duration_ms * 1000000".into(), target: "duration".into(), is_replace_missing: false }),
                // PostgreSQL writes everything to stderr; LOG, STATEMENT, DETAIL… are not errors.
                processor(
                    "Map `db.severity` to a status",
                    Kind::LookupProcessor {
                        source: "db.severity".into(),
                        target: "level".into(),
                        lookup_table: s(&[
                            "LOG,info", "INFO,info", "STATEMENT,info", "DETAIL,info", "HINT,info",
                            "CONTEXT,info", "QUERY,info", "LOCATION,info", "NOTICE,notice",
                            "WARNING,warn", "ERROR,error", "FATAL,critical", "PANIC,emergency",
                        ]),
                        default_lookup: Some("debug".into()),
                        lookup_enrichment_table: None,
                    },
                ),
                processor("Define `level` as the official status of the log", Kind::StatusRemapper { sources: s(&["level"]) }),
            ],
        ),
        pipeline(
            "Kubernetes",
            "source:kubernetes",
            vec![processor(
                "Parsing kubelet key=value messages",
                Kind::GrokParser {
                    source: "message".into(),
                    samples: s(&[r#"Probe succeeded probeType="Readiness" pod="shop/shop-api-5c6b4-h7r2m""#]),
                    grok: GrokRules {
                        support_rules: String::new(),
                        match_rules: r#"kubelet.keyvalue %{regex("[A-Za-z ]+"):kube.event} %{data::keyvalue}"#.into(),
                    },
                },
            )],
        ),
        pipeline(
            "Traefik",
            "source:traefik",
            vec![
                // JSON access logs (`--accesslog.format=json`): Traefik's field names to the standard attributes.
                remap("RequestMethod", "http.method"),
                remap("DownstreamStatus", "http.status_code"),
                remap("RequestPath", "http.url"),
                remap("RequestHost", "http.host"),
                remap("ClientHost", "network.client.ip"),
                remap("request_User-Agent", "http.useragent"),
                remap("request_Referer", "http.referer"),
                remap("DownstreamContentSize", "network.bytes_written"),
                remap("Duration", "duration"),
                // Set when tracing is on: the access log opens its trace.
                processor("Define `TraceId` as the official trace id of the log", Kind::TraceIdRemapper { sources: s(&["TraceId"]) }),
                processor("Define `SpanId` as the official span id of the log", Kind::SpanIdRemapper { sources: s(&["SpanId"]) }),
                status_category(),
                processor("Define `http.status_category` as the official status of the log", Kind::StatusRemapper { sources: s(&["http.status_category"]) }),
                processor("Access log message", Kind::StringBuilderProcessor { template: "%{http.method} %{http.host}%{http.url} %{http.status_code}".into(), target: "traefik.request".into(), is_replace_missing: false }),
                processor("Define `traefik.request` as the official message of the log", Kind::MessageRemapper { sources: s(&["traefik.request"]) }),
                processor("URL parser", Kind::UrlParser { sources: s(&["http.url"]), target: "http.url_details".into(), normalize_ending_slashes: false }),
                processor("User-Agent parser", Kind::UserAgentParser { sources: s(&["http.useragent"]), target: "http.useragent_details".into(), is_encoded: false }),
            ],
        ),
        pipeline(
            "CoreDNS",
            "source:coredns",
            vec![
                processor(
                    "Parsing CoreDNS logs",
                    Kind::GrokParser {
                        source: "message".into(),
                        samples: s(&[
                            r#"[INFO] 10.42.0.9:40212 - 5012 "A IN id.bailleul.dev. udp 33 false 512" NOERROR qr,rd,ra 64 0.004211s"#,
                            "[WARNING] No files matching import glob pattern: /etc/coredns/custom/*.server",
                        ]),
                        grok: GrokRules {
                            support_rules: String::new(),
                            match_rules: [
                                r#"coredns.query \[%{word:level}\] %{ip:network.client.ip}:%{port:network.client.port} - %{integer:dns.id} "%{word:dns.question.type} %{word:dns.question.class} %{notSpace:dns.question.name} %{word:network.transport} %{integer:dns.question.size} %{word:dns.do} %{integer:dns.bufsize}" %{word:dns.response_code} %{notSpace:dns.flags} %{integer:network.bytes_written} %{number:dns.duration_s}s"#,
                                r#"coredns.level \[%{word:level}\] %{data}"#,
                            ]
                            .join("\n"),
                        },
                    },
                ),
                processor("Duration in nanoseconds", Kind::ArithmeticProcessor { expression: "dns.duration_s * 1000000000".into(), target: "duration".into(), is_replace_missing: false }),
                processor("Define `level` as the official status of the log", Kind::StatusRemapper { sources: s(&["level"]) }),
            ],
        ),
        pipeline(
            "Redis",
            "source:redis",
            vec![
                processor(
                    "Parsing Redis logs",
                    Kind::GrokParser {
                        source: "message".into(),
                        samples: s(&["1:M 17 Nov 2026 10:14:15.027 * Ready to accept connections tcp"]),
                        grok: GrokRules {
                            support_rules: String::new(),
                            match_rules: r#"redis.log %{integer:pid}:%{regex("[A-Z]"):redis.role} %{regex("\\d{1,2} \\w+ \\d{4} [\\d:.]+"):redis.timestamp} %{regex("[.*#-]"):redis.severity} %{data}"#.into(),
                        },
                    },
                ),
                processor(
                    "Map `redis.severity` to a status",
                    Kind::LookupProcessor {
                        source: "redis.severity".into(),
                        target: "level".into(),
                        lookup_table: s(&[".,debug", "-,info", "*,notice", "#,warn"]),
                        default_lookup: None,
                        lookup_enrichment_table: None,
                    },
                ),
                processor("Define `level` as the official status of the log", Kind::StatusRemapper { sources: s(&["level"]) }),
            ],
        ),
        pipeline(
            "Datadog Agent",
            "source:agent",
            vec![
                processor(
                    "Parsing Datadog Agent logs",
                    Kind::GrokParser {
                        source: "message".into(),
                        samples: s(&["2026-11-17 10:15:26 UTC | CORE | INFO | (pkg/collector/worker/check_logger.go:59 in CheckFinished) | check:kubelet | Done running check"]),
                        grok: GrokRules {
                            support_rules: String::new(),
                            match_rules: r#"agent.log %{regex("\\d{4}-\\d{2}-\\d{2} \\d{2}:\\d{2}:\\d{2} \\S+"):agent.timestamp} \| %{notSpace:agent.component} \| %{word:level} \| \(%{data:logger.name}\) \| %{data:agent.message}"#.into(),
                        },
                    },
                ),
                processor("Define `level` as the official status of the log", Kind::StatusRemapper { sources: s(&["level"]) }),
                processor("Define `agent.message` as the official message of the log", Kind::MessageRemapper { sources: s(&["agent.message"]) }),
            ],
        ),
        // Not tied to a source: what Go programs commonly write (logrus and
        // slog text, klog), whatever the application. A line of another
        // shape matches no rule and is left as it is.
        pipeline(
            "Go (logfmt, klog)",
            "",
            vec![
                processor(
                    "Parsing logfmt and klog lines",
                    Kind::GrokParser {
                        source: "message".into(),
                        samples: s(&[
                            r#"level=INFO msg="Created new admin user" username=guillaume user_id=1"#,
                            r#"time="2026-11-17T10:13:59Z" level=info msg="Volume pvc-7fc64f33 has been created""#,
                            r#"I1117 10:13:59.888380       1 event.go:389] "Event occurred" object="miniflux/data-miniflux-postgres-0""#,
                        ]),
                        grok: GrokRules {
                            support_rules: String::new(),
                            match_rules: [
                                r#"logfmt %{regex("(?:time|ts|t|level|lvl)=\\S.*"):logfmt:keyvalue("=")}"#,
                                r#"klog %{regex("[IWEF]"):klog.severity}%{regex("\\d{4} [\\d:.]+"):klog.timestamp}\s+%{integer:pid} %{notSpace:logger.name}\] %{data:klog.message}"#,
                            ]
                            .join("\n"),
                        },
                    },
                ),
                processor(
                    "Map `klog.severity` to a level",
                    Kind::LookupProcessor {
                        source: "klog.severity".into(),
                        target: "klog.level".into(),
                        lookup_table: s(&["I,info", "W,warn", "E,error", "F,critical"]),
                        default_lookup: None,
                        lookup_enrichment_table: None,
                    },
                ),
                // Only what the parser above extracted: a `level` attribute
                // another pipeline or a JSON log left is not this one's.
                processor("Define the parsed level as the official status of the log", Kind::StatusRemapper { sources: s(&["logfmt.level", "logfmt.lvl", "klog.level"]) }),
                processor("Define the parsed message as the official message of the log", Kind::MessageRemapper { sources: s(&["logfmt.msg", "klog.message"]) }),
                processor("Define the parsed time as the official date of the log", Kind::DateRemapper { sources: s(&["logfmt.time", "logfmt.ts", "logfmt.t"]) }),
            ],
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intake::logs::{Defaults, process};

    fn log(v: Value) -> LogRow {
        process(v.as_object().unwrap().clone(), &Defaults::default(), 0)
    }

    #[test]
    fn integration_pipelines_compile() {
        for p in integration_pipelines() {
            validate(&p).unwrap_or_else(|e| panic!("{}: {e}", p.name));
        }
    }

    #[test]
    fn nginx_access_log() {
        let mut l = log(json!({
            "ddsource": "nginx",
            "message": r#"10.0.3.7 - - [16/Nov/2026:16:31:00 +0000] "GET /api/cart?u=7 HTTP/1.1" 503 12 "-" "curl/8.5.0""#,
        }));
        preview(&integration_pipelines(), &mut l);
        assert_eq!(l.ts, 1794846660000);
        assert_eq!(l.status, "error");
        assert_eq!(l.attributes["http"]["status_code"], 503);
        assert_eq!(l.attributes["http"]["status_category"], "error");
        assert_eq!(l.attributes["http"]["url_details"]["path"], "/api/cart");
        assert_eq!(l.attributes["http"]["url_details"]["queryString"]["u"], "7");
        assert_eq!(
            l.attributes["http"]["useragent_details"]["browser"]["family"],
            "curl"
        );
        assert_eq!(l.attributes["network"]["client"]["ip"], "10.0.3.7");
    }

    #[test]
    fn nginx_error_log() {
        let mut l = log(json!({
            "ddsource": "nginx",
            "message": r#"2026/11/16 16:38:48 [error] 31#31: *53752 upstream timed out (110: Connection timed out) while reading response header from upstream, client: 10.0.111.9, server: shop, request: "POST /api/orders HTTP/1.1", upstream: "http://10.42.0.17:8080/api/orders""#,
        }));
        preview(&integration_pipelines(), &mut l);
        let a = &l.attributes;
        assert_eq!(l.status, "error");
        assert_eq!(
            a["error"]["message"],
            "31#31: *53752 upstream timed out (110: Connection timed out) while reading response header from upstream"
        );
        assert_eq!(a["client"], "10.0.111.9");
        assert_eq!(a["server"], "shop");
        assert_eq!(a["request"], "POST /api/orders HTTP/1.1");
        assert_eq!(a["upstream"], "http://10.42.0.17:8080/api/orders");
    }

    #[test]
    fn postgresql_container_logs() {
        let run = |m: &str| {
            let mut l = log(json!({"ddsource": "postgresql", "status": "error", "message": m}));
            preview(&integration_pipelines(), &mut l);
            l
        };
        let l = run(
            r#"2026-11-17 10:14:23.259 UTC [78] ERROR:  relation "schema_version" does not exist at character 21"#,
        );
        assert_eq!(l.status, "error");
        assert_eq!(l.attributes["db"]["pid"], 78);
        assert_eq!(
            l.attributes["db"]["message"],
            r#"relation "schema_version" does not exist at character 21"#
        );
        let l = run(
            "2026-11-17 10:14:15.027 UTC [1] LOG:  database system is ready to accept connections",
        );
        assert_eq!(l.status, "info");
        assert!(l.message.starts_with("2026-11-17"));
        let l = run(
            "2026-11-17 10:14:23.259 UTC [78] STATEMENT:  SELECT version\n\tFROM schema_version",
        );
        assert_eq!(l.status, "info");
        let l = run("2026-11-17 10:14:15.027 UTC [1] LOG:  duration: 12.5 ms  statement: SELECT 1");
        assert_eq!(l.attributes["db"]["statement"], "SELECT 1");
        assert_eq!(l.attributes["duration"], 12500000.0);
        let l = run("FATAL:  remaining connection slots are reserved");
        assert_eq!(l.status, "critical");
    }

    #[test]
    fn traefik_json_access_log() {
        let mut l = log(json!({
            "ddsource": "traefik",
            "status": "info",
            "message": r#"{"ClientHost":"203.0.113.7","DownstreamStatus":502,"Duration":1234567,"RequestHost":"miniflux.bailleul.dev","RequestMethod":"GET","RequestPath":"/feeds?x=1","RouterName":"miniflux@kubernetes","TraceId":"e205b806eba1583b23aa4a49b3cf6933","SpanId":"840db9e96fbf9bdd","request_User-Agent":"curl/8.5.0","level":"info","msg":"","time":"2026-11-17T10:14:30Z"}"#,
        }));
        preview(&integration_pipelines(), &mut l);
        let a = &l.attributes;
        assert_eq!(l.status, "error");
        assert_eq!(l.message, "GET miniflux.bailleul.dev/feeds?x=1 502");
        assert_eq!(a["http"]["status_code"], 502);
        assert_eq!(a["http"]["url_details"]["path"], "/feeds");
        assert_eq!(a["network"]["client"]["ip"], "203.0.113.7");
        assert_eq!(a["duration"], 1234567);
        assert_eq!(a["RouterName"], "miniflux@kubernetes");
        assert_eq!(l.trace_id.as_deref(), Some("2569948217778071859"));
        assert_eq!(l.span_id.as_deref(), Some("9515465999957072861"));
        assert_eq!(l.ts, 1794910470000);

        // Traefik's own logs, in JSON too, are left alone.
        let mut l = log(json!({
            "ddsource": "traefik",
            "status": "info",
            "message": r#"{"level":"error","error":"secret miniflux/miniflux-tls does not exist","message":"Error configuring TLS","time":"2026-11-17T10:14:30Z"}"#,
        }));
        preview(&integration_pipelines(), &mut l);
        assert_eq!(
            (l.status.as_str(), l.message.as_str()),
            ("error", "Error configuring TLS")
        );
    }

    #[test]
    fn coredns_redis_and_agent_logs() {
        let run = |source: &str, m: &str| {
            let mut l = log(json!({"ddsource": source, "status": "info", "message": m}));
            preview(&integration_pipelines(), &mut l);
            l
        };
        let l = run(
            "coredns",
            "[WARNING] No files matching import glob pattern: /etc/coredns/custom/*.server",
        );
        assert_eq!(l.status, "warn");
        let l = run(
            "coredns",
            r#"[INFO] 10.42.0.9:40212 - 5012 "A IN id.bailleul.dev. udp 33 false 512" NOERROR qr,rd,ra 64 0.004211s"#,
        );
        assert_eq!(l.attributes["dns"]["question"]["name"], "id.bailleul.dev.");
        assert_eq!(l.attributes["dns"]["response_code"], "NOERROR");
        let l = run(
            "redis",
            "1:M 17 Nov 2026 10:14:15.027 # WARNING Memory overcommit must be enabled!",
        );
        assert_eq!(l.status, "warn");
        let l = run(
            "redis",
            "1:M 17 Nov 2026 10:14:15.027 * Ready to accept connections tcp",
        );
        assert_eq!(l.status, "notice");
        let l = run(
            "agent",
            "2026-11-17 10:14:47 UTC | CORE | WARN | (comp/core/remoteagentregistry/impl/services.go:136 in func2) | Failed to collect telemetry | really",
        );
        assert_eq!(l.status, "warn");
        assert_eq!(l.message, "Failed to collect telemetry | really");
        assert_eq!(l.attributes["agent"]["component"], "CORE");
    }

    #[test]
    fn go_logfmt_and_klog() {
        let run = |m: &str| {
            // What an Agent sends for a line read from stderr.
            let mut l = log(json!({"ddsource": "miniflux", "status": "error", "message": m}));
            preview(&integration_pipelines(), &mut l);
            l
        };
        let l = run(r#"level=INFO msg="Created new admin user" username=guillaume user_id=1"#);
        assert_eq!(
            (l.status.as_str(), l.message.as_str()),
            ("info", "Created new admin user")
        );
        assert_eq!(l.attributes["logfmt"]["username"], "guillaume");
        let l = run(
            r#"time="2026-11-17T10:13:59Z" level=warning msg="Volume pvc-7fc64f33 has been created""#,
        );
        assert_eq!(
            (l.status.as_str(), l.message.as_str()),
            ("warn", "Volume pvc-7fc64f33 has been created")
        );
        assert_eq!(l.ts, 1794910439000);
        let l = run(
            r#"E1117 10:13:59.888380       1 event.go:389] "Event occurred" object="miniflux/data""#,
        );
        assert_eq!(l.status, "error");
        assert_eq!(l.attributes["logger"]["name"], "event.go:389");
        assert_eq!(l.message, r#""Event occurred" object="miniflux/data""#);
        // Anything else is left as it was.
        let l = run("the level=3 is not at the start");
        assert_eq!(
            (l.status.as_str(), l.message.as_str()),
            ("error", "the level=3 is not at the start")
        );
        assert!(l.attributes.is_empty());
    }

    #[test]
    fn processors() {
        let defs = vec![PipelineDef {
            id: None,
            typ: "pipeline".into(),
            name: "app".into(),
            is_enabled: true,
            filter: FilterDef {
                query: "service:app".into(),
            },
            is_read_only: false,
            description: None,
            processors: vec![
                processor(
                    "",
                    Kind::AttributeRemapper {
                        sources: s(&["user_id"]),
                        source_type: "attribute".into(),
                        target: "usr.id".into(),
                        target_type: "attribute".into(),
                        target_format: Some("string".into()),
                        preserve_source: false,
                        override_on_conflict: false,
                    },
                ),
                processor(
                    "",
                    Kind::AttributeRemapper {
                        sources: s(&["team"]),
                        source_type: "attribute".into(),
                        target: "team".into(),
                        target_type: "tag".into(),
                        target_format: None,
                        preserve_source: true,
                        override_on_conflict: false,
                    },
                ),
                processor(
                    "",
                    Kind::ArithmeticProcessor {
                        expression: "(bytes_in + bytes_out) / 1024".into(),
                        target: "kb".into(),
                        is_replace_missing: false,
                    },
                ),
                processor(
                    "",
                    Kind::StringBuilderProcessor {
                        template: "%{usr.id} on %{team}".into(),
                        target: "who".into(),
                        is_replace_missing: false,
                    },
                ),
                processor(
                    "",
                    Kind::LookupProcessor {
                        source: "country".into(),
                        target: "country_name".into(),
                        lookup_table: s(&["fr, France", "de, Germany"]),
                        default_lookup: Some("Unknown".into()),
                        lookup_enrichment_table: None,
                    },
                ),
                processor(
                    "",
                    Kind::ServiceRemapper {
                        sources: s(&["svc"]),
                    },
                ),
                processor(
                    "",
                    Kind::Pipeline {
                        filter: FilterDef {
                            query: "@kb:>1".into(),
                        },
                        processors: vec![processor(
                            "",
                            Kind::MessageRemapper {
                                sources: s(&["who"]),
                            },
                        )],
                    },
                ),
                processor(
                    "disabled",
                    Kind::StatusRemapper {
                        sources: s(&["lvl"]),
                    },
                ),
            ],
        }];
        let mut defs = defs;
        defs[0].processors[7].is_enabled = false;
        let mut l = log(
            json!({"service": "app", "message": "x", "user_id": 42, "team": "pay", "bytes_in": 1024, "bytes_out": 2048, "country": "fr", "svc": "renamed", "lvl": "error"}),
        );
        preview(&defs, &mut l);
        let a = &l.attributes;
        assert!(a.get("user_id").is_none());
        assert_eq!(a["usr"]["id"], "42");
        assert!(l.tags.contains(&"team:pay".to_string()));
        assert_eq!(a["kb"], 3);
        assert_eq!(a["who"], "42 on pay");
        assert_eq!(a["country_name"], "France");
        assert_eq!(l.service.as_deref(), Some("renamed"));
        assert_eq!(l.message, "42 on pay");
        assert_eq!(l.status, "info");

        // Another service: the filter keeps the pipeline out.
        let mut other = log(json!({"service": "web", "message": "x", "user_id": 1}));
        preview(&defs, &mut other);
        assert_eq!(other.attributes["user_id"], 1);
    }

    #[test]
    fn datadog_api_json_round_trips() {
        let j = json!({
            "type": "pipeline", "name": "p", "is_enabled": true, "filter": {"query": "source:x"},
            "processors": [
                {"type": "grok-parser", "name": "g", "is_enabled": true, "source": "message", "samples": [],
                 "grok": {"support_rules": "", "match_rules": "r %{word:w}"}},
                {"type": "category-processor", "name": "c", "is_enabled": true, "target": "cat",
                 "categories": [{"filter": {"query": "@w:a"}, "name": "A"}]}
            ]
        });
        let p: PipelineDef = serde_json::from_value(j.clone()).unwrap();
        validate(&p).unwrap();
        let back = serde_json::to_value(&p).unwrap();
        assert_eq!(back["processors"][0]["grok"]["match_rules"], "r %{word:w}");
        assert_eq!(back["processors"][1]["type"], "category-processor");
    }
}
