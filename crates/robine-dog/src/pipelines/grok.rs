//! Datadog's grok parser: rules `name pattern`, where the pattern is a regex
//! with `%{MATCHER(args):attribute:filter(args)}` extractions. The first
//! rule matching the whole text wins.
//!
//! Matchers: date, regex, notSpace, boolean, number(Str), numberExt(Str),
//! integer(Str), integerExt(Str), word, doubleQuotedString,
//! singleQuotedString, quotedString, uuid, mac, ipv4, ipv6, ip, hostname,
//! ipOrHost, port, data, and any helper or match rule by name.
//! Filters: number, integer, boolean, nullIf, json, lowercase, uppercase,
//! keyvalue, scale, url, querystring, decodeuricomponent, useragent, csv,
//! array.

use super::useragent;
use regex::Regex;
use serde_json::{Map, Value, json};
use std::collections::HashMap;

#[derive(Debug, Clone)]
enum Conv {
    Text,
    Number,
    Integer,
    Boolean,
    /// A chrono format and a timezone (IANA name or offset; UTC if none).
    Date(String, Option<String>),
    /// Quoted strings lose their quotes.
    Unquote,
}

#[derive(Debug, Clone)]
pub enum Filter {
    Number,
    Integer,
    Boolean,
    NullIf(String),
    Json,
    Lowercase,
    Uppercase,
    KeyValue {
        sep: String,
        allow: String,
        quoting: String,
        delimiter: Option<String>,
    },
    Scale(f64),
    Url,
    QueryString,
    DecodeUri,
    UserAgent,
    Csv {
        headers: Vec<String>,
        sep: char,
        quote: char,
    },
    Array {
        open: Option<(char, char)>,
        sep: String,
        item: Option<Box<Filter>>,
    },
}

#[derive(Debug, Clone)]
struct Capture {
    group: String,
    attr: String,
    conv: Conv,
    filter: Option<Filter>,
}

#[derive(Debug, Clone)]
struct Rule {
    name: String,
    re: Regex,
    captures: Vec<Capture>,
}

#[derive(Debug, Clone)]
pub struct Grok {
    rules: Vec<Rule>,
}

/// What a sample turned into, for the editor's live preview.
#[derive(Debug, serde::Serialize)]
pub struct Outcome {
    pub rule: Option<String>,
    pub attributes: Map<String, Value>,
}

fn split_rules(text: &str) -> Result<Vec<(String, String)>, String> {
    let mut out = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (name, pattern) = line
            .split_once(char::is_whitespace)
            .ok_or_else(|| format!("line {}: expected `rule_name pattern`", i + 1))?;
        if !name.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '.') {
            return Err(format!("line {}: invalid rule name `{name}`", i + 1));
        }
        out.push((name.to_string(), pattern.trim_start().to_string()));
    }
    Ok(out)
}

/// Splits `a, "b, c", 'd'` into unquoted arguments. Inside quotes, as in
/// a string literal, `\\` is one backslash and `\"` a quote; any other
/// escape is kept for the regex (`\d`).
fn args(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut escaped = false;
    let mut any = false;
    for c in s.chars() {
        if escaped {
            if c != '\\' && Some(c) != quote {
                cur.push('\\');
            }
            cur.push(c);
            escaped = false;
            continue;
        }
        match (quote, c) {
            (_, '\\') => {
                escaped = true;
            }
            (Some(q), c) if c == q => quote = None,
            (Some(_), c) => cur.push(c),
            (None, '"' | '\'') => {
                quote = Some(c);
                any = true;
            }
            (None, ',') => {
                out.push(std::mem::take(&mut cur).trim().to_string());
                any = false;
            }
            (None, c) => {
                cur.push(c);
                if !c.is_whitespace() {
                    any = true;
                }
            }
        }
    }
    if any || !cur.trim().is_empty() || !out.is_empty() {
        out.push(cur.trim().to_string());
    }
    out
}

