//! GreptimeDB through its HTTP SQL API (`POST /v1/sql`).
//!
//! Statements are built with inline literals: GreptimeDB follows standard SQL
//! quoting (a backslash is an ordinary character, `'` is doubled), which
//! [`lit`] applies; every user-supplied string goes through it.

use anyhow::{Context, Result, anyhow, bail};
use serde_json::Value;
use std::time::Duration;

pub type Row = Vec<Value>;

#[derive(Clone)]
pub struct Greptime {
    http: reqwest::Client,
    url: String,
    db: String,
    auth: Option<(String, String)>,
}

impl Greptime {
    pub fn new(base_url: &str, db: &str, auth: Option<(String, String)>) -> Result<Self> {
        Ok(Self {
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(60))
                .pool_max_idle_per_host(16)
                .build()?,
            url: format!("{}/v1/sql", base_url.trim_end_matches('/')),
            db: db.to_string(),
            auth,
        })
    }

    async fn send(&self, db: &str, sql: &str) -> Result<Value> {
        let mut req = self
            .http
            .post(&self.url)
            .query(&[("db", db)])
            .form(&[("sql", sql)]);
        if let Some((user, pass)) = &self.auth {
            req = req.basic_auth(user, Some(pass));
        }
        let resp = req.send().await.context("GreptimeDB unreachable")?;
        let status = resp.status();
        let body: Value = resp
            .json()
            .await
            .with_context(|| format!("GreptimeDB answered {status} without JSON"))?;
        if let Some(err) = body.get("error").and_then(Value::as_str) {
            let head: String = sql.chars().take(300).collect();
            bail!("GreptimeDB: {err} (in: {head})");
        }
        if !status.is_success() {
            bail!("GreptimeDB answered {status}");
        }
        Ok(body)
    }

    /// Rows of the statement's result set, each a JSON array in column order.
    pub async fn query(&self, sql: &str) -> Result<Vec<Row>> {
        let body = self.send(&self.db, sql).await?;
        let rows = body
            .pointer("/output/0/records/rows")
            .ok_or_else(|| anyhow!("GreptimeDB returned no rows"))?;
        Ok(serde_json::from_value(rows.clone())?)
    }

    pub async fn exec(&self, sql: &str) -> Result<()> {
        self.send(&self.db, sql).await.map(|_| ())
    }

    /// Creates the database and the tables, and applies the retention periods.
    pub async fn init(&self, logs_ttl_days: i64, metrics_ttl_days: i64) -> Result<()> {
        self.send(
            "public",
            &format!("CREATE DATABASE IF NOT EXISTS {}", ident(&self.db)),
        )
        .await?;
        for ddl in SCHEMA.iter().chain(UPGRADES) {
            self.exec(ddl).await?;
        }
        for (table, days) in [
            ("logs", logs_ttl_days),
            ("spans", logs_ttl_days),
            ("metric_points", metrics_ttl_days),
        ] {
            self.exec(&format!("ALTER TABLE {table} SET 'ttl' = '{days}d'"))
                .await?;
        }
        Ok(())
    }
}

/// Telemetry tables. Rows only ever get appended; the TTL drops expired data.
const SCHEMA: &[&str] = &[
    r#"CREATE TABLE IF NOT EXISTS "logs" (
        "ts" TIMESTAMP(3) TIME INDEX,
        "id" BIGINT SKIPPING INDEX WITH (type = 'BLOOM', granularity = 1024),
        "status" STRING INVERTED INDEX,
        "service" STRING INVERTED INDEX,
        "host" STRING INVERTED INDEX,
        "source" STRING INVERTED INDEX,
        "message" STRING FULLTEXT INDEX WITH (backend = 'bloom', analyzer = 'English', case_sensitive = 'false'),
        "tags" STRING,
        "attributes" JSON2,
        "trace_id" STRING SKIPPING INDEX WITH (type = 'BLOOM', granularity = 1024),
        "span_id" STRING
    ) WITH ('append_mode' = 'true')"#,
    r#"CREATE TABLE IF NOT EXISTS "spans" (
        "ts" TIMESTAMP(9) TIME INDEX,
        "id" BIGINT,
        "trace_id" STRING SKIPPING INDEX WITH (type = 'BLOOM', granularity = 1024),
        "span_id" STRING,
        "parent_id" STRING,
        "service" STRING INVERTED INDEX,
        "name" STRING INVERTED INDEX,
        "resource" STRING INVERTED INDEX,
        "type" STRING,
        "env" STRING INVERTED INDEX,
        "version" STRING,
        "duration" BIGINT,
        "error" BOOLEAN,
        "top_level" BOOLEAN INVERTED INDEX,
        "meta" JSON2,
        "metrics" JSON2
    ) WITH ('append_mode' = 'true')"#,
    r#"CREATE TABLE IF NOT EXISTS "metric_points" (
        "ts" TIMESTAMP(3) TIME INDEX,
        "metric" STRING,
        "host" STRING,
        "tags" STRING,
        "value" DOUBLE,
        "weight" DOUBLE DEFAULT 1,
        PRIMARY KEY ("metric", "host", "tags")
    ) WITH ('append_mode' = 'true')"#,
];

