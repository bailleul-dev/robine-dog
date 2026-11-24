---
title: Monitors
section: Monitors and Alerting
order: 10
app: /monitors/manage
description: Monitors evaluate a query every minute, per group, and move between OK, Warn, Alert and No Data. They cover metrics, logs, spans, RUM, events, Error Tracking issues, service checks, other monitors and SLOs.
---

# Monitors

A monitor is a query with a threshold. Robine Dog evaluates every monitor
once a minute. When the query has a `by` clause, each group (each `host`,
each `service`…) has its own state, as Datadog's multi alert monitors do.
When a group changes state, Robine Dog records the change, posts an
[event](events.md) and sends the monitor's message to the handles it names:
see [Notifications](notifications.md).

Monitors use Datadog's definition format and API (`/api/v1/monitor`), so
monitors exported from Datadog, or managed with its clients and Terraform
provider, work as they are.

## Monitor types

The type is inferred from the query: you do not need to set it.

| Type (`type`) | Query form | Example |
| --- | --- | --- |
| `metric alert` | `<agg>(last_<window>):<metric query> <op> <n>` | `avg(last_5m):avg:system.cpu.user{env:prod} by {host} > 80` |
| `query alert` (change) | `change(<agg>(last_<w>),last_<shift>):<metric query> <op> <n>`, or `pct_change(…)` | `pct_change(avg(last_5m),last_1h):avg:shop.checkout.hits{*} by {service} > 50` |
| `query alert` (functions) | `<agg>(last_<w>):<function>(<metric query>, …) <op> <n>` | `avg(last_4h):anomalies(avg:system.cpu.user{*} by {host}, 'agile', 2, direction='above', alert_window='last_15m') >= 1` |
| `query alert` (forecast) | `<agg>(next_<w>):forecast(<metric query>, …) <op> <n>` | `max(next_1w):forecast(avg:system.disk.in_use{*} by {host}, 'linear', 1) >= 0.9` |
| `log alert` | `logs("<search>").index("*").rollup(…).by(…).last("<w>") <op> <n>` | `logs("status:error").index("*").rollup("count").by("service").last("5m") > 50` |
| `trace-analytics alert` | `trace-analytics("<search>")…` (or `spans(…)`) | `trace-analytics("service:shop-api").rollup("pc95", "@duration").by("resource_name").last("10m") > 500000000` |
| `rum alert` | `rum("<search>")…` | `rum("@type:error").rollup("count").by("@view.name").last("5m") > 10` |
| `event-v2 alert` | `events("<search>")…` | `events("source:watchdog").rollup("count").last("1h") > 0` |
| `error-tracking alert` | `error-tracking("<search>")…`, `error-tracking-traces(…)`, `error-tracking-logs(…)` | `error-tracking-traces("service:shop-api").rollup("count").by("@issue.id").last("1h") > 5` |
| `service check` | `"<check>".over(<tags>).by(<tags>).last(<n>).count_by_status()` | `"http.can_connect".over("env:prod").by("host").last(3).count_by_status()` |
| `composite` | Monitor ids with `&&`, `\|\|`, `!` and parentheses | `1001 && (1002 \|\| !1003)` |
| `slo alert` | `burn_rate("<slo id>")…` or `error_budget("<slo id>")…` | `burn_rate("a1b2c3").over("30d").long_window("1h").short_window("5m") > 14.4` |
| `synthetics alert` | Created for you by each [synthetic test](synthetics.md) | Read only |

Comparators are `>`, `>=`, `<`, `<=`, `==` and `!=`. Windows are a number
and a unit: `s`, `m`, `h`, `d` or `w` (`last_15m`, `last_1d`).

### Metric monitors

The time aggregation (`avg`, `min`, `max` or `sum`) reduces the points of
the window to one value per group. The metric query is the one of the
[Metrics Explorer](metrics.md): `avg:shop.orders{env:prod} by {service}`,
with `.as_count()`, `.as_rate()` or `.rollup()` as needed.

- **Change** monitors compare the aggregate of the window with the same
  aggregate `shift` earlier: `change` is the difference, `pct_change` the
  difference in percent of the older value.