/// `name(args)` → (name, args); `name` → (name, []).
fn call(s: &str) -> (String, Vec<String>) {
    match s.find('(') {
        Some(i) if s.ends_with(')') => (s[..i].to_string(), args(&s[i + 1..s.len() - 1])),
        _ => (s.to_string(), vec![]),
    }
}

/// Splits `matcher(args):attr:filter(args)` at the colons outside
/// parentheses and quotes.
fn token_parts(s: &str) -> Vec<String> {
    let mut parts = vec![String::new()];
    let mut depth = 0;
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for c in s.chars() {
        if escaped {
            parts.last_mut().unwrap().push(c);
            escaped = false;
            continue;
        }
        match (quote, c) {
            (_, '\\') => escaped = true,
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(c),
            (None, '(') => depth += 1,
            (None, ')') => depth -= 1,
            (None, ':') if depth == 0 && parts.len() < 3 => {
                parts.push(String::new());
                continue;
            }
            _ => {}
        }
        if escaped {
            parts.last_mut().unwrap().push('\\');
        } else {
            parts.last_mut().unwrap().push(c);
        }
    }
    parts
}

/// Java/Joda date pattern → (regex, chrono format, has year, has zone).
fn date_pattern(p: &str) -> Result<(String, String, bool, bool), String> {
    let chars: Vec<char> = p.chars().collect();
    let (mut re, mut fmt) = (String::new(), String::new());
    let (mut year, mut zone) = (false, false);
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '\'' {
            let mut j = i + 1;
            let mut lit = String::new();
            while j < chars.len() && chars[j] != '\'' {
                lit.push(chars[j]);
                j += 1;
            }
            if lit.is_empty() {
                lit.push('\'');
            }
            re.push_str(&regex::escape(&lit));
            fmt.push_str(&lit.replace('%', "%%"));
            i = j + 1;
            continue;
        }
        if !c.is_ascii_alphabetic() {
            re.push_str(&regex::escape(&c.to_string()));
            if c == '%' {
                fmt.push_str("%%");
            } else {
                fmt.push(c);
            }
            i += 1;
            continue;
        }
        let mut n = 1;
        while i + n < chars.len() && chars[i + n] == c {
            n += 1;
        }
        let (r, f): (String, &str) = match (c, n) {
            ('y' | 'Y' | 'u', 2) => {
                year = true;
                (r"\d{2}".into(), "%y")
            }
            ('y' | 'Y' | 'u', _) => {
                year = true;
                (r"\d{4}".into(), "%Y")
            }
            ('M', 1 | 2) => (r"\d{1,2}".into(), "%m"),
            ('M', 3) => ("[A-Za-z]{3}".into(), "%b"),
            ('M', _) => ("[A-Za-z]+".into(), "%B"),
            ('d', _) => (r"\d{1,2}".into(), "%d"),
            ('H' | 'k', _) => (r"\d{1,2}".into(), "%H"),
            ('h' | 'K', _) => (r"\d{1,2}".into(), "%I"),
            ('m', _) => (r"\d{1,2}".into(), "%M"),
            ('s', _) => (r"\d{1,2}".into(), "%S"),
            ('S', n) => (
                format!(r"\d{{{n}}}"),
                match n {
                    3 => "%3f",
                    6 => "%6f",
                    9 => "%9f",
                    _ => return Err(format!("{n} fraction digits are not supported (use 3, 6 or 9)")),
                },
            ),
            ('a', _) => ("[AaPp][Mm]".into(), "%p"),
            ('E', 1..=3) => ("[A-Za-z]{3}".into(), "%a"),
            ('E', _) => ("[A-Za-z]+".into(), "%A"),
            ('Z', 1) | ('x', 1 | 2) => {
                zone = true;
                ("[+-]\\d{4}".into(), "%z")
            }
            ('Z', _) | ('X' | 'x', _) => {
                zone = true;
                ("(?:Z|[+-]\\d{2}:?\\d{2}|[+-]\\d{2})".into(), "%#z")
            }
            (other, _) => return Err(format!("unsupported date letter `{other}`")),
        };
        re.push_str(&r);
        fmt.push_str(f);
        i += n;
    }
    Ok((re, fmt, year, zone))
}

