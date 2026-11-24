---
title: Software Catalog and service pages
section: APM
order: 10
app: /apm/services
description: Every service that sends traces, with its requests, errors and latency, its resources, its versions and its owners. Service definitions, Teams and Scorecards describe who owns a service and how well it is run.
---

# Software Catalog and service pages

The Software Catalog lists every service that reported traces in the time
range, with its traffic, latency and error rate. Open a service to see its
service page: request, error and latency graphs, the latency distribution,
the services it calls, each version it ran and each resource (endpoint,
query, job) it served. A resource opens its own page, with the spans that
make up its time and samples of its slowest and failed requests.

Ownership comes from **service definitions** (team, contacts, links, tier,
lifecycle), posted through Datadog's API or edited on the service page.
**Teams** group people, links and the services they own, and **Scorecards**
check every service against rules of production readiness, observability
and ownership.

## How the numbers are computed

Every figure on these pages is computed from **service entry spans**: the
first span of a service in each request (the span the tracer marks as
`top_level`), the same spans Datadog's `trace.<operation>.*` metrics count.
A request that goes through three internal spans of `shop-api` counts once.

| Figure | Meaning |
| --- | --- |
| Requests (hits) | Number of entry spans |
| Errors | Entry spans with the error flag set |
| Error rate | Errors divided by requests |
| P50, P75, P90, P95, P99 | Approximate percentiles of the entry spans' duration |
| Max, Avg | Longest and average duration |
| Total time | Sum of the durations; a resource's share of it is its **Total Time** column |

Services only appear when they have entry spans in the range. Spans are
stored for 15 days, so the catalog cannot look further back. To see which
spans were kept, see [Ingestion and retention](ingestion-control.md).

## Software Catalog

[Software Catalog](/apm/services) opens on the last hour and refreshes
every 30 seconds. The **env** selector restricts every figure to one `env`;
the text box filters services by name.

| Column | What it shows |
| --- | --- |
| Service | Name, and below it the most common operation name of its entry spans |
| Team | The `team` of its service definition, as `@handle` |
| Type | The most common span type (`web`, `db`, `cache`, `custom`…) |
| Requests | A sparkline of the range in 24 buckets, the total and the rate per second |
| P95 Latency, Avg Latency | Over the range |
| Error Rate | Shown in red above 5% |

Click a column header to sort. Click a row to open the service page, with the
same time range and env.

## Service page

The service page (`/apm/services/<name>`) shows, from top to bottom:

- **Requests, Errors, P50, P95, P99, Max**: totals of the range.
- **Ownership**: the service definition (team, tier, lifecycle, languages,
  contacts, links, description) and the service's score in each scorecard.
  **Add a service definition** or **Edit definition** opens a YAML editor,
  prefilled with a v2.2 template the first time.
- **Requests** (OK and errors stacked), **Errors** and **Latency** (p50,
  p95, p99, max) over time. Drag on a graph to zoom the time range.
- **Latency Distribution**: every request of the range in 40 buckets on a
  logarithmic scale, with p50, p95 and p99 marked.
- **Downstream Services**: the instrumented services this one calls, with
  calls, errors and average latency. A call is a span of this service whose
  child is an entry span of another service in the same trace. Click a row to
  open that service.
