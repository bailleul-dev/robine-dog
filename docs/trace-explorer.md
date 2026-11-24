---
title: Trace Explorer
section: APM
order: 20
app: /apm/traces
description: Search every stored span with Datadog's query syntax, graph them by any tag, and open a trace as a flame graph, a waterfall or a span list with its logs.
---

# Trace Explorer

The Trace Explorer searches the spans Robine Dog stored: those of the traces
kept by the [retention filters](ingestion-control.md). Filter them with the
[search syntax](search-syntax.md), narrow them with facets, graph them with
analytics, and click a span to open its whole trace.

Spans arrive from the [APM tracers](tracing.md) on the trace agent port
(`localhost:8126`), from the [Datadog Agent](agent.md) and from
[OpenTelemetry](opentelemetry.md). They are kept 15 days.

## Searching spans

[Traces](/apm/traces) opens on the last 15 minutes. Type a query in the
search bar:

```text
service:shop-api resource_name:"POST /api/orders" @duration:>500ms status:error
```

The selector next to the search bar picks which spans are searched:

| Scope | Spans searched | Equivalent clause |
| --- | --- | --- |
| Service Entry Spans (default) | The first span of each service in a request: what service pages count | `_entry:1` |
| Root Spans | Spans without a parent: one per trace | `_root:1` |
| All Spans | Every span, internal ones included | none |

A query that contains `trace_id:` always searches all spans, so
`trace_id:4815162342` finds every span of that trace whatever the scope.

Reserved attributes (`service`, `resource_name`, `operation_name`, `env`,
`version`, `trace_id`, `span_id`, `parent_id`, `span.type`, `status`,
`@duration`) are the fastest to filter on. Any other `@attribute` is looked
up in the span's tags (meta), then in its numeric metrics, so
`@http.status_code:500` works whichever way the tracer sent it. See
[Search syntax](search-syntax.md#reserved-attributes).

## The span list

Above the list, a histogram shows the matching spans over time, requests and
errors stacked. Drag on it to zoom the time range. Under it, Watchdog
Insights may point out the facet values over-represented in the errors of
the search; click one to add it to the query (see [Watchdog](watchdog.md)).

The list shows, for each span, its date, service, resource, duration, HTTP
method and status code, with the number of spans found. **Newest** sorts by
start time; **Slowest** (or a click on the **Duration** header) sorts by
duration, longest first. The list loads 100 spans at a time as you scroll.

Click a span to open its trace in the side panel. The URL keeps the query,
the scope (`spans=`), the sort, the trace (`trace=`) and the span (`span=`),
so you can share it.

## Facets

The facet panel on the left lists the core facets (Env, Service, Resource,
Operation Name, Status, Duration, Span Type) and the facets you created.
Each shows the values present in the current results; click a value to
filter on it or to exclude it. The search bar also suggests the span
attributes it has seen in the results. Facets work as in the Log Explorer:
see [Facets](log-explorer.md#facets).

## Analytics

**Visualize as** turns the list into a graph of the matching spans:

| Visualization | Shows |
| --- | --- |
| List | The spans (default) |
| Timeseries | The measure over time, lines or bars, one series per group |
| Top List | The groups ranked by the measure |
| Table | The measure by one or two facets (top N per parent group) |

The measure is a count, a unique count of an attribute, or the average, sum,
min, max, median, pc75, pc90, pc95 or pc99 of a numeric attribute
(`@duration` by default). Group by up to two facets and choose how many
groups to show. The settings live in the URL (`viz`, `agg`, `measure`, `by`,
`by2`, `limit`).

Save the query, facets, visualization and time range as a saved view with
the views menu in the page header.

## The trace view

The side panel shows the whole trace: its duration, start date, number of
spans and errors, its trace id, and each service's share of the execution
time. Three views of the spans:

| View | What it shows |
| --- | --- |
| Flame Graph | Spans stacked by depth on a time axis, colored by service; drag to zoom, hover for details |
| Waterfall | One row per span in start order, with its bar on the time axis |
| Span List | Spans grouped by service and resource: count, average duration, execution time |

Click a span in any view to show its details below:

- Service, operation name, error status, HTTP status code and resource.
- Duration (and share of the trace), **exec time** (the span's time minus its
  children's), start offset, env and version, span and parent ids, type.
- **Info**: the span's tags and metrics as a tree.
- **Logs**: the logs that carry the trace's id, from an hour before to an
  hour after the trace; the selected span's own logs are highlighted. Click a
  log to open it in the Log Explorer.
- **Errors** (on error spans): `error.type`, `error.message` and
  `error.stack`.

**Service page** opens the span's [service](services.md). **Profiles**
opens the [Continuous Profiler](profiler.md) on the span's service around the
span's time, and on its endpoint when the span is a root span.

## Correlating logs and traces

A log is linked to a trace when it carries the trace id. Robine Dog reads it
from `dd.trace_id` and `dd.span_id` (nested or dotted keys), or from
`trace_id` and `span_id`. Both forms of trace id correlate:

| Form | Example |
| --- | --- |
| Decimal, 64 bits | `5208512171318403364` |
| Hexadecimal, 128 bits (32 characters) | `6740d1f20000000048486ed8a3e3b524`: the low 64 bits are used |

Turn on log injection in the tracer (`DD_LOGS_INJECTION=true`). From a log,
**View Trace** opens its trace; from a span, the **Logs** tab lists its
logs. A [log pipeline](log-pipelines.md)'s trace id remapper can pick the id
from another attribute.

## Trace metrics

Every service entry span, kept or not by the retention filters, feeds three
metrics, as Datadog's tracers and Agent do:

| Metric | Type | Value |
| --- | --- | --- |
| `trace.<operation>.hits` | count | One per entry span |
| `trace.<operation>.errors` | count | One per entry span in error |
| `trace.<operation>` | distribution | The duration, in seconds |

`<operation>` is the span's operation name (`trace.http.request.hits`,
`trace.flask.request`). They are tagged `service`, `resource_name`, `env`,
`version` and `http.status_code`, and carry the span's host. Use them in the
[Metrics Explorer](metrics.md), dashboards and metric monitors; for other
attributes, create [span-based metrics](generate-metrics.md).

## API

Datadog's span APIs answer on port 8080 with the `DD-API-KEY` header.

| Method and path | Does |
| --- | --- |
| `POST /api/v2/spans/events/search` | Searches spans: `data.attributes.filter.query`, `filter.from`, `filter.to`, `page.limit`, `page.cursor`, `sort` |
| `GET /api/v2/spans/events` | The same, with `filter[query]`, `filter[from]`, `filter[to]`, `page[limit]`, `page[cursor]`, `sort` |
| `POST /api/v2/spans/analytics/aggregate` | Counts and measures by facets, in Datadog's request and bucket format |

```sh
curl -X POST http://localhost:8080/api/v2/spans/events/search \
  -H "DD-API-KEY: $ROBINE_DOG_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"data": {"type": "search_request", "attributes": {
        "filter": {"query": "service:shop-api status:error", "from": "now-1h", "to": "now"},
        "page": {"limit": 25}}}}'
```

The [MCP server](api.md)'s `search_datadog_spans` and `get_datadog_trace`
tools give AI assistants the same searches.

## Differences from Datadog

- Only the spans of retained traces are searchable: there is no 15-minute
  live search over every ingested span. Trace metrics and span-based metrics
  still count every span.
- Trace ids are stored as their low 64 bits in decimal.
- The trace view has no map view and no per-span infrastructure or network
  tabs.