pub fn parse_date(text: &str, fmt: &str, tz: Option<&str>) -> Option<i64> {
    use chrono::{DateTime, NaiveDateTime, TimeZone};
    let has_year = fmt.contains("%Y") || fmt.contains("%y");
    let (text, fmt) = if has_year {
        (text.to_string(), fmt.to_string())
    } else {
        // Syslog-style dates carry no year: this year's.
        (
            format!("{} {text}", chrono::Utc::now().format("%Y")),
            format!("%Y {fmt}"),
        )
    };
    let text = text.replace('Z', "+00:00");
    if fmt.contains('z') {
        return DateTime::parse_from_str(&text, &fmt)
            .ok()
            .map(|d| d.timestamp_millis());
    }
    let naive = NaiveDateTime::parse_from_str(&text, &fmt)
        .ok()
        .or_else(|| {
            chrono::NaiveDate::parse_from_str(&text, &fmt)
                .ok()
                .and_then(|d| d.and_hms_opt(0, 0, 0))
        })?;
    match tz.map(str::trim).filter(|t| !t.is_empty()) {
        None | Some("UTC" | "GMT" | "Z") => Some(naive.and_utc().timestamp_millis()),
        Some(t) => {
            if let Ok(zone) = t.parse::<chrono_tz::Tz>() {
                return zone
                    .from_local_datetime(&naive)
                    .earliest()
                    .map(|d| d.timestamp_millis());
            }
            let off = t.trim_start_matches("UTC").trim_start_matches("GMT");
            let secs = parse_offset(off)?;
            chrono::FixedOffset::east_opt(secs)?
                .from_local_datetime(&naive)
                .earliest()
                .map(|d| d.timestamp_millis())
        }
    }
}

fn parse_offset(s: &str) -> Option<i32> {
    let (sign, rest) = match s.chars().next()? {
        '+' => (1, &s[1..]),
        '-' => (-1, &s[1..]),
        _ => return None,
    };
    let digits: String = rest.chars().filter(|c| c.is_ascii_digit()).collect();
    let (h, m) = match digits.len() {
        1 | 2 => (digits.parse::<i32>().ok()?, 0),
        4 => (digits[..2].parse::<i32>().ok()?, digits[2..].parse::<i32>().ok()?),
        _ => return None,
    };
    Some(sign * (h * 3600 + m * 60))
}