- **Function** monitors wrap the query in a function and alert on the result.
  Besides `anomalies`, `outliers` and `forecast`, the metric functions are
  accepted: `abs`, `log2`, `log10`, `cumsum`, `integral`, `diff`,
  `derivative`, `monotonic_diff`, `per_second`, `per_minute`, `per_hour`,
  `moving_rollup`, `fill`, `trend_line`, `robust_trend`,
  `piecewise_constant`, `count_nonzero`, `count_not_null`, `exclude_null`,
  `clamp_min`, `clamp_max`, `cutoff_min`, `cutoff_max`, `default_zero`,
  `timeshift`, `hour_before`, `day_before`, `week_before`, `month_before`,
  `top`, `bottom`, `ewma_<n>` and `median_<n>`.

| Function | Arguments | Value compared with the threshold |
| --- | --- | --- |
| `anomalies(q, '<algo>', <bounds>, direction=…, alert_window=…, seasonality=…)` | `basic`, `agile` or `robust`; bounds in deviations (default 2); `direction` `above`, `below` or `both`; `alert_window` (default `last_15m`); `seasonality` `hourly`, `daily` or `weekly` (default weekly) | The share of the alert window's points outside the band, 0 to 1 |
| `outliers(q, '<algo>', <tolerance>)` | `dbscan`, `scaleddbscan`, `mad` or `scaledmad` | 1 for a group that is an outlier, 0 otherwise |
| `forecast(q, '<algo>', <deviations>)` | `linear` or `seasonal` | The forecast's upper bound over the `next_` window (its lower bound when the comparator is `<` or `<=`) |

A forecast monitor must use a `next_` window and learns from four times
that window. Only `forecast` accepts a `next_` window.

## Log monitors

Log monitors count or measure the logs that match a search over the
window, with the [search syntax](search-syntax.md) of the Log Explorer.

```text
logs("service:shop-api status:error @http.status_code:>=500").index("*").rollup("count").by("service,host").last("15m") >= 10
```

| Method | What it does |
| --- | --- |
| `.rollup("count")` | Counts the events (the default) |
| `.rollup("<agg>", "<measure>")` | `cardinality`, `avg`, `sum`, `min`, `max`, `median` or `pc<NN>` (such as `pc95`) of a measure (`@duration`, `@http.response_time`) |
| `.by("a,b")` | One group per value combination; facets, tags or `@attributes` |
| `.last("5m")` | The window (required) |
| `.index("*")` | Accepted for compatibility; every log is searched |

A count with no matching log is 0, not missing data, so a query such as
`… .last("10m") < 1` alerts when logs stop arriving. A group that has been
seen before and matches nothing in the window also counts 0.

RUM (`rum(…)`) and event (`events(…)`) monitors take the same methods over
RUM events and the event stream.

## APM monitors

APM monitors (`trace-analytics(…)`, or `spans(…)`) count or measure spans,
with the span search syntax of the [Trace Explorer](trace-explorer.md).
`@duration` is in nanoseconds:

```text
trace-analytics("service:shop-api env:prod").rollup("pc99", "@duration").by("resource_name").last("10m") > 2000000000
```

Add `status:error` to the search to count failed spans:
`trace-analytics("service:shop-api status:error").rollup("count").last("5m") > 20`.

**Error Tracking** monitors count the errors of [issues](error-tracking.md):
`error-tracking-traces` reads span errors, `error-tracking-logs` error logs,
and `error-tracking` both. They take `.rollup("count")` only and group by
`@issue.id`, `service` or `error.type`.

### Service checks

A service check monitor reads the runs of an Agent check (`datadog.agent.up`,
`http.can_connect`…). `.over()` filters by tags (`"*"` for all), `.by()`
groups, and the thresholds count consecutive runs rather than values:

| Option | Meaning (default 1) |
| --- | --- |
| `thresholds.critical` | Alert after this many consecutive CRITICAL runs |
| `thresholds.warning` | Warn after this many consecutive WARNING (or CRITICAL) runs |
| `thresholds.ok` | Recover after this many consecutive OK runs |

The query has no comparator. Runs from the last 24 hours are read.

### Composite monitors

A composite combines other monitors by id. A monitor in Alert counts as
true, Warn as true with a warning severity, OK as false, and No Data as
unknown. The composite alerts when the expression is true, warns when only
warnings make it true, and keeps its state while the result is unknown.
When the monitors are grouped, the composite is evaluated group by group; a
monitor without groups applies to every group. A composite cannot include
itself, and every id must exist.

### SLO alerts

`burn_rate("<id>").over("<7d|30d|90d>").long_window("<w>").short_window("<w>")`
alerts on how many times faster than allowed the [SLO](slos.md) burns its
error budget; the value is the lower of the two windows' burn rates. The
long window is 1 minute to 48 hours (default `1h`), the short one shorter
(default a twelfth of the long one). `error_budget("<id>").over("30d") > 75`
alerts on the percentage of the budget spent. The SLO must have a target
for the time frame.

