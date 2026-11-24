---
title: API and MCP server
section: Reference
order: 20
description: Robine Dog's Datadog-compatible public API, grouped by product, how to point Datadog's API clients, Terraform provider and datadog-ci at it, and its MCP server for AI assistants.
---

# API and MCP server

Robine Dog serves Datadog's public API on its main port, on Datadog's paths
and with Datadog's request and response formats. The official API clients,
the Terraform provider, `datadog-ci` and your own scripts work against it once
they are pointed at Robine Dog's address. An MCP server exposes the same
data to AI assistants under the names of Datadog's MCP tools.

## Authentication

Every public API request needs an API key, in the `DD-API-KEY` header (or a
`dd-api-key` or `api_key` query parameter): `ROBINE_DOG_API_KEY` or a named
API key. The `DD-APPLICATION-KEY` header is optional; when it carries an
application key you created, its use is recorded. See
[API and application keys](api-keys.md).

```sh
curl http://localhost:8080/api/v1/monitor \
  -H "DD-API-KEY: $ROBINE_DOG_API_KEY" -H "DD-APPLICATION-KEY: $ROBINE_DOG_APP_KEY"
```

A missing or unknown key gets `403` with `{"errors": ["Forbidden"]}`. Every
change made through the API is recorded in the [Audit Trail](audit-trail.md).

> **Note:** the interface uses its own API under `/api/ui/`, which needs a
> signed-in session and is not a stable interface. Use the public API below.

## Endpoints

The tables list the paths; each product's page describes the fields and
behaviour.

### Logs

| Path | Methods | Page |
| --- | --- | --- |
| `/api/v2/logs/events/search` | `POST` | [Log Explorer](log-explorer.md) |
| `/api/v2/logs/events` | `GET` | [Log Explorer](log-explorer.md) |
| `/api/v2/logs/analytics/aggregate` | `POST` | [Log Explorer](log-explorer.md) |
| `/api/v1/logs/config/pipelines`, `/{id}`, `/api/v1/logs/config/pipeline-order` | `GET`, `POST`, `PUT`, `DELETE` | [Log Pipelines](log-pipelines.md) |
| `/api/v1/logs/config/indexes`, `/{name}`, `/api/v1/logs/config/index-order` | `GET`, `POST`, `PUT`, `DELETE` | [Indexes](log-indexes.md) |
| `/api/v2/logs/config/archives`, `/{id}`, `/api/v2/logs/config/archive-order` | `GET`, `POST`, `PUT`, `DELETE` | [Archives](log-archives.md) |
| `/api/v2/logs/config/custom-destinations`, `/{id}` | `GET`, `POST`, `PATCH`, `DELETE` | [Log Forwarding](log-forwarding.md) |
| `/api/v2/logs/config/metrics`, `/{id}` | `GET`, `POST`, `PATCH`, `DELETE` | [Generate Metrics](generate-metrics.md) |
| `/api/v2/sensitive-data-scanner/config` and its `groups`, `rules`, `standard-patterns` | `GET`, `POST`, `PATCH`, `DELETE` | [Sensitive Data Scanner](sensitive-data-scanner.md) |
| `/api/v2/reference-tables/tables`, `/{id}`, `/{id}/rows` | `GET`, `POST`, `PATCH`, `DELETE` | [Reference Tables](reference-tables.md) |

### APM

| Path | Methods | Page |
| --- | --- | --- |
| `/api/v2/spans/events/search`, `/api/v2/spans/events` | `POST`, `GET` | [Trace Explorer](trace-explorer.md) |
| `/api/v2/spans/analytics/aggregate` | `POST` | [Trace Explorer](trace-explorer.md) |
| `/api/v2/apm/config/metrics`, `/{id}` | `GET`, `POST`, `PATCH`, `DELETE` | [Generate Metrics](generate-metrics.md) |
| `/api/v2/apm/config/retention-filters`, `/{id}`, `/api/v2/apm/config/retention-filters-execution-order` | `GET`, `POST`, `PUT`, `DELETE` | [Ingestion Control](ingestion-control.md) |
| `/api/v2/services/definitions`, `/{name}` | `GET`, `POST`, `DELETE` | [Services](services.md) |
| `/api/v2/team` and its `memberships`, `links` | `GET`, `POST`, `PATCH`, `DELETE` | [Services](services.md) |
| `/api/v2/scorecard/rules`, `/api/v2/scorecard/outcomes`, `/outcomes/batch` | `GET`, `POST`, `PUT`, `DELETE` | [Services](services.md) |