fn builtin(name: &str, a: &[String]) -> Result<Option<(String, Conv)>, String> {
    let num = r"[+-]?(?:\d+(?:\.\d*)?|\.\d+)";
    let ext = r"(?:[eE][+-]?\d+)?";
    let ipv4 = r"(?:(?:25[0-5]|2[0-4]\d|1?\d?\d)\.){3}(?:25[0-5]|2[0-4]\d|1?\d?\d)";
    let ipv6 = r"(?:[0-9A-Fa-f]{0,4}:){2,7}(?:[0-9A-Fa-f]{0,4}|(?:\d{1,3}\.){3}\d{1,3})";
    let host = r"[A-Za-z0-9](?:[A-Za-z0-9-]{0,61}[A-Za-z0-9])?(?:\.[A-Za-z0-9](?:[A-Za-z0-9-]{0,61}[A-Za-z0-9])?)*\.?";
    Ok(Some(match name {
        "word" => (r"\b\w+\b".into(), Conv::Text),
        "notSpace" => (r"\S+".into(), Conv::Text),
        "data" => (r".*?".into(), Conv::Text),
        "number" => (num.into(), Conv::Number),
        "numberStr" => (num.into(), Conv::Text),
        "numberExt" => (format!("{num}{ext}"), Conv::Number),
        "numberExtStr" => (format!("{num}{ext}"), Conv::Text),
        "integer" => (r"[+-]?\d+".into(), Conv::Integer),
        "integerStr" => (r"[+-]?\d+".into(), Conv::Text),
        "integerExt" => (format!(r"[+-]?\d+{ext}"), Conv::Integer),
        "integerExtStr" => (format!(r"[+-]?\d+{ext}"), Conv::Text),
        "boolean" => {
            let (t, f) = match a {
                [t, f, ..] => (t.clone(), f.clone()),
                _ => ("(?i:true)".into(), "(?i:false)".into()),
            };
            (format!("(?:{t}|{f})"), Conv::Boolean)
        }
        "doubleQuotedString" => (r#""(?:[^"\\]|\\.)*""#.into(), Conv::Unquote),
        "singleQuotedString" => (r"'(?:[^'\\]|\\.)*'".into(), Conv::Unquote),
        "quotedString" => (
            r#"(?:"(?:[^"\\]|\\.)*"|'(?:[^'\\]|\\.)*')"#.into(),
            Conv::Unquote,
        ),
        "uuid" => (
            r"[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}".into(),
            Conv::Text,
        ),
        "mac" => (r"(?:[0-9A-Fa-f]{2}[:-]){5}[0-9A-Fa-f]{2}".into(), Conv::Text),
        "ipv4" => (ipv4.into(), Conv::Text),
        "ipv6" => (ipv6.into(), Conv::Text),
        "ip" => (format!("(?:{ipv6}|{ipv4})"), Conv::Text),
        "hostname" => (host.into(), Conv::Text),
        "ipOrHost" => (format!("(?:{ipv6}|{ipv4}|{host})"), Conv::Text),
        "port" => (r"\d{1,5}".into(), Conv::Integer),
        "regex" => {
            let r = a.first().ok_or("regex() needs a pattern")?.clone();
            Regex::new(&r).map_err(|e| format!("regex(\"{r}\"): {e}"))?;
            (r, Conv::Text)
        }
        "date" => {
            let p = a.first().ok_or("date() needs a pattern")?;
            let (re, fmt, _, _) = date_pattern(p)?;
            (re, Conv::Date(fmt, a.get(1).cloned()))
        }
        _ => return Ok(None),
    }))
}

fn filter(name: &str, a: &[String]) -> Result<Filter, String> {
    Ok(match name {
        "number" => Filter::Number,
        "integer" => Filter::Integer,
        "boolean" => Filter::Boolean,
        "nullIf" => Filter::NullIf(a.first().cloned().unwrap_or_default()),
        "json" => Filter::Json,
        "lowercase" => Filter::Lowercase,
        "uppercase" => Filter::Uppercase,
        "keyvalue" => Filter::KeyValue {
            sep: a.first().filter(|s| !s.is_empty()).cloned().unwrap_or_else(|| "=".into()),
            allow: a.get(1).cloned().unwrap_or_default(),
            quoting: a.get(2).cloned().unwrap_or_default(),
            delimiter: a.get(3).cloned().filter(|s| !s.is_empty()),
        },
        "scale" => Filter::Scale(
            a.first()
                .and_then(|s| s.parse().ok())
                .ok_or("scale() needs a number")?,
        ),
        "url" => Filter::Url,
        "querystring" => Filter::QueryString,
        "decodeuricomponent" => Filter::DecodeUri,
        "useragent" => Filter::UserAgent,
        "csv" => Filter::Csv {
            headers: a
                .first()
                .ok_or("csv() needs the headers")?
                .split(',')
                .map(|h| h.trim().to_string())
                .collect(),
            sep: a.get(1).and_then(|s| s.chars().next()).unwrap_or(','),
            quote: a.get(2).and_then(|s| s.chars().next()).unwrap_or('"'),
        },
        "array" => {
            let mut a = a.to_vec();
            let item = match a.last().map(|s| call(s)) {
                Some((n, x)) if a.len() >= 2 && filter(&n, &x).is_ok() => {
                    a.pop();
                    Some(Box::new(filter(&n, &x)?))
                }
                _ => None,
            };
            let (open, sep) = match a.as_slice() {
                [] => (None, ",".to_string()),
                [sep] => (None, sep.clone()),
                [oc, sep, ..] => {
                    let mut c = oc.chars();
                    (c.next().zip(c.next()), sep.clone())
                }
            };
            Filter::Array { open, sep, item }
        }
        other => return Err(format!("unknown filter `{other}`")),
    })
}

struct Compiler<'a> {
    rules: &'a HashMap<String, String>,
    n: usize,
}

