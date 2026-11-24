---
title: Log collection
section: Send Data
order: 30
description: Send logs to Robine Dog's HTTP intake, as you would to Datadog's, from the Datadog Agent, a script or any HTTP client, and how reserved attributes are remapped.
---

# Log collection

Robine Dog's logs intake is Datadog's HTTP logs API: `POST /api/v2/logs`
on the main port. The Datadog Agent, Datadog's serverless forwarders, log
shippers with a Datadog output, and plain `curl` can send to it unchanged.
Logs also arrive from [OpenTelemetry](opentelemetry.md) and
[Sentry SDKs](sentry.md).

Once received, a log is remapped (this page), enriched with its
[host tags](tagging.md#host-tags-and-aliases), parsed by the
[log pipelines](log-pipelines.md), scanned by the
[Sensitive Data Scanner](sensitive-data-scanner.md), counted by
[log-based metrics](generate-metrics.md), forwarded to
[custom destinations](log-forwarding.md), written to
[archives](log-archives.md), and stored if an [index](log-indexes.md) keeps
it.

## Send logs over HTTP

```sh
curl -X POST http://localhost:8080/api/v2/logs \
  -H "DD-API-KEY: $ROBINE_DOG_API_KEY" -H 'Content-Type: application/json' \
  -d '[{"message": "payment failed", "service": "payment-service", "ddsource": "java",
        "ddtags": "env:prod,team:payments", "hostname": "web-1", "status": "error",
        "http": {"status_code": 502}}]'
```

The intake answers `202 Accepted` with `{}`.

| Request part | Accepted |
| --- | --- |
| Path | `POST /api/v2/logs`; `/v1/input` is accepted as well |
| Key | `DD-API-KEY` header, or `dd-api-key` / `api_key` query parameter (see [API and application keys](api-keys.md)) |
| Body | A JSON array of objects, or a single JSON object |
| Compression | `Content-Encoding: gzip`, `deflate` or `zstd` |
| Size | Up to 64 MiB per request |
| Query parameters | `ddsource`, `service`, `hostname` (or `host`) and `ddtags` set defaults for every log of the request |

In a JSON array, a string item is a log whose message is that string, and
empty objects are skipped. When the `Content-Type` is not JSON and the body
is not valid JSON, each non-empty line of the body is a log:

```sh
printf 'GET /health 200\nGET /cart 500\n' | curl -X POST \
  "http://localhost:8080/api/v2/logs?ddsource=nginx&service=web-store" \
  -H "DD-API-KEY: $ROBINE_DOG_API_KEY" -H 'Content-Type: text/plain' --data-binary @-
```

| Response | Meaning |
| --- | --- |
| `202` `{}` | Accepted |
| `400` `{"errors": ["Malformed JSON"]}` | A JSON `Content-Type` with a body that is not JSON |
| `400` `{"errors": ["Bad compression"]}` | The body does not decompress |
| `403` `{"errors": ["Forbidden"]}` | Missing or unknown API key |

## Send logs from the Datadog Agent

Point the Agent's logs at Robine Dog over HTTP:

```sh
DD_LOGS_ENABLED=true
DD_LOGS_CONFIG_USE_HTTP=true
DD_LOGS_CONFIG_LOGS_DD_URL=localhost:8080
DD_LOGS_CONFIG_LOGS_NO_SSL=true
```

The Agent then sends its file, journald and container logs to
`/api/v2/logs`, compressed. See [Datadog Agent](agent.md) for the full
configuration. The Agent sets `status: info` on lines it tails; a level
inside a JSON line overrides it (see below).

## Reserved attributes

Robine Dog reads these keys from each log, removes them from the attributes
and stores them as columns, as Datadog's default remappers do:

| Field | Keys read, in order | Notes |
| --- | --- | --- |
| Message | `message`, else `msg`, else `log` | |
| Date | `timestamp`, `date`, `@timestamp`, `time`, `_timestamp`, `published_date` | Epoch seconds, milliseconds, microseconds or nanoseconds (number or numeric string), or RFC 3339. Without one, the time of receipt |
| Host | `hostname`, then `host` | Then the `hostname` (or `host`) query parameter |
| Service | `service`, then `dd.service`, then a `service:` tag | Then the `service` query parameter |
| Source | `ddsource` | Then the query parameter; also added as a `source:` tag |
| Status | `status`, then `level`, `severity`, `loglevel`, `log_level`, `levelname` | Normalized, see below. Default `info` |
| Tags | `ddtags` (comma-separated) | Added to the query parameter's tags |
| Trace id | `dd.trace_id`, then `trace_id` | 64-bit decimal, or 128-bit hex (its low 64 bits are kept) |
| Span id | `dd.span_id`, then `span_id` | |

`dd.env` and `dd.version` become `env:` and `version:` tags unless `ddtags`
already has one.

### Status values

Any spelling of a level maps to one of Datadog's statuses:

| Value | Status |
| --- | --- |
| `0` to `7` (syslog severity) | `emergency`, `alert`, `critical`, `error`, `warn`, `notice`, `info`, `debug` |
| Starts with `emerg`, `f` (fatal) or `panic` | `emergency` |
| Starts with `a`, `c`, `e`, `w`, `n`, `i` | `alert`, `critical`, `error`, `warn`, `notice`, `info` |
| Starts with `d`, `t` (trace) or `v` (verbose) | `debug` |
| Starts with `o` or `s` (ok, success) | `ok` |
| Anything else | `info` |

The comparison ignores case: `WARNING`, `Warn` and `w` are all `warn`.

### Nested attributes

Keys with dots are expanded into objects, so `@http.status_code` always means
one path:

```json
{"message": "done", "http.status_code": 200, "usr": {"id": "42"}}
```

is stored with the attributes `{"http": {"status_code": 200}, "usr": {"id": "42"}}`.

## JSON messages

When the message itself is a JSON object (it starts with `{`), as when an
application logs JSON to a file the Agent tails, Robine Dog parses it like
Datadog's JSON preprocessing:

- its keys are merged into the log, over the envelope's;
- the message is then read from its `message`, `msg` or `log` key;
- a level inside it (`status`, `level`, `severity`…) wins over the
  envelope's `status`, which the Agent always sets;
- for Rust's `tracing-subscriber` JSON layout, the text in
  `fields.message` becomes the message.

```json
{"message": "{\"level\":\"ERROR\",\"msg\":\"charge failed\",\"dd.trace_id\":\"8235107843296210325\",\"amount\":12.5}", "status": "info", "service": "payment-service"}
```

is stored as an `error` log with the message `charge failed`, linked to
trace `8235107843296210325`, with the attribute `amount: 12.5`.

Messages that are plain text are kept as they are: parse them with a
[grok parser](log-pipelines.md) in a pipeline.

## Correlate logs with traces

With `DD_LOGS_INJECTION=true`, the ddtrace logging integrations add
`dd.trace_id` and `dd.span_id` to every log written during a span. Robine Dog
reads them as shown above and shows a **View Trace** button on those logs.
See [APM tracers](tracing.md#connect-logs-and-traces).

## Differences from Datadog

- Datadog's per-request and per-log size limits are not enforced; a request
  can be up to 64 MiB.
- The status prefixes are matched loosely: any word starting with `w` is
  `warn`, any word starting with `s` is `ok`.
- The intake writes in batches (every 200 ms or 5,000 rows); a log is
  searchable a fraction of a second after the `202`.