### Metrics and infrastructure

| Path | Methods | Page |
| --- | --- | --- |
| `/api/v1/query` | `GET` | [Metrics](metrics.md) |
| `/api/v2/query/timeseries`, `/api/v2/query/scalar` | `POST` | [Dashboards](dashboards.md) |
| `/api/v1/metrics`, `/api/v2/metrics`, `/api/v1/search` | `GET` | [Metrics](metrics.md) |
| `/api/v1/metrics/{name}` | `GET`, `PUT` | [Metrics](metrics.md) |
| `/api/v2/metrics/{name}/tags`, `/api/v2/metrics/{name}/volumes` | `GET`, `POST`, `PATCH`, `DELETE` | [Metrics without Limits](metrics-without-limits.md) |
| `/api/v1/hosts`, `/api/v1/hosts/totals` | `GET` | [Infrastructure](infrastructure.md) |
| `/api/v2/cost/custom_costs`, `/{id}` | `GET`, `PUT`, `DELETE` | [Cloud Cost](cloud-cost.md) |

### RUM and Synthetics

| Path | Methods | Page |
| --- | --- | --- |
| `/api/v2/rum/events/search`, `/api/v2/rum/events` | `POST`, `GET` | [RUM](rum.md) |
| `/api/v2/rum/analytics/aggregate` | `POST` | [RUM](rum.md) |
| `/api/v2/rum/config/metrics`, `/{id}` | `GET`, `POST`, `PATCH`, `DELETE` | [Generate Metrics](generate-metrics.md) |
| `/api/v1/synthetics/tests` (`/api`, `/browser`, `/{type}/{id}`, `/{id}`, `/delete`, `/search`, `/{id}/status`, `/{id}/results`) | `GET`, `POST`, `PUT`, `DELETE` | [Synthetics](synthetics.md) |
| `/api/v1/synthetics/tests/trigger/ci`, `/api/v1/synthetics/ci/batch/{id}`, `/api/v2/synthetics/tests/poll_results` | `POST`, `GET` | [Synthetics](synthetics.md) |
| `/api/v1/synthetics/variables`, `/{id}` | `GET`, `POST`, `PUT`, `DELETE` | [Synthetics](synthetics.md) |

### Monitors, dashboards and notebooks

| Path | Methods | Page |
| --- | --- | --- |
| `/api/v1/monitor`, `/{id}`, `/validate`, `/search`, `/{id}/mute`, `/{id}/unmute` | `GET`, `POST`, `PUT`, `DELETE` | [Monitors](monitors.md) |
| `/api/v1/downtime`, `/{id}` | `GET`, `POST`, `PUT`, `DELETE` | [Downtimes](downtimes.md) |
| `/api/v1/integration/webhooks/configuration/webhooks`, `/{name}` | `GET`, `POST`, `PUT`, `DELETE` | [Notifications](notifications.md) |
| `/api/v1/dashboard`, `/{id}` | `GET`, `POST`, `PUT`, `DELETE` | [Dashboards](dashboards.md) |
| `/api/v1/dashboard/lists/manual`, `/{id}`, `/api/v2/dashboard/lists/manual/{id}/dashboards` | `GET`, `POST`, `PUT`, `DELETE` | [Dashboards](dashboards.md) |
| `/api/v1/dashboard/public`, `/{token}` | `GET`, `POST`, `PUT`, `DELETE` | [Dashboards](dashboards.md) |
| `/api/v1/notebooks`, `/{id}` | `GET`, `POST`, `PUT`, `DELETE` | [Notebooks](notebooks.md) |

### Service management and security