/// Columns added after a table was first created; each statement is idempotent.
const UPGRADES: &[&str] =
    &[r#"ALTER TABLE "metric_points" ADD COLUMN IF NOT EXISTS "weight" DOUBLE DEFAULT 1"#];

/// A string literal: `it's` → `'it''s'`. NUL bytes are dropped.
pub fn lit(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for ch in s.chars() {
        match ch {
            '\'' => out.push_str("''"),
            '\0' => {}
            c => out.push(c),
        }
    }
    out.push('\'');
    out
}

pub fn opt_lit(s: Option<&str>) -> String {
    s.map(lit).unwrap_or_else(|| "NULL".into())
}

/// A quoted identifier.
pub fn ident(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

/// A float literal (non-finite values become NULL).
pub fn num(v: f64) -> String {
    if v.is_finite() {
        format!("{v:?}")
    } else {
        "NULL".into()
    }
}

/// A JSONPath for `json_get` with every segment bracketed, so dots, dashes and
/// leading digits inside a key are literal: `["http", "status_code"]` →
/// `'$["http"]["status_code"]'`.
pub fn json_path<'a>(segments: impl IntoIterator<Item = &'a str>) -> String {
    let mut p = String::from("$");
    for seg in segments {
        p.push_str("[\"");
        p.push_str(&seg.replace('\\', "\\\\").replace('"', "\\\""));
        p.push_str("\"]");
    }
    lit(&p)
}

/// Comma-delimited tag list as stored: `["env:prod", "team:a"]` → `,env:prod,team:a,`.
pub fn tags_column(tags: &[String]) -> String {
    let mut out = String::from(",");
    for t in tags {
        let t = t.replace(',', "_");
        if !t.is_empty() {
            out.push_str(&t);
            out.push(',');
        }
    }
    out
}

pub fn parse_tags_column(s: &str) -> Vec<String> {
    s.split(',')
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .collect()
}

// Accessors for result cells.

pub fn i64_at(r: &Row, i: usize) -> i64 {
    match r.get(i) {
        Some(Value::Number(n)) => n
            .as_i64()
            .or_else(|| n.as_f64().map(|f| f as i64))
            .unwrap_or(0),
        Some(Value::Bool(b)) => *b as i64,
        Some(Value::String(s)) => s.parse().unwrap_or(0),
        _ => 0,
    }
}

pub fn f64_at(r: &Row, i: usize) -> Option<f64> {
    match r.get(i) {
        Some(Value::Number(n)) => n.as_f64(),
        Some(Value::String(s)) => s.parse().ok(),
        _ => None,
    }
}

pub fn bool_at(r: &Row, i: usize) -> bool {
    match r.get(i) {
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_i64().unwrap_or(0) != 0,
        _ => false,
    }
}

pub fn str_at(r: &Row, i: usize) -> Option<String> {
    match r.get(i) {
        Some(Value::String(s)) => Some(s.clone()),
        Some(Value::Null) | None => None,
        Some(other) => Some(other.to_string()),
    }
}

/// A JSON2 cell: GreptimeDB returns the object itself.
pub fn json_at(r: &Row, i: usize) -> Value {
    match r.get(i) {
        Some(Value::String(s)) => serde_json::from_str(s).unwrap_or(Value::Null),
        Some(Value::Null) | None => Value::Object(Default::default()),
        Some(v) => v.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literals_escape_quotes_only() {
        assert_eq!(lit("it's"), "'it''s'");
        assert_eq!(lit(r"a\'b"), r"'a\''b'");
        assert_eq!(lit("x\0y"), "'xy'");
        assert_eq!(ident("a\"b"), "\"a\"\"b\"");
    }

    #[test]
    fn json_paths_bracket_every_segment() {
        assert_eq!(
            json_path(["http", "status_code"]),
            r#"'$["http"]["status_code"]'"#
        );
        assert_eq!(json_path(["_dd.hostname"]), r#"'$["_dd.hostname"]'"#);
        assert_eq!(json_path(["o'k"]), r#"'$["o''k"]'"#);
    }

    #[test]
    fn tag_lists_round_trip() {
        let tags = vec!["env:prod".to_string(), "a,b".to_string()];
        assert_eq!(tags_column(&tags), ",env:prod,a_b,");
        assert_eq!(parse_tags_column(",env:prod,a_b,"), vec!["env:prod", "a_b"]);
        assert_eq!(tags_column(&[]), ",");
    }
}
