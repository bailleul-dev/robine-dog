//! Events (Datadog's Event Management): things that happened — a monitor
//! changed state, an Agent started, a deploy was announced through the API.
//!
//! They are stored like logs, in a table of their own with the same columns,
//! so the same search syntax applies: `status` is the alert type (`error`,
//! `warn`, `info`, `ok`), `source` the source type, `message` the title and
//! the text, and the rest is under `@evt`.

use crate::AppState;
use crate::intake::logs::{LogRow, parse_timestamp_ms};
use crate::store::writer::Insertable;
use serde_json::{Map, Value, json};

/// A [`LogRow`] bound for the `events` table.
pub struct EventRow(pub LogRow);

impl Insertable for EventRow {
    const TABLE: &'static str = "events";
    const COLUMNS: &'static str = <LogRow as Insertable>::COLUMNS;

    fn values(&self, out: &mut String) {
        self.0.values(out)
    }
}

/// Datadog's alert types as statuses.
fn status(alert_type: &str) -> &'static str {
    match alert_type.to_lowercase().as_str() {
        "error" => "error",
        "warning" | "warn" => "warn",
        "success" => "ok",
        _ => "info",
    }
}

fn text(v: Option<&Value>) -> Option<String> {
    match v? {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// An event as the v1 API (`title`, `text`, `date_happened`…) or an Agent
/// (`msg_title`, `msg_text`, `timestamp`…) describes it.
pub fn from_json(v: &Value, default_source: Option<&str>, now: i64) -> Option<LogRow> {
    let title = text(v.get("title")).or_else(|| text(v.get("msg_title")))?;
    let body = text(v.get("text"))
        .or_else(|| text(v.get("msg_text")))
        .unwrap_or_default();
    let tags: Vec<String> = v
        .get("tags")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(String::from)
                .collect()
        })
        .unwrap_or_default();
    // Seconds, as Datadog's API takes them.
    let ts = v
        .get("date_happened")
        .or_else(|| v.get("timestamp"))
        .and_then(parse_timestamp_ms)
        .unwrap_or(now);
    let mut evt = Map::new();
    evt.insert("title".into(), json!(title));
    if !body.is_empty() {
        evt.insert("text".into(), json!(body));
    }
    for k in ["priority", "aggregation_key", "event_type", "device_name"] {
        if let Some(x) = text(v.get(k)) {
            evt.insert(k.into(), json!(x));
        }
    }
    let service = tags
        .iter()
        .find_map(|t| t.strip_prefix("service:"))
        .map(String::from);
    Some(LogRow {
        ts,
        status: status(&text(v.get("alert_type")).unwrap_or_default()).into(),
        service,
        host: text(v.get("host")),
        source: text(v.get("source_type_name"))
            .or_else(|| default_source.map(String::from))
            .or_else(|| Some("api".into())),
        message: if body.is_empty() {
            title
        } else {
            format!("{title}\n{body}")
        },
        tags,
        attributes: Map::from_iter([("evt".to_string(), Value::Object(evt))]),
        trace_id: None,
        span_id: None,
    })
}

pub async fn emit(st: &AppState, mut rows: Vec<LogRow>) {
    for r in &mut rows {
        st.hosts.apply(&mut r.host, &mut r.tags);
    }
    let rows = rows.into_iter().map(EventRow).collect();
    if let Err(e) = st.store.events.send(rows).await {
        tracing::warn!(error = %e, "storing events");
    }
}

/// The events of an Agent's payload: `{"events": {"<source>": [event, …]}}`.
pub fn from_agent(payload: &Value, now: i64) -> Vec<LogRow> {
    payload
        .get("events")
        .and_then(Value::as_object)
        .map(|sources| {
            sources
                .iter()
                .flat_map(|(source, list)| {
                    list.as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(move |e| from_json(e, Some(source), now))
                })
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_api_and_agent_events() {
        let e = from_json(
            &json!({"title": "Deployed shop 1.4.2", "text": "by ci", "date_happened": 1794910470,
                    "alert_type": "success", "tags": ["service:shop", "env:prod"], "priority": "low"}),
            None,
            0,
        )
        .unwrap();
        assert_eq!(e.ts, 1794910470000);
        assert_eq!(
            (e.status.as_str(), e.source.as_deref()),
            ("ok", Some("api"))
        );
        assert_eq!(e.service.as_deref(), Some("shop"));
        assert_eq!(e.message, "Deployed shop 1.4.2\nby ci");
        assert_eq!(e.attributes["evt"]["priority"], "low");

        let agent = from_agent(
            &json!({"events": {"docker": [{"msg_title": "container died", "alert_type": "error", "host": "a"}]}}),
            7,
        );
        assert_eq!(agent.len(), 1);
        assert_eq!(agent[0].source.as_deref(), Some("docker"));
        assert_eq!((agent[0].status.as_str(), agent[0].ts), ("error", 7));
        assert!(from_json(&json!({"text": "no title"}), None, 0).is_none());
    }
}
