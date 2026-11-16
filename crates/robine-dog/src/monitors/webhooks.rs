//! Webhooks, as Datadog's Webhooks integration: named endpoints a monitor
//! message reaches with `@webhook-<name>`, a payload template with `$`
//! variables (`$EVENT_TITLE`, `$ALERT_TRANSITION`, `$LINK`…), custom headers,
//! JSON or form encoding. API: `/api/v1/integration/webhooks/configuration/webhooks`.

use super::template::Context;
use super::{ALERT, NO_DATA, OK, Source, WARN};
use crate::AppState;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::sync::LazyLock;
use std::time::Duration;

pub const KIND: &str = "webhook";

pub const DEFAULT_PAYLOAD: &str = r#"{"body": "$EVENT_MSG", "last_updated": "$LAST_UPDATED", "event_type": "$EVENT_TYPE", "title": "$EVENT_TITLE", "date": "$DATE", "org": {"id": "$ORG_ID", "name": "$ORG_NAME"}, "id": "$ID"}"#;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WebhookDef {
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub payload: Option<String>,
    /// A JSON object, as a string (Datadog's format).
    #[serde(default)]
    pub custom_headers: Option<String>,
    /// `json` (default) or `form`.
    #[serde(default = "json_encoding")]
    pub encode_as: String,
}

fn json_encoding() -> String {
    "json".into()
}

static HANDLE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"@webhook-([A-Za-z0-9_\-.]+)").unwrap());
static CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .user_agent("robine-dog-webhooks")
        .build()
        .expect("http client")
});

pub fn valid_name(n: &str) -> bool {
    !n.is_empty()
        && n.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
}

/// Replaces `$VARS` (longest names first); in JSON payloads values are escaped.
pub fn substitute(template: &str, vars: &[(&str, String)], json: bool) -> String {
    let mut vars: Vec<&(&str, String)> = vars.iter().collect();
    vars.sort_by_key(|(k, _)| std::cmp::Reverse(k.len()));
    let mut out = template.to_string();
    for (k, v) in vars {
        let value = if json {
            let s = serde_json::to_string(v).unwrap_or_default();
            s[1..s.len() - 1].to_string()
        } else {
            v.clone()
        };
        out = out.replace(&format!("${k}"), &value);
    }
    out
}

pub fn variables(
    st: &AppState,
    ctx: &Context,
    title: &str,
    message: &str,
) -> Vec<(&'static str, String)> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let id = ctx.def.id.unwrap_or_default();
    let alert_type = match ctx.status {
        ALERT => "error",
        WARN => "warning",
        NO_DATA => "info",
        OK => "success",
        _ => "info",
    };
    let metric = match &ctx.parsed.source {
        Source::Metric { query, .. } => query.metric.clone(),
        Source::Events { .. } => String::new(),
    };
    let tags: Vec<String> = ctx
        .def
        .tags
        .iter()
        .cloned()
        .chain(ctx.tags.iter().map(|(k, v)| format!("{k}:{v}")))
        .collect();
    let status = format!(
        "{} {} {} ({}: {})",
        if ctx.group == "*" { "Value" } else { ctx.group },
        ctx.parsed.cmp.symbol(),
        super::fmt_value(Some(ctx.parsed.threshold)),
        ctx.status,
        super::fmt_value(ctx.value)
    );
    vec![
        ("ALERT_ID", id.to_string()),
        ("ALERT_TITLE", title.to_string()),
        ("ALERT_TYPE", alert_type.to_string()),
        ("ALERT_TRANSITION", ctx.transition.to_string()),
        ("ALERT_STATUS", status),
        ("ALERT_QUERY", ctx.def.query.clone()),
        ("ALERT_METRIC", metric),
        (
            "ALERT_SCOPE",
            if ctx.group == "*" {
                String::new()
            } else {
                ctx.group.to_string()
            },
        ),
        (
            "ALERT_PRIORITY",
            ctx.def
                .priority
                .map(|p| format!("P{p}"))
                .unwrap_or_default(),
        ),
        ("ALERT_CYCLE_KEY", format!("{id}-{}", ctx.group)),
        ("EVENT_TITLE", title.to_string()),
        ("EVENT_MSG", message.to_string()),
        ("TEXT_ONLY_MSG", message.to_string()),
        (
            "EVENT_TYPE",
            format!("{}_monitor", ctx.def.typ.replace(' ', "_")),
        ),
        ("DATE", now.to_string()),
        ("LAST_UPDATED", now.to_string()),
        ("ID", now.to_string()),
        ("AGGREG_KEY", format!("{id}")),
        (
            "LINK",
            format!("{}/monitors/{id}", st.cfg.base_url.trim_end_matches('/')),
        ),
        (
            "HOSTNAME",
            ctx.tags.get("host").cloned().unwrap_or_default(),
        ),
        ("TAGS", tags.join(",")),
        ("PRIORITY", "normal".into()),
        ("ORG_ID", "1".into()),
        ("ORG_NAME", "robine-dog".into()),
        ("USER", String::new()),
        ("USERNAME", String::new()),
        ("SNAPSHOT", String::new()),
    ]
}

