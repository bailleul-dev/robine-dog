---
title: Service Level Objectives
section: Service Management
order: 10
app: /slo/manage
description: Service level objectives track how often a service is good enough over 7, 30 or 90 days, how much error budget is left and how fast it burns. They come from metrics, monitors or time slices.
---

# Service Level Objectives

A service level objective (SLO) states a target, such as *99.9% of checkout
requests succeed over 30 days*. Robine Dog computes the service level
indicator (SLI) behind it, compares it with the target, and shows the
**error budget**: how much failure the target still allows. Burn-rate and
error-budget [monitors](monitors.md) alert you before the budget runs out.

SLOs use Datadog's v1 SLO API, so Terraform providers and API clients that
manage Datadog SLOs work against Robine Dog.

## SLO types

| Type | `type` | Good | Total |
| --- | --- | --- | --- |
| Metric-based | `metric` | The numerator query's events | The denominator query's events |
| Monitor-based | `monitor` | Seconds when none of the monitors is in `Alert` | Every second of the window |
| Time slice | `time_slice` | Seconds of the 1- or 5-minute slices whose value meets the condition | Seconds of every slice |

In every case the SLI is `good / total × 100`, summed over the whole time
frame. Metric-based SLOs count events; the two other types count seconds.

### Metric-based

The numerator counts good events and the denominator counts all events.
Both are [metric queries](metrics.md); use `.as_count()` so that each
interval contributes its count. Either side can also be a difference of
queries separated by ` - ` (with spaces), which lets you write *all minus
bad*:

```text
numerator:   sum:trace.http.request.hits{service:shop-api,env:prod}.as_count() - sum:trace.http.request.errors{service:shop-api,env:prod}.as_count()
denominator: sum:trace.http.request.hits{service:shop-api,env:prod}.as_count()
```

The `trace.<operation>.hits` and `.errors` metrics are derived from the
service entry spans of every [traced service](tracing.md).

### Monitor-based

A monitor-based SLO is up while none of its monitors is in `Alert`. `Warn`
and `No Data` count as up. For a multi-alert monitor, you can restrict the
SLO to some groups (`groups`, or **Only these groups** in the editor, the
group names as the monitor shows them, separated by `;`). The SLO reads the
monitors' stored state changes, so it covers the time since the monitors
were created.

### Time slice

A time-slice SLO cuts time into 1-minute or 5-minute slices
(`query_interval_seconds`: `60` or `300`, default `300`), computes a metric
query for each, and counts the slice as good when its value compares with
the threshold as stated (`<`, `<=`, `>`, `>=`). A slice with no data counts
as good. For example *p95 latency under 500 ms per 5 minutes*:

```json
{
  "time_slice": {
    "query": {
      "queries": [{"data_source": "metrics", "name": "query1", "query": "p95:trace.http.request{service:shop-api}"}],
      "formulas": [{"formula": "query1"}]
    },
    "comparator": "<",
    "threshold": 0.5,
    "query_interval_seconds": 300
  }
}
```

Only the first formula is evaluated.

## Targets and time frames

An SLO has one to three targets, one per time frame:

| Field | Values |
| --- | --- |
| `timeframe` | `7d`, `30d` or `90d` (rolling windows ending now) |
| `target` | A percentage strictly between 0 and 100, such as `99.9` |
| `warning` | Optional; above the target and below 100 |

The first target is the SLO's main one: it is also returned as
`timeframe`, `target_threshold` and `warning_threshold`, the shorthand
Datadog's API accepts on creation.

For each time frame the SLO has a state:

| State | When |
| --- | --- |
| `ok` | The SLI is at or above the target (and the warning, if set) |
| `warning` | The SLI is at or above the target but below the warning |
| `breached` | The SLI is below the target |
| `no_data` | Nothing to count in the window |

Statuses are cached for one minute.

## Error budget and burn rate

The **error budget** is the failure the target allows: with a 99.9% target
over 1,000,000 requests, 1,000 may fail. The budget remaining is
`(allowed − bad) / allowed × 100`; it goes below zero once the SLO is
breached.

