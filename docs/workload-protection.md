---
title: Workload Protection and Cloud Security
section: Security
order: 40
app: /security/workload
description: The Datadog security-agent's runtime security events and compliance findings, sent to Robine Dog's logs intake and summed up by rule, host, process, framework and resource.
---

# Workload Protection and Cloud Security

The Datadog Agent ships a **security-agent** with two features:

- **Workload Protection** (runtime security): with system-probe, it watches process executions, file changes, network activity and kernel events on hosts and containers, and reports what its rules flag.
- **Cloud Security** (compliance): it checks hosts, containers and Kubernetes against benchmarks (CIS…) and reports each check's result.

Both send their events as logs. Point them at Robine Dog's logs intake and
they are stored like any other log: [Workload Protection](/security/workload)
and [Cloud Security](/security/cloud) sum them up, the
[Log Explorer](log-explorer.md) searches them, and [Cloud SIEM](siem.md)
rules can detect on them.

## Send the events

Set up the Agent as described in [Datadog Agent](agent.md), then enable
the two features and give each its own logs endpoint, the address of Robine
Dog's HTTP port as `host:port`:

```yaml
runtime_security_config:
  enabled: true
  endpoints:
    logs_dd_url: localhost:8080
compliance_config:
  enabled: true
  endpoints:
    logs_dd_url: localhost:8080
```

Workload Protection also needs system-probe running with
`runtime_security_config.enabled: true`, as in Datadog.

The events arrive with these sources:

| Feature | `source` |
| --- | --- |
| Workload Protection | `runtime-security-agent` |
| Cloud Security | `compliance-agent` |

## The Workload Protection page

For the selected time range (the last day by default), four top lists of
the `source:runtime-security-agent` logs:

| List | Facet |
| --- | --- |
| Rules | `@agent.rule_id` |
| Hosts | `host` |
| Processes | `@process.file.path` |
| Containers | `@container.id` |

Click an entry to open the matching events in the Log Explorer, or **All
events** for every event of the range.

## The Cloud Security page

The same layout over the `source:compliance-agent` logs:

| List | Facet |
| --- | --- |
| Results | `@result` (such as `passed`, `failed`) |
| Failing rules | `@agent_rule_id` |
| Frameworks | `@agent_framework_id` |
| Resources | `@resource_type` |

To list only the failed checks of one framework, search the Log Explorer:

```text
source:compliance-agent @result:failed @agent_framework_id:cis-kubernetes
```

## Detect on runtime events

Because the events are logs, a [Cloud SIEM](siem.md) rule can count them.
This rule opens a signal when a host reports more than five runtime
security events in 5 minutes:

```json
{
  "name": "Runtime security events on a host",
  "message": "The security-agent flagged repeated activity on this host.",
  "queries": [{"name": "a", "query": "source:runtime-security-agent",
    "groupByFields": ["host"], "aggregation": "count"}],
  "cases": [{"name": "many", "status": "high", "condition": "a > 5"}],
  "options": {"evaluationWindow": 300, "keepAlive": 3600, "maxSignalDuration": 86400}
}
```

Create it in [Detection Rules](/security/rules) or with `POST
/api/v2/security_monitoring/rules` (see [Cloud SIEM](siem.md#api)).

## Differences from Datadog

- Robine Dog stores and summarises what the security-agent sends; it does not push rules, policies or agent configuration (no Remote Configuration).
- There is no findings explorer with posture scores, misconfiguration lifecycle or remediation: compliance results are logs, one per check run.
- Cloud Security covers what the security-agent checks on hosts and Kubernetes. Agentless scanning and cloud-account (AWS, Azure, GCP) configuration scanning are not implemented.
