---
title: Cloud SIEM
section: Security
order: 10
app: /security/signals
description: Detection rules run every minute on your logs and open security signals, which you triage, assign, turn into cases or hand to workflows.
---

# Cloud SIEM

Cloud SIEM watches your logs for threats. A **detection rule** is a log
query with a condition: when the logs that match it cross a threshold, or
show a value never seen before, the rule opens a **security signal**. You
triage signals in [Signals](/security/signals), and manage rules in
[Detection Rules](/security/rules).

Rules read every log Robine Dog stores, so anything that reaches the logs
intake can be detected: application and system logs, the attacks of
[App and API Protection](app-and-api-protection.md) (`source:appsec`), the
runtime events and findings of [Workload Protection](workload-protection.md)
(`source:runtime-security-agent`, `source:compliance-agent`).

## How rules are evaluated

Every minute, Robine Dog runs each enabled rule over the last
**evaluation window** (5 minutes by default) of logs.

| Detection | What it does |
| --- | --- |
| Threshold on a count | Counts the matching logs, per group of the *group by* fields, and opens a signal for each group whose count meets a case. |
| New value of an attribute | Lists the values of an attribute (per group) in the window, and opens a signal for each value that did not appear during the **learning period** before the window (7 days by default). |

For a new value rule, nothing is reported while the learning period holds
no matching log at all, so a fresh rule does not flag every value as new.

**Cases** turn a count into a severity. They are read in order and the
first one that holds wins: with `high: a > 20` then `medium: a > 5`, a
count of 30 is `high`, 8 is `medium` and 3 opens nothing. `a` is the count
of the group (for a new value rule, the count of the new value). A case
with an empty condition always holds. Severities are `critical`, `high`,
`medium`, `low` and `info`.

### Signals are grouped

A rule opens one signal per group. While the group keeps triggering, the
same signal is updated (last seen, highest value, latest severity) instead
of a new one being opened, as long as:

- it triggered again within **keep alive** (1 hour by default), and
- the signal is younger than the **maximum signal duration** (24 hours by default).

Past either limit, the next trigger opens a new signal. An archived signal
that keeps triggering is updated but stays archived.

## Default rules

Robine Dog creates these rules at start-up when they are missing. Disable a
default rule rather than delete it: a deleted one comes back at the next
restart.

| Rule | Logs | Detection | Cases |
| --- | --- | --- | --- |
| SSH brute force | `"Failed password" OR "authentication failure"` | count by `@network.client.ip` | `high: a > 20`, `medium: a > 5` |
| Attacks from one address | `source:appsec` | count by `@network.client.ip` | `high: a > 50`, `medium: a > 10` |
| Burst of unauthorized requests | `@http.status_code:(401 OR 403)` | count by `@network.client.ip` | `medium: a > 50` |
| New user logging in as admin | `@http.url_details.path:/admin*` | new value of `@usr.id`, 7 days of learning | `low` (always) |

All four use a 5-minute window.

## Create a rule

In [Detection Rules](/security/rules), click **New Rule**:

| Field | Meaning |
| --- | --- |
| Name | Shown on every signal of the rule. |
| Logs matching | A query in the [search syntax](search-syntax.md), as in the [Log Explorer](log-explorer.md). |
| Detection | *Threshold on a count* or *New value of an attribute*. |
| Attribute | For a new value: the attribute to watch, such as `@usr.id`. |
| Group by | Comma-separated facets, such as `@network.client.ip, service`. Up to 1,000 groups per field are evaluated. |
| Cases | One per line, `severity: condition`, such as `high: a > 20`. Operators: `>`, `>=`, `<`, `<=`, `==`. |
| Evaluation window | In minutes. |
| Message | Shown on the signal. Name `@workflow-<name>` here to start that workflow on each new signal. |

The list shows each rule's query, detection, cases and an **Enabled**
checkbox.

## Triage signals

[Signals](/security/signals) lists the signals active in the time range,
newest first, with their severity, rule, group, value, status, assignee
and last-seen time. Filter by status and severity. Click a signal to open
its panel:

- **Under review**, **Archive (false positive)**, **Archive (benign)** and **Reopen** set its status (`open`, `under_review`, `archived`, with an archive reason).
- **Assign** sets a free-text assignee.
- **View logs** opens the Log Explorer on the rule's query and the signal's group, from 10 minutes before the signal to a minute after its last trigger.
- **Create case** opens a [case](case-management.md) in the `SEC` project, titled `<rule>: <group>`, with the rule's message as description, priority `P2` for critical and high signals and `P3` otherwise, linked back to the signal.

## Respond automatically

- A new signal starts every enabled [workflow](workflows.md) triggered by security signals (all of them, or those of one rule), and the workflows named `@workflow-<name>` in the rule's message.
- Each new signal adds 1 to the `datadog.security.signals` metric, tagged `rule_id`, `severity` and the group's fields (without `@`): a [monitor](monitors.md) on it can notify anyone.

## API

Send `DD-API-KEY` (and, as Datadog clients do, `DD-APPLICATION-KEY`).

| Method and path | Does |
| --- | --- |
| `GET /api/v2/security_monitoring/rules` | Lists rules (`data`, with `id`, `isDefault`, `createdAt`). |
| `POST /api/v2/security_monitoring/rules` | Creates a rule. |
| `GET`, `PUT`, `DELETE /api/v2/security_monitoring/rules/{id}` | Reads, updates or deletes a rule. On `PUT`, the top-level fields you send replace the stored ones and the others are kept, so `{"isEnabled": false}` disables a rule without touching its options. |
| `GET /api/v2/security_monitoring/signals` | Lists signals: `from`, `to` (epoch ms, default the last 7 days), `status`, `severity`, `query` (part of the rule name or group). Also `filter[from]`, `filter[to]`, `filter[query]`. At most 500, newest first. |
| `PATCH /api/v2/security_monitoring/signals/{id}/state` | `{"data": {"attributes": {"state": "archived", "archive_reason": "false_positive"}}}` |
| `PATCH /api/v2/security_monitoring/signals/{id}/assignee` | `{"data": {"attributes": {"assignee": {"uuid": "alice@example.com"}}}}` |

A rule uses Datadog's shape; only the first query is read:

```sh
curl -X POST http://localhost:8080/api/v2/security_monitoring/rules \
  -H "DD-API-KEY: $ROBINE_DOG_API_KEY" \
  -H "DD-APPLICATION-KEY: $ROBINE_DOG_APP_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "name": "Password spraying",
    "type": "log_detection",
    "message": "One address failed to log in as many users. @workflow-block-ip",
    "queries": [{"name": "a", "query": "service:auth @evt.outcome:failure",
      "groupByFields": ["@network.client.ip"], "aggregation": "count"}],
    "cases": [{"name": "spray", "status": "high", "condition": "a > 30"}],
    "options": {"evaluationWindow": 600, "keepAlive": 3600, "maxSignalDuration": 86400}
  }'
```

For a new value rule, set `"aggregation": "new_value"`, `"metrics":
["@usr.id"]` in the query and `"newValueOptions": {"learningDuration": 7}`
(days) in `options`. `evaluationWindow`, `keepAlive` and
`maxSignalDuration` are in seconds.

## Differences from Datadog

- Rules run on logs only (`log_detection`), with two detections: count and new value. Other aggregations (cardinality, sum, max), anomaly, impossible travel, third-party and signal correlation rules are not implemented.
- One query per rule; cases compare that query's count (`a`).
- The signals API takes epoch milliseconds and its `query` matches part of the rule name or group, not the search syntax.
- Signals are not suppressed and have no notification targets of their own: use a workflow or a monitor on `datadog.security.signals`.