The **burn rate** is how many times faster than allowed the budget is
spent: `(bad / total) / (1 − target / 100)`. At 1× the budget lasts exactly
the time frame; at 14.4× a 30-day budget is gone in about two days.

## Create an SLO

1. Open [SLOs](/slo/manage) and click **New SLO**.
2. Pick **Metric based**, **Monitor based** or **Time Slice based**.
3. Enter the queries, pick the monitors, or write the slice's query and
   condition.
4. Set the targets: a time frame, a target and an optional warning. **Add**
   another time frame if you want to follow more than one.
5. Give the SLO a name, an optional description and tags such as
   `team:shop, env:prod`, and save.

When you edit an existing SLO, the editor keeps its type.

## The SLO list

The list shows each SLO with its main target, time frame, status and error
budget left. The search box filters as you type: each word must match the
name or be one of the SLO's tags exactly (`team:shop`), and `type:metric`,
`type:monitor` or `type:time_slice` keep one type.

## The SLO page

Click an SLO to open its page:

- **One tile per time frame**: the SLI and the error budget left. Click a
  tile to show that time frame in the graphs.
- **Error budget remaining**: the budget left over the selected time frame,
  as the window fills.
- **Burn rate (past hour)**: the burn rate over a sliding hour, with markers
  at 1× and 14.4×.
- **Bad events per interval** (metric SLOs) or **Downtime per interval**
  (monitor and time-slice SLOs).
- **Definition**: the queries, monitors or slice condition.
- **Alerts**: the burn-rate and error-budget monitors built on the SLO, and
  **New SLO alert**, which opens the monitor editor on this SLO and time
  frame.
- **Status corrections**: the corrections of the SLO, with **Add a
  correction** in the header.

**Edit** reopens the editor. Deleting an SLO that monitors are built on asks
for confirmation, then leaves those monitors without an SLO.

SLOs also show on dashboards through the SLO widget; see
[Dashboards](dashboards.md).

## Status corrections

A correction removes a period from the SLO, both its good and its bad: a
planned maintenance does not burn the budget. Corrections apply to every
type and to every time frame, retroactively.