| Path | Methods | Page |
| --- | --- | --- |
| `/api/v1/slo`, `/{id}`, `/search`, `/can_delete`, `/{id}/history`, `/{id}/corrections`, `/api/v1/slo/correction` | `GET`, `POST`, `PUT`, `PATCH`, `DELETE` | [SLOs](slos.md) |
| `/api/v1/events` (`GET`), `/api/v2/events/search`, `/api/v2/events` | `GET`, `POST` | [Events](events.md) |
| `/api/v2/incidents`, `/{id}` | `GET`, `POST`, `PATCH`, `DELETE` | [Incidents](incidents.md) |
| `/api/v2/cases`, `/{id}`, `/{id}/{action}` | `GET`, `POST` | [Case Management](case-management.md) |
| `/api/v2/on-call/schedules`, `/escalation-policies`, `/pages`, `/teams/{team}/on-call` | `GET`, `POST`, `PUT`, `DELETE` | [On-Call](on-call.md) |
| `/api/v2/workflows`, `/{id}`, `/{id}/instances`, `/api/v2/app-builder/apps` | `GET`, `POST`, `PUT`, `PATCH`, `DELETE` | [Workflows](workflows.md) |
| `/api/v2/security_monitoring/rules`, `/api/v2/security_monitoring/signals` | `GET`, `POST`, `PUT`, `PATCH`, `DELETE` | [Cloud SIEM](siem.md) |
| `/api/v2/audit/events` | `GET` | [Audit Trail](audit-trail.md) |
| `/api/v2/api_keys`, `/api/v2/application_keys` | `GET`, `POST`, `PATCH`, `DELETE` | [API and application keys](api-keys.md) |

