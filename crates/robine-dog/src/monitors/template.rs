//! Monitor messages, with Datadog's template syntax: conditional sections
//! (`{{#is_alert}}…{{/is_alert}}`, `{{^is_recovery}}…{{/is_recovery}}`,
//! `{{#is_match "host.name" "web"}}…{{/is_match}}`) and variables
//! (`{{value}}`, `{{threshold}}`, `{{warn_threshold}}`, `{{comparator}}`,
//! `{{last_triggered_at}}`, `{{host.name}}` and any `{{<group key>.name}}`).

use super::{ALERT, MonitorDef, NO_DATA, OK, Parsed, WARN, fmt_value};
use std::collections::BTreeMap;

pub struct Context<'a> {
    pub def: &'a MonitorDef,
    pub parsed: &'a Parsed,
    pub group: &'a str,
    pub tags: &'a BTreeMap<String, String>,
    pub status: &'a str,
    pub previous: &'a str,
    pub transition: &'a str,
    pub value: Option<f64>,
    pub renotify: bool,
    pub last_triggered: Option<i64>,
}

fn utc(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|d| d.format("%Y-%m-%d %H:%M:%S UTC").to_string())
        .unwrap_or_default()
}

impl Context<'_> {
    /// `[P2] [Triggered on {host:a}] Name`, as Datadog titles its events.
    pub fn title(&self) -> String {
        let scope = if self.group == "*" {
            String::new()
        } else {
            format!(" on {{{}}}", self.group)
        };
        let priority = self
            .def
            .priority
            .map(|p| format!("[P{p}] "))
            .unwrap_or_default();
        format!("{priority}[{}{scope}] {}", self.transition, self.def.name)
    }

    fn flag(&self, name: &str, args: &[String]) -> bool {
        let recovering = self.status == OK && self.previous != OK;
        match name {
            "is_alert" => self.status == ALERT,
            "is_warning" => self.status == WARN,
            "is_no_data" => self.status == NO_DATA,
            "is_recovery" => recovering,
            "is_alert_recovery" => recovering && self.previous == ALERT,
            "is_warning_recovery" => recovering && self.previous == WARN,
            "is_no_data_recovery" => recovering && self.previous == NO_DATA,
            "is_alert_to_warning" => self.previous == ALERT && self.status == WARN,
            "is_warning_to_alert" => self.previous == WARN && self.status == ALERT,
            "is_renotify" => self.renotify,
            "is_match" | "is_exact_match" => {
                let Some((var, values)) = args.split_first() else {
                    return false;
                };
                let actual = self.variable(var).unwrap_or_default();
                values.iter().any(|v| {
                    if name == "is_exact_match" {
                        actual == *v
                    } else {
                        actual.contains(v.as_str())
                    }
                })
            }
            _ => false,
        }
    }

    fn variable(&self, name: &str) -> Option<String> {
        let th = &self.def.options.thresholds;
        let num = |v: Option<f64>| v.map(|v| fmt_value(Some(v)));
        Some(match name {
            "value" => fmt_value(self.value),
            "threshold" => fmt_value(Some(self.parsed.threshold)),
            "warn_threshold" => num(th.warning)?,
            "alert_recovery_threshold" => num(th.critical_recovery)?,
            "warn_recovery_threshold" => num(th.warning_recovery)?,
            "comparator" => self.parsed.cmp.symbol().to_string(),
            "last_triggered_at" => utc(self.last_triggered?),
            "last_triggered_at_epoch" => self.last_triggered?.to_string(),
            "monitor_id" | "monitor.id" => self.def.id?.to_string(),
            "monitor_name" | "monitor.name" => self.def.name.clone(),
            "group" => self.group.to_string(),
            other => {
                let key = other.strip_suffix(".name")?;
                let key = key.trim_start_matches('[').trim_end_matches(']');
                self.tags.get(key)?.clone()
            }
        })
    }
}