| Field | Description |
| --- | --- |
| `slo_id` | The SLO corrected |
| `start` | Epoch seconds; for a recurring correction, the first occurrence |
| `end` | Epoch seconds, for a one-off correction |
| `duration` | Seconds; required for a recurring correction (each occurrence's length) |
| `rrule` | The recurrence, see below |
| `timezone` | The IANA zone the occurrences follow, such as `Europe/Paris` (default UTC) |
| `category` | `Scheduled Maintenance`, `Outside Business Hours`, `Deployment` or `Other` (default) |
| `description` | Free text |

The recurrence is an iCalendar `RRULE` with these parts:

| Part | Values |
| --- | --- |
| `FREQ` | `DAILY`, `WEEKLY` or `MONTHLY` (required) |
| `INTERVAL` | Every *n* days, weeks or months |
| `BYDAY` | With `WEEKLY`: `MO`, `TU`, `WE`, `TH`, `FR`, `SA`, `SU` |
| `COUNT` | The number of occurrences |
| `UNTIL` | The last date, `20261231` or `20261231T180000` |

For example, every weekend night from 22:00 for 8 hours, Paris time:

```text
FREQ=WEEKLY;BYDAY=SA,SU
```

with `start` on a Saturday at 22:00, `duration` `28800` and `timezone`
`Europe/Paris`. In the UI, **Add a correction** offers the same fields; a
recurring correction uses your browser's time zone.

For a time-slice SLO, a slice is left out when at least half of it is
corrected.

## SLO alerts

Two monitor queries watch an SLO. Their monitor type is `slo alert`.

| Query | Value compared with the threshold |
| --- | --- |
| `burn_rate("<slo id>").over("30d").long_window("1h").short_window("5m") > 14.4` | The burn rate over both windows: the lower of the two, so the alert fires only while both burn fast |
| `error_budget("<slo id>").over("30d") > 75` | The share of the time frame's error budget spent, in percent |

- `over` is the time frame whose target applies: `7d`, `30d` or `90d`
  (default `30d`).
- `long_window` is 1 minute to 48 hours (default `1h`); `short_window` must
  be shorter (default a twelfth of the long window, at least 1 minute).
- An error-budget query takes no windows.

See [Monitors](monitors.md) for thresholds, notifications and evaluation.

## API

The SLO API requires a `DD-API-KEY` header.

| Method and path | Description |
| --- | --- |
| `GET /api/v1/slo` | List; `ids`, `query` (words of the name), `tags_query` (comma-separated tags, all required), `limit` (default 1000), `offset` |
| `POST /api/v1/slo` | Create |
| `GET /api/v1/slo/{id}` | One SLO, with `overall_status` per time frame and `configured_alert_ids` |
| `PUT /api/v1/slo/{id}` | Replace |
| `DELETE /api/v1/slo/{id}` | Delete, with its corrections; refused while monitors use it unless `force=true` |
| `GET /api/v1/slo/can_delete?ids=a,b` | Which SLOs no monitor uses |
| `GET /api/v1/slo/{id}/history` | Good and total per interval, SLI and error budget; `from_ts`, `to_ts` (epoch seconds, default the past 7 days, 90 days at most), `target` |
| `GET /api/v1/slo/search` | Search; `query`, `page[size]` (default 25, at most 100), `page[number]`, `include_facets` |
| `GET /api/v1/slo/{id}/corrections` | The SLO's corrections |
| `GET`, `POST /api/v1/slo/correction` | List (optionally `slo_id`), create |
| `GET`, `PATCH`, `DELETE /api/v1/slo/correction/{id}` | One correction; `PATCH` changes the attributes given |

The search query is made of space-separated terms that must all match:
words of the name or description, `type:metric`, `type:monitor`,
`type:time_slice` (or `type:time-slice`), `tag:env:prod`, `id:<id>`, or a
tag written as is (`team:shop`). With `include_facets=true` the response
counts the results per `slo_type` and per tag key (`all_tags.team`).

Create a metric-based SLO:

```sh
curl -X POST http://localhost:8080/api/v1/slo \
  -H "DD-API-KEY: $ROBINE_DOG_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "name": "Checkout availability",
    "type": "metric",
    "query": {
      "numerator": "sum:trace.http.request.hits{service:shop-api}.as_count() - sum:trace.http.request.errors{service:shop-api}.as_count()",
      "denominator": "sum:trace.http.request.hits{service:shop-api}.as_count()"
    },
    "thresholds": [
      {"timeframe": "30d", "target": 99.9, "warning": 99.95},
      {"timeframe": "7d", "target": 99.9}
    ],
    "tags": ["team:shop", "env:prod"]
  }'
```

A monitor-based SLO takes `"type": "monitor"` and `"monitor_ids": [12]`
instead of `query`. When `type` is missing, it is inferred: `monitor` with
`monitor_ids`, `time_slice` with `sli_specification`, `metric` otherwise.

Add a one-off correction for a maintenance window:

```sh
curl -X POST http://localhost:8080/api/v1/slo/correction \
  -H "DD-API-KEY: $ROBINE_DOG_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"data": {"type": "correction", "attributes": {
        "slo_id": "<slo id>",
        "start": 1795000000,
        "end": 1795003600,
        "category": "Scheduled Maintenance",
        "description": "Database upgrade"}}}'
```

## Differences from Datadog

- Time frames are `7d`, `30d` and `90d` rolling windows; calendar-aligned
  windows (week to date, month to date) are not available.
- Metric-based SLOs take one numerator and one denominator, each a single
  query or a difference of queries; other formulas are not supported.
- Monitor-based SLOs read the state history of the monitors; Datadog's
  per-group breakdown of the SLO is not returned.
- Recurring corrections support `DAILY`, `WEEKLY` and `MONTHLY` rules only.
- The history endpoint covers 90 days at most.