impl Compiler<'_> {
    fn compile(
        &mut self,
        pattern: &str,
        depth: usize,
        caps: &mut Vec<Capture>,
    ) -> Result<String, String> {
        if depth > 16 {
            return Err("rules reference each other too deeply".into());
        }
        let mut out = String::new();
        let mut rest = pattern;
        while let Some(i) = rest.find("%{") {
            out.push_str(&rest[..i]);
            let after = &rest[i + 2..];
            // The token ends at the first `}` outside quotes.
            let (mut end, mut quote, mut esc) = (None, None, false);
            for (j, c) in after.char_indices() {
                if esc {
                    esc = false;
                    continue;
                }
                match (quote, c) {
                    (_, '\\') => esc = true,
                    (Some(q), c) if c == q => quote = None,
                    (Some(_), _) => {}
                    (None, '"' | '\'') => quote = Some(c),
                    (None, '}') => {
                        end = Some(j);
                        break;
                    }
                    _ => {}
                }
            }
            let end = end.ok_or("unclosed `%{`")?;
            let token = &after[..end];
            rest = &after[end + 1..];

            let parts = token_parts(token);
            let (name, a) = call(parts[0].trim());
            let attr = parts.get(1).map(|s| s.trim().to_string()).unwrap_or_default();
            let filt = match parts.get(2).map(|s| s.trim()).filter(|s| !s.is_empty()) {
                Some(f) => {
                    let (n, x) = call(f);
                    Some(filter(&n, &x)?)
                }
                None => None,
            };
            let (body, conv) = if let Some(sub) = self.rules.get(&name) {
                let sub = sub.clone();
                (self.compile(&sub, depth + 1, caps)?, Conv::Text)
            } else if let Some(b) = builtin(&name, &a)? {
                b
            } else {
                return Err(format!("unknown matcher or rule `{name}`"));
            };
            if attr.is_empty() && filt.is_none() {
                out.push_str(&format!("(?:{body})"));
            } else {
                let group = format!("g{}", self.n);
                self.n += 1;
                caps.push(Capture {
                    group: group.clone(),
                    attr,
                    conv,
                    filter: filt,
                });
                out.push_str(&format!("(?P<{group}>{body})"));
            }
        }
        out.push_str(rest);
        Ok(out)
    }
}

impl Grok {
    pub fn new(match_rules: &str, support_rules: &str) -> Result<Grok, String> {
        let support = split_rules(support_rules)?;
        let matching = split_rules(match_rules)?;
        if matching.is_empty() {
            return Err("at least one parsing rule is needed".into());
        }
        let all: HashMap<String, String> = support
            .iter()
            .chain(matching.iter())
            .cloned()
            .collect();
        let mut compiler = Compiler { rules: &all, n: 0 };
        let mut rules = Vec::new();
        for (name, pattern) in matching {
            let mut captures = Vec::new();
            let body = compiler
                .compile(&pattern, 0, &mut captures)
                .map_err(|e| format!("rule `{name}`: {e}"))?;
            let re = Regex::new(&format!("(?s)^(?:{body})$"))
                .map_err(|e| format!("rule `{name}`: {e}"))?;
            rules.push(Rule {
                name,
                re,
                captures,
            });
        }
        Ok(Grok { rules })
    }

