---
title: Metrics
section: Metrics
order: 10
app: /metrics/explorer
description: Metric types, Datadog's metric query language, its functions and algorithms (anomalies, forecast, outliers), formulas across queries, and the Metrics Explorer, Summary and Volume pages.
---

# Metrics

A metric is a series of timestamped numbers identified by a name, a host
and a set of `key:value` tags. Robine Dog receives metrics from
[DogStatsD](dogstatsd.md), the series API, the [Datadog Agent](agent.md) and
[OpenTelemetry](opentelemetry.md), and derives others from your data:
trace metrics (`trace.<operation>.hits`, `.errors` and the `trace.<operation>`
latency distribution), [log-, span- and RUM-based metrics](generate-metrics.md),
and the metrics of Synthetics, CI Visibility and LLM Observability.

You query them with Datadog's metric query language, in the
[Metrics Explorer](/metrics/explorer), [dashboards](dashboards.md),
[monitors](monitors.md), [SLOs](slos.md) and the API.

## Metric types

Each metric has one type, recorded in its metadata when points arrive (the
latest type sent wins). The type decides how points are combined over time.

| Type | What a point is | Combined over time by default |
| --- | --- | --- |
| `count` | A number of occurrences in an interval | Sum |
| `rate` | Occurrences per second | Average |
| `gauge` | A value at a point in time | Average |
| `distribution` | One or more samples, each with a weight | Not combined: percentiles and averages are computed over every sample |

DogStatsD samples are aggregated into **10-second buckets** before they are
stored, as the Agent does:

| DogStatsD type | Stored as |
| --- | --- |
| `c` (counter) | `count`, the sum of the bucket; a sample rate of `0.1` counts each sample 10 times |
| `g` (gauge) | `gauge`, the last value of the bucket |
| `s` (set) | `gauge`, the number of distinct values in the bucket |
| `h`, `ms` (histogram, timing) | Five metrics: `<name>.avg`, `.median`, `.95percentile`, `.max` (gauges) and `<name>.count` (rate) |
| `d` (distribution) | `distribution`, every sample kept (equal values merged with a weight) |

Points sent to `POST /api/v1/series` or `/api/v2/series` are stored as
sent, with the type of the payload (`count`, `rate`, or `gauge` by default).
The Agent's sketches (`/api/beta/sketches`) become distributions. Metric
names are made of letters, digits, `.`, `_` and `-`.

## Querying metrics

A metric query has this shape:

```text
<space aggregator>:<metric>{<filter>} by {<tag keys>}.<function>()
```

```text
sum:trace.http.request.hits{service:shop-api,!env:dev} by {resource_name}.as_count()
```

This counts the requests of `shop-api` outside `dev`, one series per
resource.

| Part | Syntax | Default |
| --- | --- | --- |
| Space aggregator | `avg`, `sum`, `min`, `max`, `count`; on distributions also `p50` (or `median`), `p75`, `p90`, `p95`, `p99` | `avg` |
| Metric | The metric name | Required |
| Filter | `{env:prod,service:web}`, `{*}` or no braces for everything | Everything |
| Group by | `by {host}`, `by {service,env}` | One series |
| `.as_count()` | Count or rate metrics as a number per interval | |
| `.as_rate()` | Count or rate metrics per second | |
| `.rollup(<method>, <seconds>)` | Time aggregation: `avg`, `sum`, `min`, `max`, `count`, `p50`…`p99`; the seconds are optional | Sum for counts, average otherwise |
| `.fill(<mode>, <seconds>)` | Fills gaps: `null`, `zero`, `linear` or `last`, gaps up to the given length | No filling |

### Filters

Inside the braces, commas mean AND and `!` excludes:

