---
title: OpenTelemetry
section: Send Data
order: 50
app: /organization-settings/opentelemetry
description: Send traces, logs and metrics from OpenTelemetry SDKs or the OpenTelemetry Collector over OTLP/HTTP, mapped the way Datadog's OTLP ingestion maps them.
---

# OpenTelemetry

Robine Dog accepts OTLP over HTTP on its main port, from OpenTelemetry SDKs
directly or from an OpenTelemetry Collector. It maps OpenTelemetry data to
Datadog's model the way Datadog's OTLP ingestion does: resource attributes
become `service`, `env`, `version` and `host`, span names follow the
semantic conventions, cumulative counters become deltas. The result is
searched, graphed and alerted on like data from the Datadog libraries.

The [OpenTelemetry](/organization-settings/opentelemetry) page of the
interface shows both configurations below with your instance's address.

## Endpoints

| Signal | Endpoint |
| --- | --- |
| Traces | `POST /v1/traces` |
| Logs | `POST /v1/logs` |
| Metrics | `POST /v1/metrics` |

| Request part | Accepted |
| --- | --- |
| Encoding | Protobuf (`application/x-protobuf`), or OTLP/JSON when the `Content-Type` contains `json` |
| Compression | `Content-Encoding: gzip`, `deflate` or `zstd`, or none |
| Key | A `dd-api-key` header, or a `dd-api-key` query parameter (see [API and application keys](api-keys.md)) |
| Size | Up to 64 MiB per request |

| Response | Meaning |
| --- | --- |
| `200` | Accepted: an empty protobuf message, or `{}` in JSON |
| `400` | The body does not decode or decompress |
| `403` | Missing or invalid key |
| `503` | Storage error; the exporter retries |

Only OTLP/HTTP is served. To send OTLP/gRPC, put a Collector in between.

## Configure an SDK

Point the exporter at the main port and pass the key as a header:

```sh
OTEL_EXPORTER_OTLP_PROTOCOL=http/protobuf
OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:8080
OTEL_EXPORTER_OTLP_HEADERS="dd-api-key=$ROBINE_DOG_API_KEY"
OTEL_SERVICE_NAME=my-api
OTEL_RESOURCE_ATTRIBUTES=deployment.environment.name=prod,service.version=1.2.3
```

The SDK appends `/v1/traces`, `/v1/logs` and `/v1/metrics` to the endpoint.
For example, with Python's auto-instrumentation:

```sh
OTEL_EXPORTER_OTLP_PROTOCOL=http/protobuf \
OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:8080 \
OTEL_EXPORTER_OTLP_HEADERS="dd-api-key=$ROBINE_DOG_API_KEY" \
OTEL_SERVICE_NAME=my-api OTEL_RESOURCE_ATTRIBUTES=deployment.environment.name=dev \
opentelemetry-instrument python app.py
```

## Configure the Collector

Receive OTLP on gRPC and HTTP, and export to Robine Dog with the
`otlphttp` exporter:

```yaml
receivers:
  otlp:
    protocols:
      grpc:
      http:
processors:
  batch:
exporters:
  otlphttp/robine-dog:
    endpoint: http://localhost:8080
    headers:
      dd-api-key: ${env:ROBINE_DOG_API_KEY}
service:
  pipelines:
    traces:
      receivers: [otlp]
      processors: [batch]
      exporters: [otlphttp/robine-dog]
    metrics:
      receivers: [otlp]
      processors: [batch]
      exporters: [otlphttp/robine-dog]
    logs:
      receivers: [otlp]
      processors: [batch]
      exporters: [otlphttp/robine-dog]
```

## Resource attributes

| Resource attribute | Becomes |
| --- | --- |
| `service.name` | `service`. Values starting with `unknown_service` are ignored; spans without a service are stored as `otlpresourcenoservicename` |
| `deployment.environment.name`, then `deployment.environment` | `env` |
| `service.version` | `version` |
| `host.name`, then `k8s.node.name` | `host` |

The other resource attributes are kept on every span and log record, next to
its own attributes.

## Traces