- **Deployments**: see [Deployment tracking](#deployment-tracking).
- **Resources**: each resource with its requests, requests per second, share
  of the service's total time, p50, p95, p99 and errors. Click a row to open
  the resource page.

**View Traces** opens the [Trace Explorer](trace-explorer.md) on
`service:<name>` (and the env), **View Logs** the Log Explorer on
`service:<name>`, and **View Profiles** the [Continuous Profiler](profiler.md)
for the service.

### Deployment tracking

The Deployments table lists each `version` that served requests in the
range, newest first, with when it was first seen (looking back up to 30
days), last seen, its requests, error rate, p50 and p95. A version's error
rate is red when it is above 1% and more than 1.5 times the previous
version's; its p95 is red when it is more than 1.5 times the previous
version's. Click a version to open its traces (`version:"<version>"`).

The table only appears when spans carry a version. Set `DD_VERSION` in the
tracer; see [Unified service tagging](tagging.md).

## Resource page

A resource page (`/apm/services/<name>/resource?resource=<resource>`) shows
one resource of a service:

- Requests and errors, latency over time and the latency distribution.
- The status codes of its requests (`http.status_code`), top 10.
- **Span Summary**: the spans found under the resource's last 500 requests,
  grouped by service and operation name, with the number of traces they
  appear in, spans per trace, average duration, p95 and their share of the
  execution time. Up to 30 rows, the largest share first.
- **Slowest requests** and **Latest errors**: 15 of each, which open the
  trace.

## Service definitions

A service definition describes a service in Datadog's format. Robine Dog
accepts schema versions `v2`, `v2.1` and `v2.2`, in JSON or YAML, and keeps
one definition per `dd-service`. Posting a definition again replaces it.

```yaml
schema-version: v2.2
dd-service: shop-api
team: storefront
description: Orders and checkout API
tier: "1"
lifecycle: production
languages: [python]
contacts:
  - name: Support
    type: email
    contact: storefront@example.com
links:
  - name: Source
    type: repo
    provider: github
    url: https://github.com/example/shop-api
  - name: Runbook
    type: runbook
    url: https://wiki.example.com/shop-api/runbook
tags: ["cost-center:web"]
```

| Field | Rules |
| --- | --- |
| `schema-version` | Required: `v2`, `v2.1` or `v2.2` |
| `dd-service` | Required: lowercase letters, digits and `- _ . / :`, 100 characters at most |
| `team` | A team's **handle**; the service is listed under that team |
| `contacts` | `type` is one of `email`, `slack`, `microsoft-teams`, `website`, `link`, and `contact` is required |
| `links` | From v2.1: `type` is one of `doc`, `repo`, `runbook`, `dashboard`, `other`; `name` and `url` are required |
| `repos`, `docs` | v2 only: shown as `repo` and `doc` links |
| `tier`, `lifecycle`, `description`, `application`, `languages`, `type`, `tags` | Shown as written; `lifecycle` cannot be empty |

### Service definition API

| Method and path | Does |
| --- | --- |
| `GET /api/v2/services/definitions` | Every definition |
| `GET /api/v2/services/definitions/{service}` | One definition |
| `POST /api/v2/services/definitions` | Creates or replaces, JSON or YAML body (bare or under `data.attributes.schema`) |
| `DELETE /api/v2/services/definitions/{service}` | Deletes |

```sh
curl -X POST http://localhost:8080/api/v2/services/definitions \
  -H "DD-API-KEY: $ROBINE_DOG_API_KEY" \
  -H "Content-Type: application/x-yaml" \
  --data-binary @service.datadog.yaml
```

The answer is Datadog's: `data[].attributes.schema` with a `meta` object.
A definition that breaks a rule is refused with the reason.

## Teams

[Teams](/teams) lists each team with its members, links and the services
whose definition names its handle. A team has a name and a **handle**
(lowercase letters, digits, `-`, `_` and `.`, unique). Members are
identified by e-mail address and can have the `admin` role. Links have a
label and an `http(s)` URL.

| Method and path | Does |
| --- | --- |
| `GET /api/v2/team` | Lists teams; `filter[keyword]` matches the name or the handle |
| `POST /api/v2/team` | Creates (`data.attributes.name`, `handle`, `description`) |
| `GET`, `PATCH`, `DELETE /api/v2/team/{id}` | Reads, updates, deletes |
| `GET`, `POST /api/v2/team/{id}/memberships` | Lists, adds a member (`data.relationships.user.data.id`, optional `data.attributes.role: admin`) |
| `DELETE /api/v2/team/{id}/memberships/{user}` | Removes a member |
| `GET`, `POST /api/v2/team/{id}/links` | Lists, adds a link (`label`, `url`, optional `position`) |
| `DELETE /api/v2/team/{id}/links/{link}` | Removes a link |

```sh
curl -X POST http://localhost:8080/api/v2/team \
  -H "DD-API-KEY: $ROBINE_DOG_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"data": {"type": "team", "attributes": {"name": "Storefront", "handle": "storefront"}}}'
```

## Scorecards

[Scorecards](/apm/scorecards) shows, for every service with spans in the
last day or a service definition, whether it passes each rule, with its
score overall and per scorecard. A score is the share of passed rules;
skipped rules do not count. Hover an outcome to read its remarks.

The out-of-the-box rules are evaluated when you open the page:

| Scorecard | Rule | Passes when |
| --- | --- | --- |
| Production Readiness | Has monitors | A monitor's query contains `service:<name>`, or a monitor is tagged with it |
| Production Readiness | Has SLOs | Same, for an SLO |
| Production Readiness | Has on-call contacts | The service definition lists contacts |
| Observability Best Practices | Deployment tracking | Entry spans of the last day carry a `version` |
| Observability Best Practices | Unified service tagging | Entry spans of the last day carry an `env` |
| Observability Best Practices | Logs correlated with traces | Logs of the last day from the service carry a trace id |
| Ownership & Documentation | Has a team | The definition names a team |
| Ownership & Documentation | Has a code repository | The definition has a `repo` link |
| Ownership & Documentation | Has documentation | The definition has a `doc` or `runbook` link |
| Ownership & Documentation | Has a tier | The definition sets a tier |

### Custom rules

**New Rule** creates a custom rule in any scorecard, existing or new. Robine
Dog does not evaluate custom rules: your own job posts their outcomes
(`pass`, `fail` or `skip`, with remarks). The latest outcome per rule and
service is kept. Disabled rules are left out of the scores.

| Method and path | Does |
| --- | --- |
| `GET /api/v2/scorecard/rules` | Built-in (`custom: false`) and custom rules |
| `POST /api/v2/scorecard/rules` | Creates a rule: `name`, `scorecard_name`, optional `description`, `enabled`, `owner` |
| `PUT`, `DELETE /api/v2/scorecard/rules/{id}` | Updates, deletes (with its outcomes); built-in rules cannot change |
| `GET /api/v2/scorecard/outcomes` | Every outcome; filter with `filter[rule][id]`, `filter[outcome][service_name]`, `filter[outcome][state]` |
| `POST /api/v2/scorecard/outcomes/batch`, `POST /api/v2/scorecard/outcomes` | Posts outcomes of custom rules |

```sh
curl -X POST http://localhost:8080/api/v2/scorecard/outcomes/batch \
  -H "DD-API-KEY: $ROBINE_DOG_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"data": {"type": "batched-outcome", "attributes": {"results": [
        {"rule_id": "<rule id>", "service_name": "shop-api", "state": "pass", "remarks": "pinned dependencies"}
      ]}}}'
```

The body can also be JSON:API `data`, an array of outcomes each with
`attributes.service_name`, `attributes.state`, `attributes.remarks` and
`relationships.rule.data.id`.

## Differences from Datadog

- The catalog lists services seen in spans only; a service with a definition
  and no traffic shows on Teams and Scorecards, not in the catalog.
- Service definitions are stored as posted and are not read from GitHub.
  Schema `v3` and entity kinds other than services are not accepted.
- Built-in Scorecard rules are fixed: they cannot be edited or disabled.
- Team members are e-mail addresses; there are no user records or team
  permissions.

> **Note:** the service and resource pages are computed in
> `crates/robine-dog/src/api/apm.rs`; definitions, teams, scorecards and
> retention filters live in `crates/robine-dog/src/catalog/`.
