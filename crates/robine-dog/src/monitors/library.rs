//! Monitor Templates (a built-in library to start from) and Monitor Quality
//! (monitors that probably need attention).

use crate::AppState;
use crate::api::ApiResult;
use axum::Json;
use axum::extract::{Query, State};
use serde_json::{Value, json};

/// (id, category, name, type, query, message)
const TEMPLATES: &[(&str, &str, &str, &str, &str, &str)] = &[
    (
        "host-cpu",
        "Infrastructure",
        "High CPU on {{host.name}}",
        "metric alert",
        "avg(last_10m):avg:system.cpu.user{*} by {host} > 90",
        "CPU has been above 90% for 10 minutes on {{host.name}}.",
    ),
    (
        "host-disk",
        "Infrastructure",
        "Disk almost full on {{host.name}} {{device.name}}",
        "metric alert",
        "max(last_15m):max:system.disk.in_use{*} by {host,device} > 0.9",
        "{{device.name}} on {{host.name}} is {{value}} full.",
    ),
    (
        "host-memory",
        "Infrastructure",
        "Low memory on {{host.name}}",
        "metric alert",
        "avg(last_10m):avg:system.mem.pct_usable{*} by {host} < 0.1",
        "Less than 10% of the memory is usable on {{host.name}}.",
    ),
    (
        "agent-up",
        "Infrastructure",
        "Agent down on {{host.name}}",
        "service check",
        "\"datadog.agent.up\".over(\"*\").by(\"host\").last(2).count_by_status()",
        "The Agent on {{host.name}} stopped reporting.",
    ),
    (
        "k8s-restarts",
        "Kubernetes",
        "Pods restarting in {{kube_namespace.name}}",
        "query alert",
        "change(sum(last_10m),last_10m):sum:kubernetes.containers.restarts{*} by {kube_namespace} > 3",
        "Containers restarted more than 3 times in 10 minutes in {{kube_namespace.name}}.",
    ),
    (
        "k8s-crashloop",
        "Kubernetes",
        "CrashLoopBackOff in {{kube_namespace.name}}",
        "metric alert",
        "max(last_5m):max:kubernetes_state.container.status_report.count.waiting{reason:crashloopbackoff} by {kube_namespace,pod_name} >= 1",
        "{{pod_name.name}} is in CrashLoopBackOff.",
    ),
    (
        "apm-errors",
        "APM",
        "Errors on {{service.name}}",
        "trace-analytics alert",
        "trace-analytics(\"status:error\").rollup(\"count\").by(\"service\").last(\"5m\") > 20",
        "More than 20 errors in 5 minutes on {{service.name}}.",
    ),
    (
        "apm-latency",
        "APM",
        "Slow {{service.name}}",
        "trace-analytics alert",
        "trace-analytics(\"*\").rollup(\"pc99\", \"@duration\").by(\"service\").last(\"10m\") > 2000000000",
        "p99 latency of {{service.name}} is above 2 s.",
    ),
    (
        "logs-errors",
        "Logs",
        "Error logs on {{service.name}}",
        "log alert",
        "logs(\"status:error\").index(\"*\").rollup(\"count\").by(\"service\").last(\"5m\") > 50",
        "More than 50 error logs in 5 minutes on {{service.name}}.",
    ),
    (
        "rum-errors",
        "RUM",
        "Front-end errors on {{service.name}}",
        "rum alert",
        "rum(\"@type:error\").rollup(\"count\").by(\"service\").last(\"10m\") > 20",
        "Browser errors are spiking on {{service.name}}.",
    ),
    (
        "ssl-expiry",
        "Synthetics",
        "Certificate expiring",
        "metric alert",
        "min(last_1h):min:synthetics.ssl.days_left{*} by {host} < 14",
        "The certificate of {{host.name}} expires in less than 14 days.",
    ),
];

/// `GET /api/ui/monitors/templates`.
pub async fn templates() -> Json<Value> {
    Json(
        json!(TEMPLATES.iter().map(|(id, cat, name, ty, query, message)| json!({
        "id": id, "category": cat, "name": name, "type": ty, "query": query, "message": message,
    })).collect::<Vec<_>>()),
    )
}