    pub fn parse(&self, text: &str) -> Option<Outcome> {
        for rule in &self.rules {
            let Some(m) = rule.re.captures(text) else {
                continue;
            };
            let mut attributes = Map::new();
            for c in &rule.captures {
                let Some(raw) = m.name(&c.group) else {
                    continue;
                };
                let Some(mut v) = convert(raw.as_str(), &c.conv) else {
                    continue;
                };
                if let Some(f) = &c.filter {
                    match apply_filter(f, v) {
                        Some(x) => v = x,
                        None => continue,
                    }
                }
                if c.attr.is_empty() {
                    if let Value::Object(o) = v {
                        for (k, x) in o {
                            set_path(&mut attributes, &k, x);
                        }
                    }
                } else {
                    set_path(&mut attributes, &c.attr, v);
                }
            }
            return Some(Outcome {
                rule: Some(rule.name.clone()),
                attributes,
            });
        }
        None
    }
}

fn convert(raw: &str, conv: &Conv) -> Option<Value> {
    Some(match conv {
        Conv::Text => json!(raw),
        Conv::Number => json!(raw.parse::<f64>().ok()?),
        Conv::Integer => match raw.parse::<i64>() {
            Ok(i) => json!(i),
            Err(_) => json!(raw.parse::<f64>().ok()? as i64),
        },
        Conv::Boolean => json!(raw.eq_ignore_ascii_case("true")),
        Conv::Date(fmt, tz) => json!(parse_date(raw, fmt, tz.as_deref())?),
        Conv::Unquote => {
            let inner = &raw[1..raw.len().saturating_sub(1).max(1)];
            json!(inner.replace("\\\"", "\"").replace("\\'", "'"))
        }
    })
}

fn text_of(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

pub fn decode_uri(s: &str) -> String {
    percent_encoding::percent_decode_str(&s.replace('+', " "))
        .decode_utf8_lossy()
        .into_owned()
}

pub fn query_string(s: &str) -> Map<String, Value> {
    s.trim_start_matches('?')
        .split('&')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let (k, v) = p.split_once('=').unwrap_or((p, ""));
            (decode_uri(k), json!(decode_uri(v)))
        })
        .collect()
}

/// `http.url_details`, as Datadog's URL parser fills it.
pub fn url_details(s: &str) -> Option<Value> {
    let u = url::Url::parse(s)
        .or_else(|_| url::Url::parse(&format!("http://placeholder{s}")))
        .ok()?;
    let relative = u.host_str() == Some("placeholder");
    let mut d = Map::new();
    if !relative {
        d.insert("scheme".into(), json!(u.scheme()));
        if let Some(h) = u.host_str() {
            d.insert("host".into(), json!(h));
        }
        if let Some(p) = u.port() {
            d.insert("port".into(), json!(p));
        }
    }
    d.insert("path".into(), json!(u.path()));
    if let Some(q) = u.query() {
        d.insert("queryString".into(), Value::Object(query_string(q)));
    }
    Some(Value::Object(d))
}

fn key_value(s: &str, sep: &str, allow: &str, quoting: &str, delimiter: Option<&str>) -> Map<String, Value> {
    let quotes: Vec<(char, char)> = if quoting.is_empty() {
        vec![('"', '"'), ('\'', '\''), ('<', '>')]
    } else {
        let c: Vec<char> = quoting.chars().collect();
        c.chunks(2)
            .map(|p| (p[0], *p.get(1).unwrap_or(&p[0])))
            .collect()
    };
    let value_class = format!(r"[\w.\-_@{}]+", regex::escape(allow));
    let quoted: String = quotes
        .iter()
        .map(|(o, c)| {
            format!(
                "{}([^{}]*){}",
                regex::escape(&o.to_string()),
                regex::escape(&c.to_string()),
                regex::escape(&c.to_string())
            )
        })
        .collect::<Vec<_>>()
        .join("|");
    let re = Regex::new(&format!(
        r"([\w.\-_@]+)\s*{}\s*(?:{quoted}|({value_class}))",
        regex::escape(sep)
    ));
    let mut out = Map::new();
    let Ok(re) = re else { return out };
    let parts: Vec<&str> = match delimiter {
        Some(d) => s.split(d).collect(),
        None => vec![s],
    };
    for part in parts {
        for c in re.captures_iter(part) {
            let key = c[1].to_string();
            let val = (2..c.len()).find_map(|i| c.get(i)).map(|m| m.as_str());
            if let Some(v) = val {
                out.insert(key, json!(v));
            }
        }
    }
    out
}

