---
title: Service Map
section: APM
order: 30
app: /apm/map
description: A graph of the services that sent traces and the calls between them, colored by error rate, built from the parent and child spans of stored traces.
---

# Service Map

The Service Map draws every service that sent traces in the time range and
the calls between them. It answers "what does this service depend on, and
who depends on it?" and shows at a glance which services and which
dependencies are failing.

The map is computed from the stored spans: no configuration is needed beyond
[instrumenting your services](tracing.md) with a `service` name each.

## How dependencies are found

A dependency (an edge from service A to service B) is a pair of spans in the
same trace where:

- the parent span belongs to service A,
- the child span belongs to service B, a different service,
- the child is a **service entry span**: the first span of B in that request.

Each edge carries the number of calls, the number of those calls in error
and their average duration, all measured on B's entry spans. The 500
busiest edges of the range are drawn.

Because edges come from real parent and child spans, a call appears only
when both sides are traced. A database or a third-party API shows as a
service when the tracer gives its spans a service name of their own (as
older ddtrace integrations do with `postgres` or `redis`); otherwise it is
part of the calling service's spans.

> **Note:** only stored traces are used. If [retention filters](ingestion-control.md)
> drop most traces of a service, its edges show fewer calls than it made.

## Reading the map

[Service Map](/apm/map) opens on the last hour and refreshes every minute.

| Element | Meaning |
| --- | --- |
| Column | Services are laid out by call depth: callers on the left, the services they call to the right |
| Node | One service, with its span type in the circle, its requests per second and p95 latency underneath |
| Node color | Its error rate: green under 1%, orange from 1% to 5%, red at 5% and above |
| Edge | A dependency, from caller to callee; thicker when it carries more calls (log scale) |
| Edge color | Grey under 1% errors, orange from 1% to 5%, red at 5% and above |

The header shows how many services and dependencies the map holds.
Hover an edge to read its calls, error rate and average latency.

Within a column, services are placed next to the services that call them,
and the first column puts the services that call the most others first.

## Filtering and exploring

| Control | Effect |
| --- | --- |
| Time picker | The range of spans used |
| env | Only the spans of one `env` |
| Find a service | Highlights the first service whose name contains the text, with its neighbors |
| Hover a service | Highlights it, the services it calls and those calling it; other nodes and edges fade; active edges show their calls per minute and average latency |
| Click a service | Opens its details panel and keeps it highlighted |
| Click the background | Closes the panel |

## Details panel

Clicking a service opens a panel with:

- **Requests** per second, **Errors** (error rate, in the health color),
  **p50** and **p95** latency, computed on its entry spans like the
  [service page](services.md).
- **Called by**: the services calling it, each with calls and average
  latency.
- **Calls**: the services it calls, the same way.
- **View service page**: opens the service page with the same time range.

Click a service in **Called by** or **Calls** to move the panel to it.

## Differences from Datadog

- Nodes are services only: there is no grouping by team or by
  infrastructure, and no inferred services (databases, queues and external
  APIs that are not traced).
- The layout is a fixed left-to-right layering by call depth; nodes cannot be
  dragged.