## Evaluation and states

Every minute, each monitor's query runs over its window, ending now (or
`evaluation_delay` seconds ago). Each group gets one value, then a state:

| State | When |
| --- | --- |
| `Alert` | The value crosses the query's threshold (`thresholds.critical`) |
| `Warn` | The value crosses `thresholds.warning` |
| `No Data` | `notify_no_data` is on and the group has had no value for `no_data_timeframe` minutes |
| `OK` | Otherwise |

- A group in Alert stays in Alert until the value is back past
  `critical_recovery`; the same goes for Warn and `warning_recovery`.
- Without data, and without `notify_no_data`, a group keeps its state.
- The monitor's overall state is its worst group: Alert, then Warn, No Data
  and OK.
- A new group that starts OK is not reported. A group that is OK and has had
  no data for a day is dropped, and comes back when it is seen again.
- Creating or saving a monitor evaluates it at once.
- Each evaluated value is stored and kept as long as metrics (15 months by
  default) for the monitor's history graph.

## Options

| Option | Default | Meaning |
| --- | --- | --- |
| `thresholds.critical` | From the query | Set from the query's threshold when you save |
| `thresholds.warning` | none | Warn threshold, compared with the same comparator |
| `thresholds.critical_recovery`, `thresholds.warning_recovery` | none | Values the group must get past to recover |
| `notify_no_data` | `false` | Turn missing data into `No Data` |
| `no_data_timeframe` | Twice the window, in minutes | Minutes without data before `No Data` |
| `renotify_interval` | none | Minutes between reminders while a group is not OK |
| `escalation_message` | none | Appended to the message of reminders |
| `evaluation_delay` | 0 | Seconds the window is shifted back, for data that arrives late |
| `new_group_delay` | 0 | Seconds a new group keeps its state after it first appears |
| `require_full_window` | `false` | Metric monitors: only groups with data from the start of the window |
| `silenced` | `{}` | Muted scopes; see [Muting](#muting) |
| `priority` (top level) | none | 1 to 5, shown as `P1`…`P5` in titles |
| `tags` (top level) | `[]` | Used by search, downtimes and the Tags facet |

Other options are kept as given and ignored.

## Create a monitor

Open [New Monitor](/monitors/create) and pick a type. The editor has three
steps:

1. **Define the metric** or **Define the search query**, **Pick the check**,
   **Combine monitors** or **Pick the SLO**, depending on the type. **Edit
   the query as text** switches to the raw query; **Back to the editor**
   parses it back into the form.
2. **Set alert conditions**: the comparator, the alert and warning
   thresholds, the recovery thresholds, **Notify if data is missing**, the
   evaluation delay and the new group delay.
3. **Configure notifications**: the name, the message (with buttons that
   insert `{{#is_alert}}`, `{{value}}` and your `@webhook-` handles), tags,
   priority, **Renotify if still not resolved** (never, 10 or 30 minutes,
   1 or 4 hours, or a day) and the escalation message.

On the right, **Current evaluation** runs the query as you type and shows
each group's value and the state it would have now.

[Templates](/monitors/templates) lists ready-made monitors: high CPU, disk
almost full, low memory, Agent down, pod restarts and CrashLoopBackOff,
APM errors and latency, error logs, front-end errors and certificate
expiry. **Use** creates the monitor and opens it for editing, so you can
add recipients and adjust the scope.

## Manage and triggered monitors

[Manage Monitors](/monitors/manage) lists every monitor with its state,
priority, tags, type, mute icon and last trigger. The search box filters on
the name, query and tags; the facets filter by status, type, muted and tag.
The filters are kept in the URL.

[Triggered Monitors](/monitors/triggered) lists the groups in Alert, Warn or
No Data, worst first, with their value and since when.

A monitor's page shows its query, a **History** graph of the evaluated
values per group with the thresholds drawn on it, the state of each group,
the **Events** (every state change, its value and who was notified) and the
message. From there you can **Mute**, **Clone**, **Edit** or delete it.

## Muting

**Mute** on a monitor's page silences all its groups until you unmute it.
Through the API you can mute one scope, for a time:

```sh
curl -X POST http://localhost:8080/api/v1/monitor/1001/mute \
  -H "DD-API-KEY: $ROBINE_DOG_API_KEY" -H "Content-Type: application/json" \
  -d '{"scope": "host:web-1", "end": 1767225600}'
```

A scope is `*` or `key:value` tags, comma separated, that a group must all
carry. `end` is in epoch seconds; without it the mute lasts until
`/unmute`, which takes `{"scope": "host:web-1"}` or `{"all_scopes": true}`.
A muted monitor is still evaluated and its state still changes; only the
notifications stop. For planned silences, use [downtimes](downtimes.md).

## Monitor Quality

[Monitor Quality](/monitors/quality) lists the monitors that probably need
attention:

| Issue | Rule |
| --- | --- |
| `no_recipient` | The message has no `@` handle |
| `muted_forever` | A scope is muted without an end |
| `muted_long` | A scope is muted for more than 30 days |
| `alert_too_long` | A group has been in Alert for more than 7 days |
| `no_data` | The monitor is in No Data |
| `no_delay` | The query reads `aws.`, `gcp.` or `azure.` metrics without `evaluation_delay` |

## Search monitors

`GET /api/v1/monitor/search` takes Datadog's monitor search syntax. Terms are
separated by spaces and must all match:

| Term | Matches |
| --- | --- |
| `status:alert` | The overall state: `ok`, `warn`, `alert` or `nodata` |
| `type:log` | Types that start with the value (`metric`, `log`, `trace-analytics`, `service`…) |
| `muted:true` | Monitors with at least one muted scope |
| `id:1001` | One monitor |
| `tag:"env:prod"` or `env:prod` | A monitor tag |
| `checkout` | A word of the name |

```sh
curl -G http://localhost:8080/api/v1/monitor/search \
  -H "DD-API-KEY: $ROBINE_DOG_API_KEY" \
  --data-urlencode 'query=status:alert tag:"team:shop"' -d per_page=50
```

The response has the page of `monitors`, the `counts` per status, type,
muted and tag, and `metadata` with `page`, `per_page` (30 by default, 100
at most), `page_count` and `total_count`.

## API

The routes take a `DD-API-KEY` header; `DD-APPLICATION-KEY` is accepted and
not required.

| Method and path | What it does |
| --- | --- |
| `GET /api/v1/monitor` | Lists monitors, synthetic tests included; `name=` (substring) and `monitor_tags=a,b` filter |
| `POST /api/v1/monitor` | Creates a monitor; ids start at 1000 |
| `GET /api/v1/monitor/{id}` | One monitor, with `overall_state` and `state.groups` |
| `PUT /api/v1/monitor/{id}` | Partial update: only the fields given change, and `options` are merged |
| `DELETE /api/v1/monitor/{id}` | Deletes it and its history; returns `deleted_monitor_id` |
| `POST /api/v1/monitor/validate` | Checks a definition without saving it |
| `POST /api/v1/monitor/{id}/mute`, `/unmute` | See [Muting](#muting) |
| `GET /api/v1/monitor/search` | See [Search monitors](#search-monitors) |

```sh
curl -X POST http://localhost:8080/api/v1/monitor \
  -H "DD-API-KEY: $ROBINE_DOG_API_KEY" -H "DD-APPLICATION-KEY: $ROBINE_DOG_APP_KEY" \
  -H "Content-Type: application/json" -d @- <<'EOF'
{
  "name": "Error logs on shop-api",
  "query": "logs(\"service:shop-api status:error\").index(\"*\").rollup(\"count\").last(\"5m\") > 50",
  "message": "{{#is_alert}}{{value}} error logs in 5 minutes.{{/is_alert}} @webhook-ops-alerts",
  "tags": ["team:shop", "env:prod"],
  "priority": 2,
  "options": {"thresholds": {"warning": 20}, "renotify_interval": 60}
}
EOF
```

A definition is refused when the query does not parse, the name is empty,
the priority is not 1 to 5, a composite names a missing monitor, or an SLO
alert names an SLO without a target for its time frame.

## Differences from Datadog

- Synthetic tests appear as `synthetics alert` monitors with ids from
  2,000,000,000, but you change them on the [test](synthetics.md), not
  through the monitor API.
- Template variables are rendered in the message, not in the monitor name:
  the title of a notification uses the name as written.
- `.index()` on log monitors is ignored: every log is searched.
- Options Robine Dog does not use (`notify_audit`, `include_tags`,
  `timeout_h`…) are stored and returned, with no effect.
- Monitor search in the UI is a text filter with facets; the Datadog search
  syntax is available through the API.