| Filter | Matches |
| --- | --- |
| `{*}` | Every series |
| `{env:prod,service:web}` | Series with both tags |
| `{env:prod,!host:web-1}` | Series tagged `env:prod` on any host but `web-1` |
| `{host:web-*}` | Wildcards, as in the [search syntax](search-syntax.md#wildcards-and-existence) |
| `{env:prod OR env:staging}` | Each comma-separated item is a search term, so `OR` and parentheses work inside one item |
| `{canary}` | A tag without a value |

`host` filters and groups on the series' host; any other key is a tag.
Series that lack a `by` key are grouped under `N/A`.

### How a query is evaluated

Robine Dog evaluates a query as Datadog does, in two steps:

1. **Time aggregation.** Each series (one host and tag combination) is
   reduced to one value per interval: summed for counts, averaged for
   gauges and rates, or with the `.rollup()` method you give.
2. **Space aggregation.** The series are combined per `by` group with the
   space aggregator. `count:` gives the number of series reporting in each
   interval.

`.as_rate()` divides a count by the interval's length in seconds;
`.as_count()` multiplies a rate by it. They have no effect on gauges.

The interval is what `.rollup()` asks for, otherwise the one that gives
about 150 points over the time range (1 hour gives 30-second points). It is
never shorter than 10 seconds.

### Percentiles on distributions

Distributions skip the time aggregation: each interval's value is computed
over every sample of every series in the group, weighted. `p95:trace.http.request{service:shop-api}`
is the 95th percentile of all the requests of the interval, not an average
of per-host percentiles. `avg:`, `sum:`, `min:`, `max:` work on the samples
too, and `count:` gives the number of samples.

```text
p99:trace.http.request{service:shop-api} by {resource_name}
count:checkout.basket_size{env:prod}
```

## Functions

Wrap a query in a function, in the Metrics Explorer, a dashboard query or a
monitor: `ewma_10(avg:system.load.1{*} by {host})`. Functions nest:
`anomalies(ewma_5(avg:x{*} by {host}), 'basic', 2)`. In a formula, the first
argument is the query's name or another formula: `per_second(a)`.

| Function | What it does |
| --- | --- |
| `abs(q)`, `log2(q)`, `log10(q)`, `sqrt(q)` | Point by point; values outside the function's domain are dropped |
| `clamp_min(q, n)`, `clamp_max(q, n)` | Raises or lowers values to `n` |
| `cutoff_min(q, n)`, `cutoff_max(q, n)` | Drops values below or above `n` |
| `default_zero(q)` | Replaces missing points with 0 |
| `fill(q, null\|zero\|linear\|last, seconds)` | Fills gaps up to the given length; leading gaps are never filled, and only `zero` and `last` extend past the last point |
| `per_second(q)`, `per_minute(q)`, `per_hour(q)` | The value of each interval as a rate |
| `diff(q)`, `derivative(q)` | Difference with the previous point; `derivative` divides it by the interval in seconds |
| `monotonic_diff(q)` | Like `diff`, but a drop (a counter reset) counts the new value as the increase |
| `cumsum(q)` | Running sum over the time range |
| `integral(q)` | Running sum of value × interval (seconds) |
| `ewma_3(q)` … `ewma_20(q)` | Exponentially weighted moving average over N points (any N) |
| `median_3(q)` … `median_9(q)` | Rolling median over N points (any N) |
| `moving_rollup(q, seconds, 'avg')` | Rolling `avg`, `sum`, `min`, `max`, `median` or `count` over a window, 300 seconds by default |
| `trend_line(q)` | Least squares line |
| `robust_trend(q)` | Theil-Sen line, which a few outliers don't pull |
| `piecewise_constant(q)` | The series as steps |
| `count_nonzero(q)`, `count_not_null(q)` | One series: how many series are non-zero, or have a value, at each point |
| `exclude_null(q)` | Drops the groups whose tag value is `N/A` |
| `timeshift(q, -3600)`, `hour_before(q)`, `day_before(q)`, `week_before(q)`, `month_before(q)` | The query over an earlier time range; `timeshift` takes seconds or a duration (`-1h`) |
| `top(q, 10, 'mean', 'desc')`, `bottom(q, 10, 'mean')` | The N series with the highest (or lowest) `mean`, `sum`, `min`, `max`, `last`, `area` or `l2norm` |

### Algorithms

| Function | Arguments | What it shows |
| --- | --- | --- |
| `anomalies(q, 'basic', 2)` | Algorithm, bounds (2 by default), `seasonality='hourly'\|'daily'\|'weekly'` | A band of expected values around each series; points outside it are drawn in red |
| `forecast(q, 'linear', 1)` | Algorithm, deviations (1 by default), `seasonality=` | The predicted values and a band; the graph runs a quarter of its range into the future |
| `outliers(q, 'dbscan', 3)` | Algorithm, tolerance (3 by default), percentage (MAD only, 20 by default) | The series that stray from their group; the others are greyed |

`anomalies` algorithms:

| Algorithm | Expected value |
| --- | --- |
| `basic` | A rolling mean and deviation of the points just before. No seasonality, adapts fast. |
| `agile` | The previous season, shifted to the current level: follows level shifts quickly. |
| `robust` | The median of the previous three seasons: stable, slow to follow shifts. |

`agile` and `robust` read the three previous seasons (weekly unless you set
`seasonality`); where no earlier season has data, they judge points as
`basic` does.

`forecast` algorithms: `linear` fits a least squares line on the recent
points, the band widening with the distance; `seasonal` repeats the last
season plus the average change between seasons, and falls back to `linear`
until two seasons of history exist. Without `seasonality`, it picks the
longest of weekly, daily and hourly that the history can hold.

`outliers` algorithms: `dbscan` and `scaleddbscan` cluster the series by
distance and flag those outside the main cluster; `mad` and `scaledmad` flag
a series when more than the percentage of its points are further than
tolerance × MAD from the group's median. The scaled variants divide
distances by the group's level. A group needs at least three series.

## Formulas

Dashboards and the query API run several named queries on one time grid and
combine them with formulas: `+`, `-`, `*`, `/`, parentheses, numbers and the
functions above.

```text
errors / hits * 100
per_second(a) - week_before(per_second(a))
top(a / b, 5, 'max', 'desc')
```

A formula matches the series of its queries by their group tags: `a by {host}`
divided by `b by {host}` gives one series per host; a query without
`by` applies to every group of the other. A point missing on either side,
or a division by zero, gives no point. A query's `data_source` can also be
`logs`, `spans`, `rum` or `events`, with a `search`, a `compute` and a
`group_by`: its counts and measures land on the same grid as the metrics.

## The Metrics Explorer

[Metrics Explorer](/metrics/explorer) graphs up to 10 queries, lettered
`a` to `j`.

- **The query editor** has a field per part: the metric (suggested from the
  metrics received), the filter (suggested from the tags seen in the past
  hour), the space aggregator (percentiles appear for distributions), the
  group by keys, and `.as_count()` / `.as_rate()` for counts and rates.
- **`</>`** switches the query to text, to type functions, `.rollup()` or
  `.fill()`. It switches back when the text fits the editor.
- **Lines / Bars** changes the graph. Drag across the graph to zoom into a
  time range.
- **The legend** lists each series with its average, minimum, maximum and
  latest value.

Each query shows at most 50 series, the largest ones. The queries are kept
in the URL (`?q=…`), so a graph can be shared as a link. An error under a
query tells you what is wrong with it, or that the metric has no data.

## Summary

[Summary](/metrics/summary) lists every metric with its type, unit,
description and when it last reported. Search by name or filter by type.
Click a metric to open its panel:

- **Metadata**: edit the unit and the description, then **Save**. Trace
  latency distributions default to seconds.
- **Tags (past hour)**: the number of series and hosts, and the values of
  each tag key.
- **Graph in Explorer** opens the metric with a fitting query: `sum` and
  `.as_count()` for counts, `p95` for distributions, `avg` otherwise.

## Volume

[Volume](/metrics/volume) shows which metrics fill the storage: points and
distinct series stored per metric over the past hour, 4 hours or day, the
estimated points per day and each metric's share. **By prefix** groups
metrics by the first segment of their name (`trace.*`, `system.*`). To cut
the series of a metric, keep fewer of its tags with
[Metrics without Limits](metrics-without-limits.md).

## API

These routes take `DD-API-KEY` (header or `api_key` parameter), so
Datadog's clients and SDKs work against them.

| Endpoint | What it does |
| --- | --- |
| `POST /api/v2/query/timeseries` | Queries and formulas over a time range (epoch milliseconds), Datadog's request and response format; 1 to 20 queries |
| `POST /api/v2/query/scalar` | One value per series: each query reduced by its `aggregator` (`avg`, `sum`, `min`, `max`, `last`), then the formulas |
| `GET /api/v1/query?from=&to=&query=` | Datadog's v1 query (epoch seconds), several queries separated by commas |
| `GET /api/v1/metrics?from=` | Metrics active since a time (epoch seconds) |
| `GET /api/v2/metrics` | Every metric name |
| `GET`, `PUT /api/v1/metrics/{name}` | A metric's metadata; `PUT` sets `unit` and `description` |
| `GET /api/v1/search?q=metrics:<text>` | Metric names containing a text |

```sh
to=$(date +%s000); from=$((to - 3600000))
curl -s -X POST http://localhost:8080/api/v2/query/timeseries \
  -H "DD-API-KEY: $ROBINE_DOG_API_KEY" \
  -H "DD-APPLICATION-KEY: $ROBINE_DOG_APP_KEY" \
  -H "Content-Type: application/json" \
  -d @- <<EOF
{"data": {"type": "timeseries_request", "attributes": {
  "from": $from, "to": $to,
  "queries": [
    {"data_source": "metrics", "name": "errors", "query": "sum:trace.http.request.errors{service:shop-api}.as_count()"},
    {"data_source": "metrics", "name": "hits", "query": "sum:trace.http.request.hits{service:shop-api}.as_count()"}
  ],
  "formulas": [{"formula": "errors / hits * 100", "alias": "error_rate"}]
}}}
EOF
```

The response holds `times`, the `interval`, and per series its
`group_tags` and `values`. Series of `anomalies()` and `forecast()` also
carry `bounds` (`lower`, `upper`), and those of `outliers()` an `outlier`
flag; Datadog's response has neither.

```sh
curl -G http://localhost:8080/api/v1/query \
  -H "DD-API-KEY: $ROBINE_DOG_API_KEY" \
  --data-urlencode "from=$(($(date +%s) - 3600))" \
  --data-urlencode "to=$(date +%s)" \
  --data-urlencode "query=avg:system.cpu.user{env:prod} by {host}"
```

## Retention

Metric points are kept for 455 days (about 15 months), set by
`ROBINE_DOG_METRICS_RETENTION_DAYS`. See [Configuration](configuration.md).

## Differences from Datadog

- A percentile space aggregator (`p95:`) on a metric that is not a
  distribution is computed as `avg:`.
- On distributions, `.rollup()`, `.as_count()` and `.as_rate()` have no
  effect: each interval aggregates all its samples.
- Intervals are never shorter than 10 seconds.
- The Metrics Explorer has no formula field; use formulas in dashboards or
  the query API.
- `anomalies(q, 'adaptive')` is read as `robust`.
- The query API checks the API key only; an application key is accepted and
  not required.
- Units are free text, not chosen from Datadog's unit list.