fn csv_line(s: &str, sep: char, quote: char) -> Vec<String> {
    let mut out = vec![String::new()];
    let mut inq = false;
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if inq {
            if c == quote && chars.get(i + 1) == Some(&quote) {
                out.last_mut().unwrap().push(quote);
                i += 1;
            } else if c == quote {
                inq = false;
            } else {
                out.last_mut().unwrap().push(c);
            }
        } else if c == quote {
            inq = true;
        } else if c == sep {
            out.push(String::new());
        } else {
            out.last_mut().unwrap().push(c);
        }
        i += 1;
    }
    out
}

pub fn apply_filter(f: &Filter, v: Value) -> Option<Value> {
    let s = text_of(&v);
    Some(match f {
        Filter::Number => json!(s.trim().parse::<f64>().ok()?),
        Filter::Integer => json!(s.trim().parse::<f64>().ok()? as i64),
        Filter::Boolean => json!(s.trim().eq_ignore_ascii_case("true")),
        Filter::NullIf(x) => {
            if &s == x {
                return None;
            }
            v
        }
        Filter::Json => serde_json::from_str(&s).ok()?,
        Filter::Lowercase => json!(s.to_lowercase()),
        Filter::Uppercase => json!(s.to_uppercase()),
        Filter::KeyValue {
            sep,
            allow,
            quoting,
            delimiter,
        } => Value::Object(key_value(&s, sep, allow, quoting, delimiter.as_deref())),
        Filter::Scale(k) => json!(s.trim().parse::<f64>().ok()? * k),
        Filter::Url => url_details(&s)?,
        Filter::QueryString => Value::Object(query_string(&s)),
        Filter::DecodeUri => json!(decode_uri(&s)),
        Filter::UserAgent => useragent::parse(&s),
        Filter::Csv {
            headers,
            sep,
            quote,
        } => Value::Object(
            headers
                .iter()
                .cloned()
                .zip(csv_line(&s, *sep, *quote).into_iter().map(Value::String))
                .collect(),
        ),
        Filter::Array { open, sep, item } => {
            let mut body = s.trim();
            if let Some((o, c)) = open {
                body = body.strip_prefix(*o)?.strip_suffix(*c)?;
            }
            let items: Vec<Value> = body
                .split(sep.as_str())
                .map(str::trim)
                .filter(|x| !x.is_empty())
                .filter_map(|x| match item {
                    Some(f) => apply_filter(f, json!(x)),
                    None => Some(json!(x)),
                })
                .collect();
            Value::Array(items)
        }
    })
}

