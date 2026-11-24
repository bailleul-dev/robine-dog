---
title: Unified service tagging
section: Getting Started
order: 20
description: The service, env, version and host tags tie logs, traces and metrics together. How each intake sets them and how Robine Dog uses them to link one kind of data to another.
---

# Unified service tagging

Four tags identify where a piece of telemetry comes from:

| Tag | Meaning | Example |
| --- | --- | --- |
| `service` | The application or component | `service:payment-service` |
| `env` | The environment it runs in | `env:prod` |
| `version` | The release of the code | `version:3.2.0` |
| `host` | The machine, VM, node or container host | `host:web-1` |

When logs, traces and metrics carry the same values, you can go from one to
the other: from an error log to its trace, from a trace to its service page,
from a service to its trace metrics, from a deployment to the errors it
introduced. Set the same three variables in every application:

```sh
DD_SERVICE=payment-service
DD_ENV=prod
DD_VERSION=3.2.0
```

The Datadog tracers read them for spans, the tracers' log injection writes
them into logs, and the DogStatsD clients add them to metrics as tags.

## How each intake sets them

### Traces (ddtrace, Datadog Agent)

| Field | Comes from |
| --- | --- |
| `service` | The span's `service` (`DD_SERVICE`); a span without one is stored as `unnamed-service` |
| `env` | The span's `env` tag (`DD_ENV`); for Agent payloads, the Agent's `env` when the tracer set none |
| `version` | The span's `version` tag (`DD_VERSION`); for Agent payloads, the tracer payload's application version |
| `host` | The span's `_dd.hostname` tag; for Agent payloads, the Agent's host name |

`env` and `version` are stored as span columns, searched as `env:prod` and
`version:3.2.0` in the [Trace Explorer](trace-explorer.md).

### Logs (HTTP intake, Datadog Agent)

| Field | Comes from, in order |
| --- | --- |
| `service` | `service`, then `dd.service`, then a `service:` tag in `ddtags` |
| `host` | `hostname`, then `host` |
| `env` | An `env:` tag in `ddtags`, else the `dd.env` attribute |
| `version` | A `version:` tag in `ddtags`, else the `dd.version` attribute |
| `source` | `ddsource` (also added as a `source:` tag) |

The tracers' log injection (`DD_LOGS_INJECTION=true`) adds `dd.service`,
`dd.env`, `dd.version`, `dd.trace_id` and `dd.span_id` to each log record,
which is what fills these fields when the application logs JSON. The query
parameters `service`, `hostname` (or `host`), `ddsource` and `ddtags` set
defaults for every log of a request. See [Log collection](log-collection.md).

### Metrics (DogStatsD, series API)

`env`, `service` and `version` are ordinary tags. The `host` is separate:

| Intake | `host` comes from |
| --- | --- |
| DogStatsD | A `host:` tag, which is removed from the tags and becomes the host |
| `POST /api/v1/series` | The series' `host` field |
| `POST /api/v2/series` | The resource of type `host` in `resources` |
| Agent series and sketches | The host the Agent reports for each series |

In metric queries, `host` filters and groups like a tag:
`avg:system.cpu.user{env:prod} by {host}`. See [DogStatsD and the series API](dogstatsd.md).

### OpenTelemetry

The resource attributes map as Datadog's OTLP ingestion maps them:

| Resource attribute | Becomes |
| --- | --- |
| `service.name` | `service` (values starting with `unknown_service` are ignored) |
| `deployment.environment.name`, then `deployment.environment` | `env` |
| `service.version` | `version` |
| `host.name`, then `k8s.node.name` | `host` |

```sh
OTEL_SERVICE_NAME=payment-service
OTEL_RESOURCE_ATTRIBUTES=deployment.environment.name=prod,service.version=3.2.0
```

See [OpenTelemetry](opentelemetry.md).

### Sentry SDKs

| Sentry field | Becomes |
| --- | --- |
| The service named in the DSN path (`/sentry/<service>/1`), else a `service` tag, else `sentry-<project>` | `service` |
| `environment` | `env` |
| `release` | `version` |
| `server_name` | `host` |

See [Sentry SDKs](sentry.md).

## Host tags and aliases

The Datadog Agent describes its host in a metadata payload
(`POST /intake/`), sent at start and every few minutes. Robine Dog keeps two
things from it:

- **Host tags**: every list under `host-tags` (the Agent's own `system`
  tags, `DD_TAGS`, `env`, `kube_cluster_name`, cloud provider tags…).
- **Host aliases**: the other names of the host (`meta.host_aliases`), for
  example a Kubernetes node is also `<node>-<cluster>`.

At ingestion, for anything that names a host, Robine Dog does what Datadog
does:

1. A host name that is an alias is replaced by the real host name.
2. The host's tags are added, except for a tag key the data already has a
   value for. A log tagged `env:staging` on a host tagged `env:prod` stays
   `env:staging`.

This applies to logs from the HTTP intake, series (`/api/v1/series`,
`/api/v2/series`, the Agent's protobuf series and OTLP metrics), the Agent's
sketches, Sentry events and events. Hosts and their tags are stored in
PostgreSQL and reloaded at startup; a metadata payload without tags keeps the
tags already known.

> **Note:** host tags are not applied to spans, to DogStatsD metrics received
> directly on port 8125, or to OTLP logs. When the Agent forwards DogStatsD
> metrics, they arrive as series and get the host tags.

## How the tags link your data

| From | To | Through |
| --- | --- | --- |
| A log | Its trace | The log's `trace_id`, from `dd.trace_id` (64-bit decimal or 128-bit hex) |
| A span | Its logs | The **Logs** tab of the span, by trace id |
| A span | Its service page | `service` |
| A service | Its trace metrics | `trace.<operation>.hits`, `.errors` and the latency distribution, tagged `service`, `env`, `version`, `resource_name` |
| A service | Its deployments | Each `version` that served requests, on the service page |
| A host | Its metrics and logs | `host`, after alias resolution |

Trace ids are stored as the decimal value of their low 64 bits, so a log
carrying the 128-bit hex id of an OpenTelemetry or W3C trace finds the same
trace as one carrying the 64-bit decimal id.

## Reserved attributes

Some keys are columns rather than tags or attributes. They are filtered
without the `@` prefix and are the fastest to search:

| Logs | Spans |
| --- | --- |
| `service`, `host`, `source`, `status`, `trace_id`, `message` | `service`, `resource_name`, `operation_name`, `env`, `version`, `trace_id`, `span_id`, `parent_id`, `span.type`, `status`, `@duration` |

On logs, `env` and `version` are tags (`env:prod`). Everything else is an
attribute (`@http.status_code:500`). See [Search syntax](search-syntax.md).

## Differences from Datadog

- Tags are not lowercased: `env:Prod` and `env:prod` are two values. Metric
  tags are trimmed, deduplicated and sorted, and a comma inside one becomes
  `_`.
- Host tags are joined at ingestion and stored with the data. Changing a
  host's tags does not change data already received.
- Kubernetes and container tags come only from what the Agent puts on the
  data it forwards; Robine Dog has no tagger of its own.