/// `name "a" "b"` → (`name`, [`a`, `b`]).
fn split_tag(inner: &str) -> (String, Vec<String>) {
    let inner = inner.trim();
    let (name, rest) = inner.split_once(char::is_whitespace).unwrap_or((inner, ""));
    let mut args = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    for c in rest.chars() {
        match c {
            '"' if quoted => {
                args.push(std::mem::take(&mut cur));
                quoted = false;
            }
            '"' => quoted = true,
            c if quoted => cur.push(c),
            _ => {}
        }
    }
    (name.to_string(), args)
}

/// Renders a template against a context.
pub fn render(template: &str, ctx: &Context) -> String {
    let mut out = String::new();
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else {
            out.push_str(&rest[start..]);
            return out;
        };
        let tag = after[..end].trim();
        rest = &after[end + 2..];
        match tag.chars().next() {
            Some(c @ ('#' | '^')) => {
                let (name, args) = split_tag(&tag[1..]);
                // Find the matching close tag, allowing nested sections of the same name.
                let open = format!("{{{{#{name}");
                let open_inv = format!("{{{{^{name}");
                let close = format!("{{{{/{name}}}}}");
                let mut depth = 1;
                let mut i = 0;
                let mut body_end = None;
                while i < rest.len() {
                    let r = &rest[i..];
                    if r.starts_with(&close) {
                        depth -= 1;
                        if depth == 0 {
                            body_end = Some(i);
                            break;
                        }
                        i += close.len();
                    } else if r.starts_with(&open) || r.starts_with(&open_inv) {
                        depth += 1;
                        i += 2;
                    } else {
                        i += r.chars().next().map(char::len_utf8).unwrap_or(1);
                    }
                }
                let Some(body_end) = body_end else {
                    continue;
                };
                let body = &rest[..body_end];
                rest = &rest[body_end + close.len()..];
                if ctx.flag(&name, &args) == (c == '#') {
                    out.push_str(&render(body, ctx));
                }
            }
            Some('/') => {}
            _ => out.push_str(&ctx.variable(tag).unwrap_or_default()),
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::super::parse;
    use super::*;
    use serde_json::json;

    #[test]
    fn sections_and_variables() {
        let mut def: MonitorDef = serde_json::from_value(json!({
            "id": 1001, "name": "High CPU", "type": "metric alert", "priority": 2,
            "query": "avg(last_5m):avg:cpu{*} by {host} > 80",
            "options": {"thresholds": {"critical": 80, "warning": 60}}
        }))
        .unwrap();
        def.message = "{{#is_alert}}CPU is {{value}} on {{host.name}} (> {{threshold}}){{/is_alert}}{{#is_warning}}warn {{warn_threshold}}{{/is_warning}}{{^is_alert}} not alert{{/is_alert}}{{#is_recovery}}back to normal{{/is_recovery}}{{#is_match \"host.name\" \"web\"}} [web]{{/is_match}} @webhook-ops".into();
        let p = parse(&def.query).unwrap();
        let tags = BTreeMap::from([("host".to_string(), "web-1".to_string())]);
        let mut ctx = Context {
            def: &def,
            parsed: &p,
            group: "host:web-1",
            tags: &tags,
            status: ALERT,
            previous: OK,
            transition: "Triggered",
            value: Some(91.5),
            renotify: false,
            last_triggered: None,
        };
        assert_eq!(
            render(&def.message, &ctx),
            "CPU is 91.5 on web-1 (> 80) [web] @webhook-ops"
        );
        assert_eq!(ctx.title(), "[P2] [Triggered on {host:web-1}] High CPU");
        ctx.status = OK;
        ctx.previous = ALERT;
        ctx.transition = "Recovered";
        assert_eq!(
            render(&def.message, &ctx),
            " not alertback to normal [web] @webhook-ops"
        );
        ctx.status = WARN;
        assert_eq!(
            render("{{#is_warning}}w={{warn_threshold}}{{/is_warning}}", &ctx),
            "w=60"
        );
    }
}