/// Sets `a.b.c`, creating the intermediate objects.
pub fn set_path(m: &mut Map<String, Value>, path: &str, v: Value) {
    let mut parts = path.splitn(2, '.');
    let head = parts.next().unwrap_or_default().to_string();
    match parts.next() {
        None => {
            m.insert(head, v);
        }
        Some(rest) => {
            let entry = m.entry(head).or_insert_with(|| Value::Object(Map::new()));
            if !entry.is_object() {
                *entry = Value::Object(Map::new());
            }
            if let Value::Object(inner) = entry {
                set_path(inner, rest, v);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(rules: &str, support: &str, text: &str) -> Value {
        let g = Grok::new(rules, support).unwrap();
        Value::Object(g.parse(text).expect("no rule matched").attributes)
    }

    #[test]
    fn nginx_combined() {
        let support = r#"_auth %{notSpace:http.auth:nullIf("-")}
_bytes_written %{integer:network.bytes_written}
_client_ip %{ipOrHost:network.client.ip}
_version HTTP\/%{regex("\\d+\\.\\d+"):http.version}
_url %{notSpace:http.url}
_ident %{notSpace:http.ident:nullIf("-")}
_user_agent %{regex("[^\\\"]*"):http.useragent}
_referer %{notSpace:http.referer}
_status_code %{integer:http.status_code}
_method %{word:http.method}
_date_access %{date("dd/MMM/yyyy:HH:mm:ss Z"):date_access}"#;
        let rules = r#"access.common %{_client_ip} %{_ident} %{_auth} \[%{_date_access}\] "(?>%{_method} |)%{_url}(?> %{_version}|)" %{_status_code} (?>%{_bytes_written}|-)
access.combined %{access.common} (%{number:duration:scale(1000000000)} )?"%{_referer}" "%{_user_agent}"( "%{data:_x_forwarded_for:nullIf("-")}")?.*"#;
        // `(?>…)` (atomic groups) are Java-only: the nginx pipeline is written without them.
        let rules = rules.replace("(?>", "(?:");
        let v = parse(
            &rules,
            support,
            r#"10.0.3.7 - - [16/Nov/2026:16:31:00 +0000] "GET /api/products?limit=24 HTTP/1.1" 200 48211 "-" "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36""#,
        );
        assert_eq!(v["network"]["client"]["ip"], "10.0.3.7");
        assert_eq!(v["http"]["method"], "GET");
        assert_eq!(v["http"]["status_code"], 200);
        assert_eq!(v["http"]["url"], "/api/products?limit=24");
        assert_eq!(v["http"]["version"], "1.1");
        assert_eq!(v["network"]["bytes_written"], 48211);
        assert_eq!(v["date_access"], 1794846660000i64);
        assert!(v["http"].get("auth").is_none());
    }

    #[test]
    fn matchers_and_filters() {
        let v = parse(
            r#"r %{word:user} paid %{number:amount} at %{date("yyyy-MM-dd HH:mm:ss", "Europe/Paris"):ts} %{data::keyvalue}"#,
            "",
            "alice paid 12.5 at 2026-07-01 10:00:00 status=ok code=\"A 1\" path=/x",
        );
        assert_eq!(v["user"], "alice");
        assert_eq!(v["amount"], 12.5);
        assert_eq!(v["ts"], 1782892800000i64); // 08:00 UTC (Paris is UTC+2 in July)
        assert_eq!(v["status"], "ok");
        assert_eq!(v["code"], "A 1");

        let v = parse(
            r#"r %{data:payload:json} %{notSpace:tags:array("[]", ",")} %{integer:ms:scale(0.001)}"#,
            "",
            r#"{"a":{"b":1}} [x,y] 1500"#,
        );
        assert_eq!(v["payload"]["a"]["b"], 1);
        assert_eq!(v["tags"], json!(["x", "y"]));
        assert_eq!(v["ms"], 1.5);

        let v = parse(
            r#"r %{notSpace:http.url:url} %{quotedString:q} %{boolean:ok}"#,
            "",
            r#"https://shop.example.com:8443/cart?id=7&x=a%20b "hi there" TRUE"#,
        );
        assert_eq!(v["http"]["url"]["host"], "shop.example.com");
        assert_eq!(v["http"]["url"]["port"], 8443);
        assert_eq!(v["http"]["url"]["queryString"]["x"], "a b");
        assert_eq!(v["q"], "hi there");
        assert_eq!(v["ok"], true);
    }

    #[test]
    fn first_matching_rule_wins_and_errors_are_reported() {
        let g = Grok::new("a %{integer:n}\nb %{word:w}", "").unwrap();
        assert_eq!(g.parse("42").unwrap().rule.as_deref(), Some("a"));
        assert_eq!(g.parse("abc").unwrap().rule.as_deref(), Some("b"));
        assert!(g.parse("a b").is_none());
        assert!(Grok::new("a %{nope:x}", "").is_err());
        assert!(Grok::new("a %{word:x:nofilter}", "").is_err());
        assert!(Grok::new("", "").is_err());
    }

    #[test]
    fn syslog_dates_without_year() {
        let fmt = date_pattern("MMM dd HH:mm:ss").unwrap().1;
        let ms = parse_date("Nov 16 16:31:00", &fmt, None).unwrap();
        assert!(ms > 1_700_000_000_000);
    }
}
