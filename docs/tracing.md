---
title: APM tracers
section: Send Data
order: 20
description: Send traces from the Datadog tracing libraries (ddtrace) to Robine Dog, which plays the local trace agent on port 8126, and connect them to your logs.
---

# APM tracers

The Datadog tracing libraries send their spans to a local Datadog Agent on
port 8126. Robine Dog serves the same trace agent API on that port, so a
traced application reports to it directly, with no Agent in between. You
can also keep an Agent and let it forward the traces; see
[Datadog Agent](agent.md).

Every span received feeds the [trace metrics](#trace-metrics) and the
span-based metrics; [retention filters](ingestion-control.md) then decide
which traces are stored and shown in the [Trace Explorer](trace-explorer.md).

## Configure your application

Point the tracer at port 8126 and set the [unified service tags](tagging.md):

```sh
DD_TRACE_AGENT_URL=http://localhost:8126
DD_SERVICE=my-api
DD_ENV=dev
DD_VERSION=1.4.2
DD_LOGS_INJECTION=true
```

`DD_AGENT_HOST=localhost` (with `DD_TRACE_AGENT_PORT=8126`, the default)
works too. Then start the application the way the tracer for its language
expects:

| Language | Library | Start |
| --- | --- | --- |
| Python | `ddtrace` | `ddtrace-run python app.py` |
| Node.js | `dd-trace` | `node --require dd-trace/init app.js` |
| Java | `dd-java-agent.jar` | `java -javaagent:dd-java-agent.jar -jar app.jar` (or `-Ddd.trace.agent.url=http://localhost:8126`) |
| Go | `dd-trace-go` | `tracer.Start()` in your code; it reads the same variables |
| Ruby | `datadog` | Your usual start command, with the gem's instrumentation loaded |
| .NET | `Datadog.Trace` | The .NET tracer's environment, plus the variables above |
| PHP | `dd-trace-php` | The PHP extension, plus the variables above |

For example, in Python:

```sh
DD_TRACE_AGENT_URL=http://localhost:8126 DD_SERVICE=my-api DD_ENV=dev \
DD_LOGS_INJECTION=true ddtrace-run python app.py
```

> **Note:** the trace port has no key by default, like an Agent's. Keep it
> on a trusted network, or set `ROBINE_DOG_TRACE_REQUIRE_KEY`; see
> [API and application keys](api-keys.md).

## Connect logs and traces

With `DD_LOGS_INJECTION=true`, the tracers' logging integrations add
`dd.trace_id`, `dd.span_id`, `dd.service`, `dd.env` and `dd.version` to every
log record written during a span. Send those logs to the
[logs intake](log-collection.md) (as JSON, or through a pipeline that
extracts the ids) and Robine Dog links them both ways:

- In the Log Explorer, a log with a trace id has a **View Trace** button.
- In the trace view, the **Logs** tab of a span lists the logs of its trace.

Trace ids in logs can be the 64-bit decimal form or the 128-bit hexadecimal
form (32 characters); both find the same trace.

## Trace agent API

| Endpoint | Format | Response |
| --- | --- | --- |
| `PUT`/`POST /v0.3/traces` | msgpack, or JSON with a JSON `Content-Type` | `{"rate_by_service": {"service:,env:": 1.0}}` |
| `PUT`/`POST /v0.4/traces` | msgpack, or JSON | Same |
| `PUT`/`POST /v0.5/traces` | msgpack with a string table | Same |
| `GET /info` | | The endpoints served, as a 7.60 Agent describes them |
| `POST /v0.6/stats` | | `202`, dropped: Robine Dog computes stats from the spans |

The `rate_by_service` answer tells the tracers to keep 100% of traces, and
`/info` reports `client_drop_p0s: false`, so the tracers send every trace,
including the ones they sampled out. The request body can be up to 64 MiB.

The trace port also serves what tracers send to an Agent for other products:

| Endpoint | Product |
| --- | --- |
| `/profiling/v1/input` | [Continuous Profiler](profiler.md) (`DD_PROFILING_ENABLED=true`) |
| `/v0.1/pipeline_stats` | [Data Streams Monitoring](data-streams.md) (`DD_DATA_STREAMS_ENABLED=true`) |
| `/telemetry/proxy/...` | The libraries each service loads, for [Code Security](code-security.md) |
| `/debugger/v1/input`, `/debugger/v2/input` | Exception Replay snapshots, stored as `source:dd_debugger` logs, for [Error Tracking](error-tracking.md) |
| `/evp_proxy/v2/...`, `/evp_proxy/v4/...` | [LLM Observability](llm-observability.md), [LLM Experiments](llm-experiments.md) and test events for [CI Visibility](ci-visibility.md) |

## What happens to a span

1. **Decoding.** The span's ids are kept as unsigned 64-bit decimals. A
   span without a service is stored as `unnamed-service`; a span without a
   resource takes its operation name as resource. `env` and `version` come
   from the span's tags of the same name.
2. **Service entry spans.** A span is a service entry span (Datadog's
   "top level") when the tracer marked it with `_top_level` or
   `_dd.top_level`, when it is a root, when its parent is not in the same
   payload, or when its parent belongs to another service. These are the
   spans that the service pages and the trace metrics count.
3. **Metrics.** The trace metrics and the
   [span-based metrics](generate-metrics.md) are computed from every span,
   before any retention decision. Attacks recorded by the tracers' WAF are
   saved as logs for [App and API Protection](app-and-api-protection.md).
4. **Retention.** The [retention filters](ingestion-control.md) are matched
   against the trace, in order. A trace is kept when one of its spans
   matches an enabled filter and the trace falls within that filter's rate,
   decided on the trace id. Until you change them, the default filters keep
   everything.
5. **Storage.** Kept spans are written in batches and kept
   `ROBINE_DOG_RETENTION_DAYS` days (15 by default).

### Sampling priority

The tracer's sampling priority (`_sampling_priority_v1`) and decision maker
(`_dd.p.dm`) are recorded and shown in
[Ingestion Control](ingestion-control.md), but they do not decide what is
stored: a trace the tracer sampled out (priority 0 or below) is kept if a
retention filter keeps it.

## Trace metrics

Each service entry span adds to three metrics, named after its operation:

| Metric | Type | Value |
| --- | --- | --- |
| `trace.<operation>.hits` | count | 1 per span |
| `trace.<operation>.errors` | count | 1 per span in error, 0 otherwise |
| `trace.<operation>` | distribution | The span's duration, in seconds |

They are tagged `service`, `resource_name`, `env` and `version` (when the
span has them) and `http.status_code` (when the span has one), and their host
is the span's `_dd.hostname`. They are aggregated in 10-second buckets like
DogStatsD metrics.

```text
sum:trace.http.request.hits{service:shop-api,env:prod} by {resource_name}.as_count()
p95:trace.servlet.request{service:payment-service}
```

Use them in [dashboards](dashboards.md) and [monitors](monitors.md) like
any metric. The usage metrics `datadog.estimated_usage.apm.*` are described
in [Ingestion Control](ingestion-control.md).

## Differences from Datadog

- Robine Dog does not run the Agent's samplers: tracers are told to keep
  every trace, and only the retention filters decide what is stored.
- Trace metrics are computed from the spans received, not from the
  tracers' or the Agent's client-side stats, which are dropped.
- Trace ids are stored on their low 64 bits; the high 64 bits of a 128-bit
  id stay in the `_dd.p.tid` tag the tracer sets.