/// Posts one webhook; the HTTP status on success.
pub async fn send(hook: &WebhookDef, vars: &[(&str, String)]) -> anyhow::Result<u16> {
    let json = hook.encode_as != "form";
    let template = hook
        .payload
        .as_deref()
        .filter(|p| !p.trim().is_empty())
        .unwrap_or(DEFAULT_PAYLOAD);
    let body = substitute(template, vars, true);
    let mut req = CLIENT.post(&hook.url);
    if let Some(h) = hook
        .custom_headers
        .as_deref()
        .filter(|h| !h.trim().is_empty())
    {
        let headers: Map<String, Value> = serde_json::from_str(&substitute(h, vars, true))
            .map_err(|e| anyhow::anyhow!("custom_headers: {e}"))?;
        for (k, v) in headers {
            req = req.header(
                k,
                v.as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| v.to_string()),
            );
        }
    }
    req = if json {
        req.header("content-type", "application/json").body(body)
    } else {
        let fields: Vec<(String, String)> = match serde_json::from_str::<Value>(&body) {
            Ok(Value::Object(o)) => o
                .into_iter()
                .map(|(k, v)| {
                    (
                        k,
                        v.as_str()
                            .map(str::to_string)
                            .unwrap_or_else(|| v.to_string()),
                    )
                })
                .collect(),
            _ => vec![("payload".into(), body)],
        };
        let encoded: String = fields
            .iter()
            .map(|(k, v)| {
                format!(
                    "{}={}",
                    percent_encoding::utf8_percent_encode(k, percent_encoding::NON_ALPHANUMERIC),
                    percent_encoding::utf8_percent_encode(v, percent_encoding::NON_ALPHANUMERIC)
                )
            })
            .collect::<Vec<_>>()
            .join("&");
        req.header("content-type", "application/x-www-form-urlencoded")
            .body(encoded)
    };
    let res = req.send().await?;
    let status = res.status();
    if !status.is_success() {
        anyhow::bail!("HTTP {status}");
    }
    Ok(status.as_u16())
}

pub async fn load(st: &AppState, name: &str) -> anyhow::Result<Option<WebhookDef>> {
    Ok(st
        .store
        .pg
        .doc(KIND, name)
        .await?
        .and_then(|d| serde_json::from_value(d.body).ok()))
}

/// Sends the notification to every `@webhook-…` handle of the message;
/// returns the handles reached (failures are marked).
pub async fn notify(st: &AppState, ctx: &Context<'_>, title: &str, message: &str) -> Vec<String> {
    let mut names: Vec<String> = HANDLE
        .captures_iter(message)
        .map(|c| c[1].to_string())
        .collect();
    names.dedup();
    let vars = variables(st, ctx, title, message);
    let mut out = Vec::new();
    for name in names {
        let result = match load(st, &name).await {
            Ok(Some(hook)) => send(&hook, &vars).await.map(|_| ()),
            Ok(None) => Err(anyhow::anyhow!("no webhook named {name}")),
            Err(e) => Err(e),
        };
        match result {
            Ok(()) => out.push(format!("@webhook-{name}")),
            Err(e) => {
                tracing::warn!(webhook = %name, error = %e, "webhook failed");
                out.push(format!("@webhook-{name} (failed: {e})"));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn substitution_escapes_json() {
        let vars = vec![
            ("EVENT_TITLE", "a \"b\"".to_string()),
            ("EVENT_MSG", "line1\nline2".to_string()),
            ("EVENT", "x".to_string()),
        ];
        let out = substitute(r#"{"t": "$EVENT_TITLE", "m": "$EVENT_MSG"}"#, &vars, true);
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["t"], "a \"b\"");
        assert_eq!(v["m"], "line1\nline2");
        assert!(valid_name("ops-alerts_1"));
        assert!(!valid_name("a b"));
    }
}