/// What is wrong with a monitor (as listed by `/api/v1/monitor`).
pub fn issues(m: &Value, now_s: i64) -> Vec<(&'static str, String)> {
    let mut out = Vec::new();
    let message = m["message"].as_str().unwrap_or("");
    let is_synthetic = m["type"].as_str() == Some("synthetics alert");
    if !is_synthetic && !message.contains('@') {
        out.push((
            "no_recipient",
            "notifies nobody: no @handle in the message".into(),
        ));
    }
    for (scope, end) in m["options"]["silenced"].as_object().into_iter().flatten() {
        match end.as_i64() {
            None => out.push(("muted_forever", format!("muted without an end ({scope})"))),
            Some(e) if e - now_s > 30 * 86_400 => out.push((
                "muted_long",
                format!("muted for more than 30 days ({scope})"),
            )),
            _ => {}
        }
    }
    let state = m["overall_state"].as_str().unwrap_or("");
    if state == "Alert" {
        let since = m["state"]["groups"]
            .as_object()
            .into_iter()
            .flatten()
            .filter(|(_, g)| g["status"] == "Alert")
            .filter_map(|(_, g)| g["since_ts"].as_i64())
            .min();
        if let Some(s) = since.filter(|s| now_s - s > 7 * 86_400) {
            out.push((
                "alert_too_long",
                format!(
                    "alerting for {} days: tune the threshold or fix the cause",
                    (now_s - s) / 86_400
                ),
            ));
        }
    }
    if state == "No Data" {
        out.push(("no_data", "no data: the query may match nothing".into()));
    }
    let q = m["query"].as_str().unwrap_or("");
    let cloud = ["aws.", "gcp.", "azure."]
        .iter()
        .any(|p| q.contains(&format!(":{p}")));
    if cloud && m["options"]["evaluation_delay"].as_i64().unwrap_or(0) == 0 {
        out.push((
            "no_delay",
            "cloud metric without evaluation_delay: they arrive late".into(),
        ));
    }
    out
}

/// `GET /api/ui/monitors/quality`.
pub async fn quality(State(st): State<AppState>) -> ApiResult<Value> {
    let Json(all) = crate::api::monitors::list(State(st), Query(Default::default())).await?;
    let now = crate::intake::logs::now_ms() / 1000;
    let rows: Vec<Value> = all
        .iter()
        .filter_map(|m| {
            let i = issues(m, now);
            (!i.is_empty()).then(|| {
                json!({"id": m["id"], "name": m["name"], "type": m["type"],
                "overall_state": m["overall_state"],
                "issues": i.iter().map(|(k, t)| json!({"kind": k, "text": t})).collect::<Vec<_>>()})
            })
        })
        .collect();
    Ok(Json(json!({"monitors": all.len(), "data": rows})))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn templates_parse() {
        for (id, _, _, ty, q, _) in TEMPLATES {
            let p = super::super::parse(q).unwrap_or_else(|e| panic!("{id}: {e:?}"));
            assert_eq!(super::super::type_of(&p), *ty, "{id}");
        }
    }

    #[test]
    fn quality_rules() {
        let now = 100 * 86_400;
        let m = json!({"message": "no handle", "type": "metric alert", "query": "avg(last_5m):avg:aws.ec2.cpu{*} > 1",
            "overall_state": "Alert", "options": {"silenced": {"*": null, "host:a": now + 40 * 86_400}},
            "state": {"groups": {"*": {"status": "Alert", "since_ts": now - 9 * 86_400}}}});
        let kinds: Vec<_> = issues(&m, now).into_iter().map(|i| i.0).collect();
        assert_eq!(
            kinds,
            vec![
                "no_recipient",
                "muted_forever",
                "muted_long",
                "alert_too_long",
                "no_delay"
            ]
        );
        let ok = json!({"message": "@ops@example.com", "type": "metric alert", "query": "avg(last_5m):avg:x{*} > 1",
            "overall_state": "OK", "options": {}});
        assert!(issues(&ok, now).is_empty());
    }
}
