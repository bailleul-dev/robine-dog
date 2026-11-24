---
title: Error Tracking
section: APM
order: 50
app: /error-tracking
description: Errors from traces, logs, browsers and Sentry SDKs grouped into issues, with their volume, stack trace and breakdown, a review status and regressions.
---

# Error Tracking

An application that fails once fails many times the same way. Error Tracking
groups individual errors into **issues**: one line per distinct problem,
with how often it happens, since when, and the stack trace of its latest
occurrences. You review an issue once, resolve it, and Robine Dog tells you
if it comes back.

There is nothing to configure: Error Tracking reads the errors already
stored in spans and logs.

## Where errors come from

| Source | What counts as an error | Shown as |
| --- | --- | --- |
| [APM spans](tracing.md) | A span with the error flag and an `error.type` tag | APM |
| [Logs](log-collection.md) | A log with status `error`, `critical`, `alert` or `emergency` and an `error.kind` (or `error.type`) or `error.stack` attribute | Logs |
| [Browser (RUM)](rum.md) | Each RUM error is also stored as an error log with `source:browser` | Logs |
| [Sentry SDKs](sentry.md) | Each Sentry event becomes an error log with `error.kind`, `error.message` and `error.stack` | Logs |

The tracers set `error.type`, `error.message` and `error.stack` on spans when
an exception escapes a traced function. For logs, a
[pipeline](log-pipelines.md) can remap your logger's exception fields to
`error.kind`, `error.message` and `error.stack`.

## How errors are grouped

Errors with the same **fingerprint** are one issue. The fingerprint is made
of:

| Part | Spans | Logs |
| --- | --- | --- |
| Source | `spans` | `logs` |
| Service | `service` | `service` |
| Type | `error.type` | `error.kind`, else `error.type`, else `Error` |
| Message pattern | `error.message` | `error.message`, else the log message |
| Top frame | The first indented line of `error.stack` | Same |

In the message pattern, every number and every run of 8 or more hexadecimal
characters is replaced by `*`, so `Order 1234 not found` and `Order 98 not
found` are one issue. The top frame is the first frame line of the stack in
Java, Python, Go, JavaScript and similar formats.

### Custom grouping

When the application knows better, it sets an `error.fingerprint` tag on the
span (or attribute on the log). That value replaces the message pattern, and
the stack trace no longer splits the issue: every error with the same
service, type and fingerprint is one issue, whatever its message.

```python
from ddtrace import tracer

with tracer.trace("payment.charge") as span:
    try:
        charge(order)
    except ProviderError as e:
        span.set_tag("error.fingerprint", f"provider-{e.provider}")
        raise
```

## Issues list

[Error Tracking](/error-tracking) lists the issues that had errors in the
time range, the most frequent first (200 at most). Filter them with the
search bar, using the [search syntax](search-syntax.md) on the underlying
spans and logs (`service:shop-api env:prod`, `@error.type:TimeoutError`),
and with the facets on the left:

| Facet | Values |
| --- | --- |
| Status | For Review, Reviewed, Ignored, Resolved |
| Source | APM, Logs |
| Team | The team owning the service, from its [service definition](services.md#service-definitions) |
| Service | The issue's service |

Each row shows the error type, message, service and top frame, a volume
graph over the range, the number of errors, when it was first and last
seen, and its status.

## Issue page

Click an issue to open its page:

- **Status**: change it with the selector.
- **Occurrences**, **First seen**, **Last seen**, and a graph of the errors
  over the time range.
- **View in** opens the matching spans in the Trace Explorer or logs in the
  Log Explorer.
- **Stack trace** of the latest occurrence (or of the one you click in the
  occurrences table), with **View trace** (spans) or **View log** (logs). Frames
  of your code are highlighted; library and runtime frames (`site-packages`,
  `node_modules`, `org.springframework`, `jakarta.`, `/usr/lib`, `runtime/`)
  are dimmed.
- **Breakdown**: the share of the occurrences by version, env, host and
  resource for spans, by host for logs (top 10 values each).
- **Occurrences** table: the latest 200, with date, message, host and, for
  spans, resource, env and version.

## Statuses and regressions

| Status | Meaning |
| --- | --- |
| For Review | New issues start here |
| Reviewed | Someone looked at it |
| Ignored | Not worth acting on; stays ignored when it happens again |
| Resolved | Fixed |

An issue's first sighting is remembered, so **First seen** stays correct
when you narrow the time range. When a **Resolved** issue has an error after
the time it was resolved, it goes back to **For Review** and is marked
**Regression**.

## Exception Replay

When Exception Replay is turned on in the tracer, the tracer captures the
local variables of the frames of an exception and sends them as snapshots to
the Agent's debugger endpoint. Robine Dog receives them on the trace port
(`/debugger/v1/input` and `/debugger/v2/input` on `localhost:8126`) and
stores them as logs with `source:dd_debugger`.

The tracer tags the error span with the snapshots' ids
(`_dd.debug.error.<n>.snapshot_id`). On the issue page, an occurrence with
snapshots shows **Exception Replay: N snapshots with the local variables**,
which opens those snapshot logs in the Log Explorer
(`source:dd_debugger @debugger.snapshot.id:<id>`).

## Alerting on issues

Error Tracking [monitors](monitors.md) count errors grouped into issues:

```text
error-tracking-traces("service:shop-api env:prod").rollup("count").by("@issue.id").last("5m") > 10
```

Use `error-tracking-traces`, `error-tracking-logs`, `error-tracking-rum`
or `error-tracking` (spans and logs). They count with `rollup("count")` and
group by `@issue.id`, `service` or `error.type`. Issues can also be shown
in a dashboard's list widget.

## Differences from Datadog

- There is no public Error Tracking API; statuses are changed in the UI.
- Issues have no assignee, no comments and no suspected cause; to follow
  one up, open a [case](case-management.md).
- Grouping is computed when you open the page, over the stored spans and
  logs, so an issue's volume follows the [retention filters](ingestion-control.md)
  and [log indexes](log-indexes.md).
- Exception Replay shows the snapshots as logs, not inline in the stack
  trace.