The intakes (`/api/v2/logs`, `/api/v1/series`, `/api/v2/series`,
`POST /api/v1/events`, `/api/v1/check_run`…) and `GET /api/v1/validate` are
described in the [Send data](README.md#send-data) pages.

### Searching

The search endpoints take Datadog's request format: a `filter` with a
`query` in the [search syntax](search-syntax.md), `from` and `to`
(`now-15m` and `now` by default; also ISO 8601 dates or epoch seconds or
milliseconds), and a `page` with a `limit` (10 by default, 1,000 at most) and
a `cursor`. Span searches also take a `sort`.

```sh
curl -X POST http://localhost:8080/api/v2/logs/events/search \
  -H "DD-API-KEY: $ROBINE_DOG_API_KEY" -H 'Content-Type: application/json' \
  -d '{"filter": {"query": "service:payment-service status:error", "from": "now-1h", "to": "now"},
       "page": {"limit": 25}}'
```

The response holds `data` and, when there are more results, the next
cursor in `meta.page.after`. The `GET` forms take the same fields as query
parameters: `filter[query]`, `filter[from]`, `page[limit]`, `page[cursor]`.

## Use Datadog's clients

Datadog's tools derive their API address from a site; point them at Robine
Dog's address explicitly instead.

### API clients

The official API clients take a host in their configuration. In Python:

```python
import os
from datadog_api_client import ApiClient, Configuration
from datadog_api_client.v1.api.monitors_api import MonitorsApi

configuration = Configuration(host="http://localhost:8080")
configuration.api_key["apiKeyAuth"] = os.environ["ROBINE_DOG_API_KEY"]
configuration.api_key["appKeyAuth"] = os.environ["ROBINE_DOG_APP_KEY"]

with ApiClient(configuration) as api_client:
    for monitor in MonitorsApi(api_client).list_monitors():
        print(monitor.id, monitor.name, monitor.overall_state)
```

### Terraform

Set the Datadog provider's `api_url`:

```hcl
provider "datadog" {
  api_url = "http://localhost:8080/"
  api_key = var.robine_dog_api_key
  app_key = var.robine_dog_app_key
}
```

The provider requires an application key: create one in
[API Keys](/organization-settings/api-keys). Resources whose API is listed
above, such as `datadog_monitor`, `datadog_dashboard_json`,
`datadog_logs_custom_pipeline`, `datadog_logs_index` or
`datadog_synthetics_test`, are managed as they are in Datadog.

### datadog-ci

`datadog-ci synthetics run-tests` reads its API address from
`DD_API_HOST_OVERRIDE`:

```sh
DD_API_HOST_OVERRIDE=http://localhost:8080 DD_API_KEY=$ROBINE_DOG_API_KEY \
DD_APP_KEY=$ROBINE_DOG_APP_KEY datadog-ci synthetics run-tests --public-id abc-def-ghi
```

Robine Dog also receives the uploads of `datadog-ci junit upload`
(`/api/v2/cireport`), `datadog-ci coverage upload` (`/api/v2/cicovreprt`)
and `datadog-ci deployment mark` (`/api/intake/ci/custom_spans`); see
[CI Visibility](ci-visibility.md), [Code Coverage](code-coverage.md) and
[CD Visibility](cd-visibility.md).

## MCP server

Robine Dog serves a Model Context Protocol server at `/v1/mcp`, modelled on
Datadog's: the tools of Datadog's `core` toolset, under their names, so an
assistant set up for Datadog's MCP server works against Robine Dog. Each
tool calls the same API as the interface, so a model reads what the pages
show.

| | |
| --- | --- |
| Transport | Streamable HTTP: JSON-RPC requests (or batches) in `POST /v1/mcp`, answered in JSON. No event stream: `GET` answers `405` |
| Protocol versions | `2025-06-18`, `2025-03-26`, `2024-11-05` |
| Authentication | `ROBINE_DOG_API_KEY` in a `DD_API_KEY` header, a `DD-API-KEY` header, or `Authorization: Bearer <key>`. Named API keys are not accepted |
| Without `ROBINE_DOG_API_KEY` | Only answers when sign-in is disabled (`ROBINE_DOG_AUTH=disabled`) |

A request without a valid key gets `401` with a JSON-RPC error.

```json
{
  "mcpServers": {
    "robine-dog": {
      "type": "http",
      "url": "http://localhost:8080/v1/mcp",
      "headers": { "DD_API_KEY": "<API key>" }
    }
  }
}
```

> **Note:** some reverse proxies, nginx among them, drop headers whose name
> contains an underscore. Behind one, use `Authorization: Bearer <API key>`.

### Tools

| Tool | Does |
| --- | --- |
| `search_datadog_logs` | Search logs by query and time |
| `analyze_datadog_logs` | Count or aggregate logs, grouped by facets |
| `search_datadog_spans` | Search spans |
| `aggregate_spans` | Count or aggregate spans (`p95` of `@duration` by `resource_name`…) |
| `get_datadog_trace` | A whole trace, by trace id |
| `search_datadog_entities` | Services seen by APM, with requests, errors and latency |
| `search_datadog_metrics` | Metrics, with their type and unit |
| `get_datadog_metric_context` | A metric's metadata, tags and tag values |
| `get_datadog_metric` | Metric queries over time |
| `search_datadog_monitors` | Monitors with their state, query and thresholds |
| `search_datadog_events` | Events: monitor alerts, Watchdog stories, incidents, Agent and API events |
| `search_datadog_rum_events` | RUM events |
| `search_datadog_hosts` | Hosts reporting through an Agent |
| `search_datadog_dashboards`, `get_datadog_dashboard` | Dashboards, and one with its widgets |
| `search_datadog_incidents`, `get_datadog_incident` | Incidents, and one with its timeline (by id or number, `IR-12`) |
| `search_datadog_notebooks`, `get_datadog_notebook` | Notebooks, and one with its cells |
| `create_datadog_notebook` | Create a notebook from Markdown |
| `append_datadog_notebook` | Add Markdown, a metric graph or a log graph to a notebook |

The last two write; the others are marked read-only. Times are given as
`now-15m`, `now-4h`, `now-7d` (units `s`, `m`, `h`, `d`, `w`, `mo`), an ISO
8601 date or epoch seconds; the default range is the past hour. Queries use
the [search syntax](search-syntax.md) and Datadog's metric query language.
Every tool takes `max_tokens` (10,000 by default, between 500 and 100,000):
a longer answer is cut, with a note asking to narrow the query.

## Differences from Datadog

- There is no rate limiting and no `X-RateLimit-*` headers.
- Application keys are never required, and their scopes are not enforced.
- Only the endpoints listed here exist; any other request under `/api/`
  with a valid key is acknowledged with `202` and ignored, so that the
  Datadog Agent stays healthy (see [Datadog Agent](agent.md)).
- The MCP server has only the `core` toolset's tools listed above.