| OpenTelemetry | Robine Dog span |
| --- | --- |
| Trace id (128 bits) | `trace_id`: its low 64 bits, in decimal. The high 64 bits go to the `_dd.p.tid` tag; the full id is in `otel.trace_id` |
| Span id, parent span id | `span_id`, `parent_id`, in decimal |
| Span kind | `span.kind` tag (`server`, `client`, `producer`, `consumer`, `internal`) |
| Status `ERROR` | The span is an error; the status message is `error.message` |
| `exception` event | `exception.type`, `exception.message`, `exception.stacktrace` become `error.type`, `error.message`, `error.stack` |
| String attributes | Tags |
| Numeric attributes | Span metrics |
| Instrumentation scope | `otel.library.name`, `otel.library.version` |
| Status code | `otel.status_code` (`Unset`, `Ok`, `Error`) |

A span is a service entry span when it is a root, or its kind is `server`
or `consumer`.

### Operation name, resource and type

| Span | Operation name | Resource | Type |
| --- | --- | --- | --- |
| An `operation.name` attribute | Its value | | |
| A `resource.name` attribute | | Its value | |
| HTTP server (`http.request.method` or `http.method`) | `http.server.request` | `<METHOD> <http.route>`, or the method alone | `web` |
| HTTP client | `http.client.request` | The method | `http` |
| Database client (`db.system.name` or `db.system`) | `<db.system>.query` | `db.query.text` or `db.statement`, else the span name | `cache` for `redis` and `memcached`, else `sql` |
| Messaging (`messaging.system`) | `<system>.<messaging.operation.type>` | The span name | `queue` |
| RPC (`rpc.system`) | `<system>.<kind>.request` | The span name | |
| Other server or client spans | `<scope name>.<kind>` (`opentelemetry.<kind>` without a scope) | The span name | `web` for servers |
| Anything else | The span name | The span name | `custom` |

Spans then go through the same steps as spans from the Datadog tracers:
trace metrics, span-based metrics and [retention filters](ingestion-control.md).
See [APM tracers](tracing.md#trace-metrics).

## Logs

| OpenTelemetry | Robine Dog log |
| --- | --- |
| Body | `message` (a structured body is stored as JSON text) |
| Severity text, else severity number | `status` (numbers 1–4 trace, 5–8 debug, 9–12 info, 13–16 warn, 17–20 error, 21–24 fatal) |
| Time, else observed time | The log's date |
| Trace id, span id | `dd.trace_id`, `dd.span_id` (low 64 bits), and `otel.trace_id` |
| `exception.type`, `exception.message`, `exception.stacktrace` | `error.kind`, `error.message`, `error.stack` |
| Severity number | `otel.severity_number` |
| Resource and record attributes | Attributes; dotted keys become nested objects |

The source is `otlp_log_ingestion`. Logs then go through the usual
processing: [pipelines](log-pipelines.md), indexes, archives. Logs with a
trace id link to their trace like logs with `dd.trace_id`.

## Metrics

| OpenTelemetry metric | Stored as |
| --- | --- |
| Gauge | A gauge |
| Sum, monotonic | A count; cumulative sums are turned into deltas |
| Sum, not monotonic | A gauge |
| Histogram, exponential histogram | `<name>.count` and `<name>.sum` (counts, deltas when cumulative), `<name>.min` and `<name>.max` (gauges) when present |
| Summary | `<name>.count` and `<name>.sum` (counts, deltas), and `<name>.quantile` (a gauge tagged `quantile:<q>`) |

Metric names are kept as they are. The tags of a point are its attributes
plus `service`, `env` and `version` from the resource; the host is the
resource's host.

### Cumulative temporality

The Datadog Agent stores counters as deltas, and so does Robine Dog. For a
cumulative sum, histogram or summary, Robine Dog remembers the last value of
each series (by host, service, name and tags) and stores the difference:

- A value lower than the previous one, or a new start time, is a reset: the
  new value is stored as the delta.
- The first point of a series counts in full when the counter started after
  Robine Dog did; otherwise it only sets the baseline (Datadog's
  `initial_cumulative_monotonic_value: auto`).

The last values are kept in memory, so after a restart the first point of
each series sets the baseline again. Delta temporality is stored as sent.

## Differences from Datadog

- OTLP/gRPC is not served; use a Collector.
- Histogram buckets are not kept: only count, sum, min and max. Use a
  Datadog distribution (DogStatsD `d`) for percentiles.
- Host tags from the Agent's metadata are applied to OTLP metrics, not to
  OTLP spans and logs.
- Span links and span events other than `exception` are dropped.
