---
title: Ingestion and retention
section: APM
order: 40
app: /apm/ingestion-control
description: How ingested spans become metrics and which traces are stored. Retention filters keep the traces matching a query at a rate; Ingestion Control shows what each service sends and how its tracer sampled it.
---

# Ingestion and retention

Every span Robine Dog receives is counted, but not every trace has to be
stored. As in Datadog, ingestion and retention are two steps:

1. **Every span** feeds the [trace metrics](trace-explorer.md#trace-metrics),
   the [span-based metrics](generate-metrics.md), the App and API Protection
   attack logs and the usage counters of Ingestion Control. These numbers are
   exact whatever is stored afterwards.
2. **Retention filters**, in order, then decide which traces are stored and
   searchable in the [Trace Explorer](trace-explorer.md) and on
   [service pages](services.md). Traces that no filter keeps are dropped.

This applies to traces from the tracers (port 8126), from the Datadog Agent
(`/api/v0.2/traces`) and from OpenTelemetry (`/v1/traces`).

> **Note:** what the tracer sends is decided before Robine Dog sees it. A
> tracer configured with `DD_TRACE_SAMPLE_RATE=0.1` only sends a tenth of
> its traces; Ingestion Control shows that decision, and retention filters
> work on what arrives.

## How retention filters decide

Spans arrive in chunks: one trace, or the part of it one service or one Agent
flush sent. For each chunk:

1. The enabled filters are tried in their execution order.
2. A filter applies when its query matches at least one span of the chunk.
3. The first applicable filter whose rate includes the trace keeps the chunk.
   Whether a trace falls within a rate is decided on the **trace id**, with
   the same hash as Datadog's tracers, so every chunk of a trace gets the
   same answer from a given rate.
4. A chunk that no filter keeps is dropped.

Kept spans get the tag `_dd.retention_filter:<filter name>`, so you can
search `@_dd.retention_filter:"Error Default"` to see what a filter kept.

A filter on one service's spans keeps that service's chunks: the other
services of the same trace are only kept if a filter keeps them too.

### Default filters

On first start Robine Dog creates two filters, so everything is stored
until you change them:

| Order | Name | Query | Rate |
| --- | --- | --- | --- |
| 1 | Error Default | `status:error` | 100% |
| 2 | Default | `*` | 100% |

**Error Default** is Datadog's error filter: you can change its rate or
disable it, but not its query, and it cannot be deleted. To store fewer
traces, lower the rate of **Default** and add filters for what you always
want to keep:

| Name | Query | Rate |
| --- | --- | --- |
| Slow checkouts | `service:shop-api resource_name:"POST /api/checkout" @duration:>2s` | 100% |
| Payment calls | `service:payments` | 50% |
| Default | `*` | 10% |

## Retention Filters page

[Retention Filters](/apm/retention-filters) lists the filters in execution
order with their query, retention rate and state. **New Retention Filter**
asks for:

| Field | Meaning |
| --- | --- |
| Name | Required; shown in `_dd.retention_filter` |
| Keep traces with a span matching | A [span query](search-syntax.md) |
| Retention rate (%) | The share of matching traces kept, 0 to 100 |
| Enabled | Disabled filters are skipped |

New filters run **before** the existing ones. Use the arrows to move a filter
up or down. Changes apply to spans received from then on.

## Ingestion Control

[Ingestion Control](/apm/ingestion-control) shows what each service sent over
the last hour:

| Column | Meaning |
| --- | --- |
| Service | The service of the spans |
| Spans/s | Ingested spans per second, over the hour |
| Spans (1h) | Ingested spans |
| Volume (1h) | An estimate of their size in bytes |
| Stored | The share of its spans the retention filters kept |
| Sampling decisions (root spans) | How the tracer decided to keep its traces, from the root span's `_dd.p.dm` and sampling priority |
| Tracer | The tracer language and version (`language`, `_dd.tracer_version` tags) |
| Last seen | When the service last sent spans |

The totals at the top add up every service. A service whose spans are never
the root of a trace shows "not a trace root" for sampling decisions.

The decision makers (`_dd.p.dm`) are shown with their meaning and whether the
priority kept or dropped the trace (`rule: keep`, `agent rate: drop`):

| `_dd.p.dm` | Shown as |
| --- | --- |
| `0` | `default` (or `none` without a sampling priority) |
| `1` | `agent rate` |
| `2` | `remote rate (auto)` |
| `3` | `rule` |
| `4` | `manual` |
| `5` | `appsec` |
| `8` | `single span` |
| `11` | `remote rate (user)` |
| `12` | `remote rate (dynamic)` |

These counters are kept in memory: they start again from zero when Robine
Dog restarts, and a service is forgotten after a day without spans.

## Usage metrics

Every chunk also posts Datadog's estimated usage metrics, tagged `service`,
which you can graph or alert on:

| Metric | Type | Counts |
| --- | --- | --- |
| `datadog.estimated_usage.apm.ingested_spans` | count | Spans received |
| `datadog.estimated_usage.apm.ingested_bytes` | count | Estimated bytes received |
| `datadog.estimated_usage.apm.indexed_spans` | count | Spans stored by the retention filters |

```text
sum:datadog.estimated_usage.apm.ingested_spans{*} by {service}.as_count()
```

## Retention filters API

Datadog's API, with the `DD-API-KEY` header:

| Method and path | Does |
| --- | --- |
| `GET /api/v2/apm/config/retention-filters` | Lists the filters in order, with `execution_order` |
| `POST /api/v2/apm/config/retention-filters` | Creates a filter, placed first |
| `GET /api/v2/apm/config/retention-filters/{id}` | Reads a filter |
| `PUT /api/v2/apm/config/retention-filters/{id}` | Replaces a filter (the error filter keeps its query) |
| `DELETE /api/v2/apm/config/retention-filters/{id}` | Deletes a filter, except the error filter |
| `PUT /api/v2/apm/config/retention-filters-execution-order` | Sets the order: `data` lists every filter's `id` once |

```sh
curl -X POST http://localhost:8080/api/v2/apm/config/retention-filters \
  -H "DD-API-KEY: $ROBINE_DOG_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"data": {"type": "apm_retention_filter", "attributes": {
        "name": "Slow checkouts",
        "filter": {"query": "service:shop-api @duration:>2s"},
        "rate": 1.0,
        "enabled": true}}}'
```

| Attribute | Rules |
| --- | --- |
| `name` | Required |
| `filter.query` | A valid span query |
| `rate` | 0 to 1 |
| `enabled` | Default `true` |
| `trace_rate` | Accepted (0 to 1) and returned, but not used: whole chunks are kept |
| `filter_type` | Set by Robine Dog: `spans-sampling-processor`, or `spans-errors-sampling-processor` for the error filter |

## Differences from Datadog

- Robine Dog keeps or drops whole chunks; it does not keep single spans of a
  trace, so `trace_rate` has no effect.
- There is no "Intelligent Retention" filter and no 15-minute window in which
  every span is searchable.
- Ingestion Control is read-only: it does not change the tracers' sampling
  through Remote Configuration. Set sampling in the tracer
  (`DD_TRACE_SAMPLE_RATE`, `DD_TRACE_SAMPLING_RULES`).
