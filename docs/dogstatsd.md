---
title: DogStatsD and the series API
section: Send Data
order: 40
app: /metrics/summary
description: Send custom metrics with any DogStatsD client over UDP, or post them to the series API, and how Robine Dog aggregates and stores each metric type.
---

# DogStatsD and the series API

Robine Dog receives metrics two ways:

- **DogStatsD** over UDP on port 8125, as a local Datadog Agent does. Any
  DogStatsD client works. Robine Dog aggregates the samples into 10-second
  buckets, like the Agent.
- **The series API** over HTTP on port 8080 (`/api/v1/series`,
  `/api/v2/series`), as the Agent and scripts post it. Points are stored as
  sent, without aggregation.

Every metric then appears in [Metrics Summary](/metrics/summary) and can be
graphed in the [Metrics Explorer](/metrics/explorer), dashboards and
monitors; see [Metrics](metrics.md).

## DogStatsD

Point your client at port 8125:

```sh
DD_DOGSTATSD_URL=udp://localhost:8125
# or
DD_AGENT_HOST=localhost DD_DOGSTATSD_PORT=8125
```

The official DogStatsD clients add `env`, `service` and `version` tags from
`DD_ENV`, `DD_SERVICE` and `DD_VERSION` (see [Unified service tagging](tagging.md)).
To try it from a shell:

```sh
echo -n 'shop.orders.placed:1|c|#env:dev,payment_method:card' | nc -u -w0 localhost 8125
```

No API key is needed: like an Agent's DogStatsD port, keep it on a trusted
network. Set `ROBINE_DOG_DOGSTATSD_LISTEN=off` to turn it off.

### Datagram format

One metric per line; a packet can hold several lines (up to 65,535 bytes).

```text
<name>:<value>[:<value>…]|<type>[|@<sample rate>][|#<tag>,<tag>…][|T<unix seconds>]
```

| Field | Example | Meaning |
| --- | --- | --- |
| Name and value | `page.views:1` | Several values in one line (`latency:12:15:9`) are several samples |
| Type | `\|c` | See the table below |
| Sample rate | `\|@0.5` | Between 0 (excluded) and 1 |
| Tags | `\|#env:prod,team:web` | `key:value` or plain tags |
| Timestamp | `\|T1764000000` | The sample's time, in Unix seconds; without it, the time of receipt |

A `host:<name>` tag is removed from the tags and becomes the metric's host.
Other fields, such as the container id (`|c:…`), are ignored. A line that
doesn't parse (unknown type, value that isn't a number, sample rate out of
range) is dropped.

### Metric types and aggregation

Samples are grouped by name, host and tags into 10-second buckets aligned on
the clock, and flushed every 10 seconds.

| Type | Stored as | Value of a bucket |
| --- | --- | --- |
| `c` count | count | The sum of the values, each divided by its sample rate (`1\|c\|@0.5` counts 2) |
| `g` gauge | gauge | The last value received |
| `s` set | gauge | The number of distinct values |
| `h` histogram, `ms` timer | gauges and a rate | Five metrics: `<name>.avg`, `<name>.median`, `<name>.95percentile`, `<name>.max` (gauges) and `<name>.count` (a rate: samples per second over the bucket) |
| `d` distribution | distribution | Every sample, so percentiles are exact over any time range and group |

The sample rate only scales counts. When one name changes type within a
bucket, the latest type wins.

```text
api.request.duration:120|h|#env:prod     -> api.request.duration.avg, .median, .95percentile, .max, .count
api.request.duration:120|d|#env:prod     -> api.request.duration, queried as p50:, p95:, p99:…
```

### Service checks and events

| Datagram | Handling |
| --- | --- |
| `_sc\|<name>\|<status>\|d:<unix seconds>\|h:<host>\|#<tags>\|m:<message>` | Stored as a service check run; status `0` OK, `1` WARNING, `2` CRITICAL, `3` UNKNOWN. Without `h:`, a `host:` tag sets the host. Use them in [service check monitors](monitors.md) |
| `_e{…}` | Ignored. Post events to `POST /api/v1/events` instead; see [Events](events.md) |

