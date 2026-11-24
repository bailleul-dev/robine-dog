---
title: App and API Protection
section: Security
order: 20
app: /security/appsec
description: The attacks your tracers' in-app WAF detects become searchable logs and security signals, next to an inventory of your API endpoints.
---

# App and API Protection

The Datadog tracers embed a web application firewall (WAF). With it turned
on, each request that matches one of its rules (SQL injection, cross-site
scripting, scanners, path traversal…) is tagged on its trace. Robine Dog
turns every such match into a log with `source:appsec`, so attacks can be
searched in the [Log Explorer](log-explorer.md), counted by
[monitors](monitors.md) and detected by [Cloud SIEM](siem.md) rules.

The [App & API Protection](/security/appsec) page sums up the attacks and
lists the API endpoints your services serve.

## Turn on the WAF

Set `DD_APPSEC_ENABLED=true` on a service instrumented with a Datadog tracer
that sends its traces to Robine Dog (see [APM tracers](tracing.md)):

```sh
DD_APPSEC_ENABLED=true \
DD_SERVICE=shop-api DD_ENV=prod DD_VERSION=1.4.2 \
DD_TRACE_AGENT_URL=http://localhost:8126 \
ddtrace-run gunicorn shop.wsgi
```

Nothing else is needed on the Robine Dog side: the WAF runs in your
application, and its findings travel with the traces.

## How attacks are recorded

When a span carries the WAF's report (the `_dd.appsec.json` tag), Robine
Dog writes one log per triggered rule, at the span's start time:

| Log field | Value |
| --- | --- |
| `source` | `appsec` |
| `service`, `host`, `env` tag | The span's service, `_dd.hostname` and env |
| `status` | `warn`, or `info` when the tracer blocked the request (`appsec.blocked:true`) |
| `message` | `<rule name> (<attack type>) on <method> <route>`, such as `SQL Injection (sql_injection) on GET /search` |
| `@appsec.rule.id`, `@appsec.rule.name` | The WAF rule |
| `@appsec.type`, `@appsec.category` | The rule's `type` and `category` tags, such as `sql_injection` and `attack_attempt` |
| `@appsec.blocked` | Whether the request was blocked |
| `@appsec.address`, `@appsec.value` | Where the payload was found (`server.request.query`…) and the value that matched |
| `@http.method`, `@http.route`, `@http.url`, `@http.status_code`, `@http.useragent` | From the span |
| `@network.client.ip` | From `http.client_ip`, `network.client.ip` or `actor.ip` |
| `@usr.id` | When the span has a `usr.id` |
| `trace_id`, `span_id` | The attacked span, to open its trace |

These logs are written whatever the [ingestion controls](ingestion-control.md)
decide for the trace itself: an attack is kept even when its trace is not.

To find attacks, search the Log Explorer:

```text
source:appsec @appsec.type:sql_injection -@appsec.blocked:true
```

## The App & API Protection page

The page covers the selected time range (the last day by default).

| Part | Shows |
| --- | --- |
| Attack types | Top 10 `@appsec.type` values by number of attacks |
| Attackers | Top 10 `@network.client.ip` |
| Targeted services | Top 10 services |
| Rules | Top 10 `@appsec.rule.name` |
| API endpoints | The endpoint inventory (below) |

Click an entry of a top list to open the matching logs in the Log
Explorer; **All attacks** opens every `source:appsec` log of the range.

### API endpoint inventory

The inventory lists every `http.route` seen on a service entry span in the
range (up to 2,000 routes), grouped by service, method and route:

| Column | Meaning |
| --- | --- |
| Service, Endpoint | The service and `<method> <route>`, such as `GET /api/orders/{id}` |
| Requests, Errors | Service entry spans of that route, and those in error |
| Attacks | `source:appsec` logs of that service and route; click to open them |
| Last seen | The latest request |

An endpoint appears as soon as your tracer sets `http.route` on its
requests, whether or not the WAF is enabled.

## Detect attackers

The default Cloud SIEM rule **Attacks from one address** counts
`source:appsec` logs per `@network.client.ip` over 5 minutes, and opens a
`medium` signal above 10 attacks and a `high` one above 50. Edit its
thresholds, or add your own rules on `source:appsec` logs, in
[Detection Rules](/security/rules). See [Cloud SIEM](siem.md).

## Differences from Datadog

- Robine Dog reads what the tracers' WAF reports on spans. It does not send WAF rules or blocking lists to the tracers (no Remote Configuration), so the tracers use their bundled rules, and blocking an IP or user from the UI is not available.
- There is no separate attack explorer: attacks are logs. Attacker profiles, API endpoint schemas and sensitive data findings are not implemented.
- Only the triggers listed in `_dd.appsec.json` are recorded; the first matched parameter of each trigger is kept.
