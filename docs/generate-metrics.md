---
title: Generate Metrics
section: Logs
order: 60
app: /logs/generate-metrics
description: Turn logs and spans into metrics as they are ingested, counts or distributions tagged by their attributes, to graph and alert on them long after the events themselves are gone.
---

# Generate Metrics

A generated metric counts the logs or spans that match a query, or records
the distribution of one of their numeric attributes, at the moment they are
ingested. The result is an ordinary metric: you graph it in the
[Metrics Explorer](metrics.md) and on dashboards, and alert on it with a
metric [monitor](monitors.md).

Generated metrics are useful for two reasons:

- Metrics are kept 15 months, logs and spans 15 days. A metric keeps the
  trend after the events are gone.
- They see events that are not stored. Log-based metrics count logs before
  [index](log-indexes.md) exclusion filters drop them, and span-based metrics
  count every span before [retention filters](ingestion-control.md) decide
  which traces to keep.

There are three kinds, each with its own page and API:

| Kind | Page | API |
| --- | --- | --- |
| Log-based | [Logs › Generate Metrics](/logs/generate-metrics) | `/api/v2/logs/config/metrics` |
| Span-based | [APM › Generate Metrics](/apm/generate-metrics) | `/api/v2/apm/config/metrics` |
| RUM-based | [RUM › Generate Metrics](/rum/generate-metrics) | `/api/v2/rum/config/metrics` |

RUM-based metrics work the same way, with an event type and a rule for
counting sessions and views once; see [Real User Monitoring](rum.md).

## How a metric is computed

A metric only counts events received after it is created, as in Datadog:
creating one does not backfill the past.

For each log that comes out of its [pipelines](log-pipelines.md) and the
[Sensitive Data Scanner](sensitive-data-scanner.md), and for each span
received, Robine Dog tests every generated metric's filter. When it matches:

1. The group-by paths are read from the event and become tags,
   `tag_name:value`. An event without the attribute gets the value `n/a`.
2. A count adds 1; a distribution adds the value of its measure. An event
   whose measure is missing or not a number is skipped by a distribution.
3. The sample is aggregated with the others in 10-second buckets, like
   DogStatsD samples, tagged with the event's host, and written with the
   other metrics.

A count is stored with the `count` type, a distribution with the
`distribution` type.

## Creating a metric

On the Generate Metrics page, click **New Metric** and fill in:

| Field | Description |
| --- | --- |
| Metric name | The metric's name, prefilled with `logs.` or `spans.`. Letters, digits, `.`, `_` and `-`, at most 200 characters. It cannot be changed afterwards. |
| Logs matching / Spans matching | A query in the [search syntax](search-syntax.md). Empty or `*` matches everything. |
| Compute | **Count of** logs or spans, or **Distribution of** a measure. |
| Measure | For a distribution, the numeric attribute: `@duration`, `@http.response_time`… |
| Include percentiles | For a distribution; kept in the definition (see [Differences from Datadog](#differences-from-datadog)). |
| Group by | Comma-separated paths that become tags: `service, @http.status_code`. Write `path:tag_name` to rename one: `@http.status_code:status_code`. |

The metric's name links to the Metrics Explorer with a ready-made query.
**Edit** changes the filter, the group-by and the percentiles option; the
name, the computation and the measure are fixed once created, as in Datadog.
Deleting a metric stops it from being computed; the points already written
stay.

### Naming and tags

- A group-by path without `@` (`service`, `env`, `host`) reads the reserved
  attribute or the tag; with `@` it reads an attribute.
- The tag name defaults to the path without its `@`: `@http.status_code`
  becomes `http.status_code:200`.
- Commas in values are replaced by `_`, as for every metric tag.
- Every distinct combination of tag values is a separate series. Group by
  attributes with a bounded set of values (status codes, services,
  countries), not by ids.

### Span-based metrics

Span-based metrics take the same fields. A few things to know:

- `@duration` is in nanoseconds, as Datadog stores span durations. A
  distribution of `@duration` is therefore in nanoseconds.
- A span attribute is looked up in its tags, then in its numeric metrics,
  so `@http.status_code` works whichever way the tracer sent it.
- Every span is counted, including the spans of traces that retention
  filters do not keep.

## Querying a generated metric

Query a count with `sum` and `.as_count()` to get the number of events per
interval:

```text
sum:logs.checkout.errors{env:prod} by {status_code}.as_count()
```

Query a distribution with `avg`, `min`, `max`, `sum`, `count`, or a
percentile: `p50`, `p75`, `p90`, `p95`, `p99`.

```text
p95:spans.checkout.duration{service:shop-api} by {resource_name}
```

See [Querying metrics](metrics.md#querying-metrics) for the full syntax.

## API

The endpoints take `DD-API-KEY` and Datadog's JSON:API format. The metric's
name is its `id`; the `type` is `logs_metrics`, `spans_metrics` or
`rum_metrics`.

| Method and path | Purpose |
| --- | --- |
| `GET /api/v2/logs/config/metrics` | List log-based metrics |
| `POST /api/v2/logs/config/metrics` | Create one |
| `GET /api/v2/logs/config/metrics/{id}` | Get one |
| `PATCH /api/v2/logs/config/metrics/{id}` | Update its `filter`, `group_by` or `compute.include_percentiles` |
| `DELETE /api/v2/logs/config/metrics/{id}` | Delete it |

The span-based (`/api/v2/apm/config/metrics`) and RUM-based
(`/api/v2/rum/config/metrics`) endpoints are the same.

The attributes of a definition:

| Attribute | Description |
| --- | --- |
| `compute.aggregation_type` | `count` or `distribution` |
| `compute.path` | The measure of a distribution (required for one) |
| `compute.include_percentiles` | Optional boolean |
| `filter.query` | The search query; defaults to `*` |
| `group_by` | A list of `{"path": "...", "tag_name": "..."}`; `tag_name` is optional |

Create a count of checkout errors by status code:

```sh
curl -X POST http://localhost:8080/api/v2/logs/config/metrics \
  -H "DD-API-KEY: $ROBINE_DOG_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"data": {"id": "logs.checkout.errors", "type": "logs_metrics",
       "attributes": {
         "compute": {"aggregation_type": "count"},
         "filter": {"query": "service:checkout status:error"},
         "group_by": [{"path": "@http.status_code", "tag_name": "status_code"}]}}}'
```

Create a distribution of request durations by resource:

```sh
curl -X POST http://localhost:8080/api/v2/apm/config/metrics \
  -H "DD-API-KEY: $ROBINE_DOG_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"data": {"id": "spans.checkout.duration", "type": "spans_metrics",
       "attributes": {
         "compute": {"aggregation_type": "distribution", "path": "@duration"},
         "filter": {"query": "service:shop-api operation_name:http.request"},
         "group_by": [{"path": "resource_name"}]}}}'
```

The API answers `400` with an error message when the name is invalid or
already used, when `aggregation_type` is neither `count` nor
`distribution`, when a distribution has no `path`, or when the filter is not
a valid query.

## Differences from Datadog

- Percentiles (`p50` to `p99`) can be queried on every distribution,
  whether `include_percentiles` is set or not.
- The metric name is checked for its characters only; Robine Dog does not
  reserve prefixes.
- Log-based metrics have no separate "uniqueness" or "count of a field"
  options; RUM-based metrics have their own, described in
  [Real User Monitoring](rum.md).