```sh
echo -n '_sc|http.can_connect|2|h:web-1|#env:prod,url:http://web-1/health|m:connection refused' \
  | nc -u -w0 localhost 8125
```

## The series API

Post metrics over HTTP with an API key (see [API and application keys](api-keys.md)).
The intake answers `202` with `{"errors": []}`.

### v2

```sh
curl -X POST http://localhost:8080/api/v2/series \
  -H "DD-API-KEY: $ROBINE_DOG_API_KEY" -H 'Content-Type: application/json' \
  -d "{\"series\": [{\"metric\": \"queue.depth\", \"type\": 3,
        \"points\": [{\"timestamp\": $(date +%s), \"value\": 12}],
        \"tags\": [\"env:dev\", \"queue:orders\"],
        \"resources\": [{\"type\": \"host\", \"name\": \"worker-1\"}]}]}"
```

| Field | Meaning |
| --- | --- |
| `metric` | The name |
| `type` | `1` count, `2` rate, `3` gauge; `0` (unspecified) or absent is a gauge |
| `points` | `{"timestamp": <unix seconds>, "value": <number>}` |
| `tags` | `key:value` strings |
| `resources` | The resource of type `host` is the host |

With a `Content-Type` containing `protobuf`, the body is read as the Agent's
protobuf `MetricPayload`.

### v1

```sh
curl -X POST http://localhost:8080/api/v1/series \
  -H "DD-API-KEY: $ROBINE_DOG_API_KEY" -H 'Content-Type: application/json' \
  -d "{\"series\": [{\"metric\": \"backup.duration\", \"type\": \"gauge\",
        \"points\": [[$(date +%s), 42.5]], \"host\": \"db-1\", \"tags\": [\"env:prod\"]}]}"
```

`type` is `count`, `rate` or `gauge` (the default); `points` are
`[<unix seconds>, <value>]` pairs; `host` is the host.

Both versions accept gzip, deflate and zstd bodies. Points whose value is not
finite, and metrics whose name is empty, longer than 200 characters or has
characters other than letters, digits, `.`, `_` and `-`, are dropped.

### Sketches

The Datadog Agent sends distributions as DDSketches to
`POST /api/beta/sketches`. Each bin of a sketch becomes a point weighted by
its count, at the bin's value (clamped to the sketch's exact minimum and
maximum), so percentiles are accurate to the Agent's sketch precision
(under 1% relative error). The metric is stored as a distribution.

## Storage

Every point is stored with its time, metric name, host, sorted tags, value
and weight (the number of samples it stands for, for distributions). Each
metric's type is recorded the first time it is seen and shown in
[Metrics Summary](/metrics/summary), where you can also set its unit and
description.

| Type | Over time, within a graph's interval | Across series |
| --- | --- | --- |
| count | Summed; `.as_count()` and `.as_rate()` apply | The query's aggregator (`sum:`, `avg:`…) |
| rate | Averaged (values are per second, as sent) | The query's aggregator |
| gauge | Averaged | The query's aggregator |
| distribution | Not pre-aggregated: computed over all the samples | `avg:`, `sum:`, `min:`, `max:`, `count:`, and `p50:`, `p75:`, `p90:`, `p95:`, `p99:` |

Tags are trimmed, deduplicated and sorted; a comma inside a tag becomes `_`.
Host tags from the Agent's metadata are added to series and sketches (not to
metrics received directly over DogStatsD); see
[Unified service tagging](tagging.md#host-tags-and-aliases).
[Metrics without Limits](metrics-without-limits.md) can drop tag keys before
the points are written.

Metrics are kept `ROBINE_DOG_METRICS_RETENTION_DAYS` days, 455 by default
(15 months). Buckets still in memory are written when the server stops
(`SIGTERM` or Ctrl-C).

## Differences from Datadog

- DogStatsD events (`_e{…}`) are ignored.
- Histograms compute `avg`, `median`, `95percentile`, `max` and `count`;
  the Agent's `histogram_aggregates` and `histogram_percentiles` settings
  have no equivalent.
- Series points are stored as sent, without the Agent's aggregation.
- DogStatsD has no origin detection and no UNIX socket: UDP only.
