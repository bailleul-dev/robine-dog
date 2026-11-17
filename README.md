# robine-dog

A personal, self-hosted log, trace and metrics explorer to practise Datadog's
Logs, APM and Metrics workflows on real data: the same search syntax, facets,
explorers, trace views, service pages and metric queries, fed by the **real
Datadog clients** (ddtrace libraries, DogStatsD, the logs and series HTTP
intakes). Sign-in is through Robine ID.

Rust (axum) + React (Vite, TanStack Query). One binary serves the UI, the API
and every intake. Telemetry (logs, spans, metric points) is stored in
[GreptimeDB](https://greptime.com) standalone, used only as a SQL store (table
TTLs, full-text and inverted indexes, JSON columns); the application's own
state (facets, metric metadata) is in PostgreSQL.

## What works

| Datadog | robine-dog |
| --- | --- |
| Log Explorer: search bar, facets, volume histogram (drag to zoom), log side panel, attribute actions (filter / exclude / create facet) | `/logs` |
| Live Tail | `/logs/livetail` |
| Trace Explorer: span search, facets, Service Entry / Root / All spans, slowest | `/apm/traces` |
| Trace view: flame graph, waterfall, span list, span tags/metrics, errors, correlated logs | side panel on a span |
| Service Catalog and service page: requests, errors, p50/p95/p99/max, latency distribution, resources, downstream services | `/apm/services` |
| Log ↔ trace correlation (`dd.trace_id` / `dd.span_id`, 64-bit or 128-bit) | "View Trace", span "Logs" tab |
| Search syntax: `AND`/`OR`/`NOT`/`-`, `( )`, `key:value`, `@attr:value`, `key:val*`, `key:*`, `@n:>=500`, `@n:[1 TO 5]`, `@duration:>1.5s`, `key:(a OR b)`, `"phrases"` | everywhere |
| Logs HTTP intake `POST /api/v2/logs` (JSON, gzip/deflate, `DD-API-KEY`), status/host/service/date remapping, JSON messages parsed | port 8080 |
| Trace agent API `/v0.3`, `/v0.4`, `/v0.5/traces` (msgpack/JSON), `/info` | port 8126 |
| Metrics Explorer: `avg:metric{tag:a,!tag:b} by {host}.as_count()`, `.as_rate()`, `.rollup(max, 60)`, p50…p99 on distributions, several queries, lines/bars | `/metrics/explorer` |
| Metrics Summary: type, unit, description (editable), tags and hosts per metric | `/metrics/summary` |
| DogStatsD (`c`, `g`, `s`, `h`/`ms`, `d`, sample rates, `#tags`, `\|T` timestamps), aggregated into 10-second buckets | UDP 8125 |
| Series intake `POST /api/v1/series`, `/api/v2/series` (JSON), `/api/v1/validate` | port 8080 |
| Analytics ("Visualize as" Timeseries / Top List / Table): count, unique count, avg/sum/min/max/median/pcNN of a measure, by up to two facets (top N per parent, like Datadog) | Log and Trace Explorers |
| Log Patterns: the latest 10,000 matching logs clustered by message shape (Drain), wildcards highlighted, click to search | Log Explorer |
| Aggregate API `/api/v2/logs/analytics/aggregate`, `/api/v2/spans/analytics/aggregate` (Datadog's request and bucket format) | port 8080 |
| Dashboards: Datadog's dashboard JSON (import / export), 12-column grid, timeseries (lines, bars), query value with conditional formatting, top list, table, note (Markdown) and group widgets, template variables (`$service`), widget editor with live preview | `/dashboard/lists` |
| Queries and formulas: metrics, logs and spans queries on one time grid, `a / b * 100`, `per_second`, `cumsum`, `diff`, `derivative`, `abs`, `log10`, `clamp_min/max`, `default_zero`, `timeshift`, `hour_before`… `week_before`, `top(a, 10, 'mean', 'desc')`; scalar queries reduce first, as Datadog | `/api/v2/query/timeseries`, `/api/v2/query/scalar` |
| Dashboards API `/api/v1/dashboard` (list, CRUD) | port 8080 |
| Saved views of the Log and Trace explorers (query, facets, visualization, time) | `/api/ui/views` |
| Generate Metrics: log- and span-based counts and distributions, tagged by attributes, computed at ingestion | `/api/v2/logs/config/metrics`, `/api/v2/apm/config/metrics` |
| OpenTelemetry: OTLP/HTTP traces, logs and metrics (protobuf and JSON, gzip), with Datadog's OTLP mapping | `/v1/traces`, `/v1/logs`, `/v1/metrics` |
| Service Map: services laid out by call depth, edges from real parent / child spans across services (calls, error rate, latency), health colors, neighbors on hover, details panel | `/apm/map` |
| Error Tracking: error spans (`error.type`, `error.message`, `error.stack`) and error logs (`error.kind`, `error.stack`) grouped into issues by service, type, masked message and top stack frame; volume, first / last seen, stack trace with app frames highlighted, breakdown by version, env, host and resource, links to the trace or log; For Review / Reviewed / Ignored / Resolved, with regressions | `/error-tracking` |
| Sentry SDKs: envelope and store endpoints, the DSN's key being a client token; events become error logs (`error.kind`, `error.message`, `error.stack`, `usr.*`, `http.*`, trace ids) and Error Tracking issues | `/api/<project>/envelope/`, `/sentry/<service>/api/<project>/envelope/` |
| Monitors: metric (`avg(last_5m):avg:system.cpu.user{*} by {host} > 80`), logs (`logs("…").rollup(…).by(…).last("5m") > 10`) and APM (`trace-analytics(…)`) queries, evaluated every minute per group, OK / Warn / Alert / No Data, recovery thresholds, renotify, mute by scope, priority, event history and value graph, Manage / Triggered Monitors pages, an editor with live evaluation | `/monitors` |
| Events: monitor state changes, Agent events and `POST /api/v1/events`, searched like logs (`source:alert status:error @monitor.id:12`) | `/api/v1/events` |
| Metrics volume (points and series stored per metric) and the integrations reporting, found from log sources and metric prefixes | — |
| Real User Monitoring: Datadog's browser SDK (`proxy: "https://<robine-dog>/rum/proxy"`, the client token as key): views with their web vitals, actions, resources, errors (also filed in Error Tracking), long tasks; browser logs through the same proxy. Sites install the SDK as Datadog documents it (npm or its CDN), with `proxy` added | `/rum/proxy`, `/api/v2/rum` |
| Watchdog: every five minutes each service's last quarter of an hour is compared with its previous day (error logs, failed requests, p95 latency); anomalies open a story and post a `source:watchdog` event | — |
| Notebooks: text, metric graphs and log graphs in cells | — |
| Incidents: severity, state, commander, a timeline of notes and changes; declared by hand or from a Watchdog story; each change of state posts a `source:incident` event | — |
| Status page: public at `/status` once enabled; components take their state from a synthetic test or a monitor, and incidents marked public are listed | — |
| MCP server, as Datadog's: streamable HTTP at `/v1/mcp`, the API key in `DD_API_KEY` (or `Authorization: Bearer`), the tools of Datadog's `core` toolset under their names (`search_datadog_logs`, `get_datadog_metric`, `search_datadog_spans`, `get_datadog_trace`, `search_datadog_monitors`, `search_datadog_incidents`, notebooks…) | `/v1/mcp` |
| Downtimes: scheduled silences by monitor, monitor tags and scope | `/api/v1/downtime` |
| Infrastructure: hosts, containers and Kubernetes resources (pods, deployments, stateful sets, daemon sets, nodes, namespaces), from the Agent's `system.*`, `container.*`, `kubernetes.*` and `kubernetes_state.*` metrics | — |
| Synthetic API tests: HTTP requests on a schedule with assertions (status code, response time, body, headers); each run is a `source:synthetics` log and the `synthetics.http.response.time` and `synthetics.test_runs` metrics | `/api/v1/synthetics/tests` |
| SLOs, metric-based: good over total events against a target per time frame, error budget | `/api/v1/slo` |
| Notifications: Datadog message templates (`{{#is_alert}}`, `{{value}}`, `{{host.name}}`, `{{#is_match}}`…) to `@webhook-<name>` (Webhooks integration format: `$EVENT_TITLE`… payloads, custom headers, JSON or form) | `/monitors/webhooks` |
| Monitors and webhooks APIs `/api/v1/monitor` (CRUD, partial update, validate, mute/unmute), `/api/v1/integration/webhooks/configuration/webhooks` | port 8080 |
| Log Pipelines: grok parser (Datadog matchers and filters, helper rules, live sample preview), date / status / service / message / trace id / span id remappers, attribute remapper, URL and user-agent parsers, category, arithmetic, string builder and lookup processors, nested pipelines; read-only integration pipelines (Nginx, PostgreSQL, Kubernetes, Traefik, CoreDNS, Redis, Datadog Agent, Go logfmt / klog) | `/logs/pipelines`, applied at intake |
| Logs pipelines API `/api/v1/logs/config/pipelines`, `/pipeline-order` (Datadog's JSON, `DD-API-KEY`) | port 8080 |
| Host tags and host aliases from the Agent's metadata (`env`, `kube_cluster_name`, `DD_TAGS`…), joined to the logs and metrics of that host as Datadog does | `/intake/`, applied at ingestion |
| Datadog Agent: protobuf traces `/api/v0.2/traces` (classic and indexed formats), protobuf series, sketches `/api/beta/sketches`, logs, zstd | port 8080 |
| Trace metrics `trace.<span>.hits`, `.errors` and `trace.<span>` (distribution, seconds), tagged `service`, `resource_name`, `env`, `version`, `http.status_code` | computed at ingestion |


## Run it

Requirements: Rust 1.85+, Node 22+, Docker (for the databases).

```sh
cp .env.example .env    # then fill in the Robine ID client (see below)
make dev                # databases (docker compose), server :8080 + :8126 + udp :8125, UI http://localhost:5173
make demo               # in another terminal: an hour of history, then live traffic
```

`make deps` starts GreptimeDB (`127.0.0.1:14000`) and PostgreSQL
(`127.0.0.1:15432`) alone; `make deps-down` stops them, keeping their volumes.

Without Robine ID, on loopback only: `make dev-noauth`.

`make demo` simulates a small shop (web-store → shop-api → cart-service /
payment-service → Postgres, Redis, Stripe) with ~8% declined payments and ~3%
provider timeouts, plus host and business metrics over DogStatsD, using the
same wire formats as ddtrace, the Agent and DogStatsD.

## Sign-in with Robine ID

robine-dog is a confidential OIDC client: authorization code with PKCE S256
and a nonce, `client_secret_post`, ID token RS256-verified against the JWKS,
then a 12-hour HMAC-signed session cookie. Only the `sub` values listed in
`ROBINE_DOG_OIDC_SUBJECTS` are let in.

The clients `robine-dog` and `robine-dog-dev` are declared in silo01's
`robine_id_applications`; production deployment is described in
[`deploy/README.md`](deploy/README.md). The callback is
`<ROBINE_DOG_BASE_URL>/auth/callback` (`http://localhost:5173/auth/callback` in
development).

## Send it real data

**Traces** — any ddtrace library, robine-dog plays the local agent:

```sh
DD_TRACE_AGENT_URL=http://localhost:8126 DD_SERVICE=my-api DD_ENV=dev \
DD_LOGS_INJECTION=true ddtrace-run python app.py
```

(Node: `DD_TRACE_AGENT_URL=… node --require dd-trace/init app.js`; Java:
`-javaagent:dd-java-agent.jar -Ddd.trace.agent.url=…`; Go: `DD_TRACE_AGENT_URL`.)

**Logs** — the HTTP intake, like Datadog's:

```sh
curl -X POST http://localhost:8080/api/v2/logs \
  -H "DD-API-KEY: $ROBINE_DOG_API_KEY" -H 'Content-Type: application/json' \
  -d '[{"message":"payment failed","service":"pay","ddsource":"python",
        "ddtags":"env:dev,team:pay","status":"error","http":{"status_code":502}}]'
```

From a Datadog Agent: `DD_LOGS_CONFIG_USE_HTTP=true`,
`DD_LOGS_CONFIG_LOGS_DD_URL=<host>:8080`, `DD_LOGS_CONFIG_LOGS_NO_SSL=true`.

With `DD_LOGS_INJECTION=true`, the ddtrace log integrations add
`dd.trace_id`/`dd.span_id` to every log, which links logs and traces both ways.

**Metrics** — any DogStatsD client, robine-dog plays the Agent's DogStatsD
server:

```sh
DD_DOGSTATSD_URL=udp://localhost:8125     # or DD_AGENT_HOST + DD_DOGSTATSD_PORT
echo -n 'shop.orders:1|c|#env:dev,payment_method:card' | nc -u -w0 localhost 8125
```

Or the series API, as the Agent or a script posts it:

```sh
curl -X POST http://localhost:8080/api/v2/series -H "DD-API-KEY: $ROBINE_DOG_API_KEY" \
  -H 'Content-Type: application/json' \
  -d "{\"series\":[{\"metric\":\"queue.depth\",\"type\":3,\"points\":[{\"timestamp\":$(date +%s),\"value\":12}],
        \"tags\":[\"env:dev\"],\"resources\":[{\"type\":\"host\",\"name\":\"worker-1\"}]}]}"
```

**The Datadog Agent** — point every endpoint at robine-dog and give it a
site that doesn't resolve, so whatever is not overridden (process,
inventories, remote config…) can't reach Datadog with your key:

```sh
docker run -d --name dd-agent \
  -e DD_API_KEY=$ROBINE_DOG_API_KEY -e DD_SITE=robine-dog.invalid \
  -e DD_DD_URL=http://robine-dog:8080 \
  -e DD_APM_DD_URL=http://robine-dog:8080 \
  -e DD_LOGS_ENABLED=true -e DD_LOGS_CONFIG_USE_HTTP=true \
  -e DD_LOGS_CONFIG_LOGS_DD_URL=robine-dog:8080 -e DD_LOGS_CONFIG_LOGS_NO_SSL=true \
  -e DD_PROCESS_AGENT_ENABLED=false -e DD_PROCESS_CONFIG_PROCESS_COLLECTION_ENABLED=false \
  -e DD_PROCESS_CONFIG_CONTAINER_COLLECTION_ENABLED=false \
  -e DD_REMOTE_CONFIGURATION_ENABLED=false -e DD_INSTRUMENTATION_TELEMETRY_ENABLED=false \
  -e DD_INVENTORIES_ENABLED=false -e DD_SBOM_ENABLED=false \
  -p 8126:8126 -p 8125:8125/udp datadog/agent:7
```

In Kubernetes, the same Agent as a DaemonSet connects a whole cluster:
silo01's `datadog_agent` role is one (container logs by namespace, kubelet
and kube-state metrics, unified service tagging labels, OTLP), and what
robine-dog runs against in production.

Applications then talk to the Agent as usual (APM on 8126, DogStatsD on 8125);
it forwards traces, series, sketches and logs to robine-dog. Endpoints
robine-dog doesn't use (`/intake/`, check runs, metadata…) are acknowledged.

**OpenTelemetry** — OTLP/HTTP (protobuf or JSON) on the main port, mapped
as Datadog's OTLP ingestion maps it (`service.name`, `deployment.environment.name`,
`service.version`, `host.name`; operation and resource names from the semantic
conventions; exceptions become `error.*`; cumulative counters become deltas):

```sh
OTEL_EXPORTER_OTLP_PROTOCOL=http/protobuf \
OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:8080 \
OTEL_EXPORTER_OTLP_HEADERS="dd-api-key=$ROBINE_DOG_API_KEY" \
OTEL_SERVICE_NAME=my-api OTEL_RESOURCE_ATTRIBUTES=deployment.environment.name=dev \
opentelemetry-instrument python app.py
```

Traces, logs and metrics go to `/v1/traces`, `/v1/logs` and `/v1/metrics`.

**Errors from a Sentry SDK** — keep the SDK, change the DSN; the path names
the service, the key is `ROBINE_DOG_CLIENT_TOKEN` (or the API key):

```python
sentry_sdk.init(dsn="http://<client token>@localhost:8080/sentry/my-api/1",
                environment="dev", release="my-api@1.4.2")
```

Each error is an error log (`source` is the SDK's platform) and an issue in
Error Tracking; `environment` and `release` are `env` and `version`. With a
plain DSN (`…@host/1`) the service is the event's `service` tag, or
`sentry-<project>`. Transactions, sessions and attachments are dropped.

**Log pipelines** — raw lines (nginx, PostgreSQL…) are parsed by the
pipelines of `/logs/pipelines`, in Datadog's grok syntax; Terraform's
`datadog_logs_custom_pipeline` and the API clients work against
`/api/v1/logs/config/pipelines`.

## Configuration

All in the environment (see [`.env.example`](.env.example)): listen addresses
(`ROBINE_DOG_DOGSTATSD_LISTEN=off` disables DogStatsD),
`ROBINE_DOG_GREPTIME_URL` / `_DB` (optional `_USER` / `_PASSWORD`),
`ROBINE_DOG_POSTGRES_URL`, `ROBINE_DOG_CLIENT_TOKEN` (the key of Sentry DSNs),
`ROBINE_DOG_RETENTION_DAYS` (logs and spans,
default 15) and `ROBINE_DOG_METRICS_RETENTION_DAYS` (default 455, i.e. 15
months) applied as table TTLs, `ROBINE_DOG_API_KEY`,
`ROBINE_DOG_TRACE_REQUIRE_KEY`, the OIDC client, `ROBINE_DOG_SESSION_SECRET`,
`ROBINE_DOG_LOG_FORMAT=json` (robine-dog's own logs, one JSON object per line).

Rows are written in batches (every 200 ms or 5,000 rows) and metric buckets
every 10 seconds; on SIGTERM both are flushed before exiting.

## Layout

```text
crates/robine-dog/src
  main.rs            listeners, routes, graceful shutdown
  store/             GreptimeDB (HTTP SQL, schema, batched writers), PostgreSQL (migrations)
  query/             Datadog search syntax: parser → GreptimeDB SQL
  intake/logs.rs     /api/v2/logs (remapping, JSON messages, tags, trace ids)
  intake/traces.rs   trace-agent API (v0.4/v0.5 msgpack), service entry spans, trace metrics
  intake/series.rs   /api/v1/series, /api/v2/series
  intake/sentry.rs   Sentry SDK envelopes and events → error logs
  intake/agent.rs    Datadog Agent protobuf: traces (v0.2), series, sketches
  hosts.rs           host tags and aliases from the Agent's metadata, applied at ingestion
  pipelines/         log pipelines: processors, grok engine, user-agent parser
  monitors/          monitor queries, evaluation loop, message templates, webhooks
  metrics/           DogStatsD, 10 s aggregator, metric query language → SQL
  api/               UI API: logs, spans, facets, APM, metrics, pipelines, analytics, patterns
  auth.rs            Robine ID OIDC + session
crates/robine-dog-demo  demo traffic generator
web/src                React UI (pages/, components/, lib/)
```

`make check` runs rustfmt, clippy, the tests and the TypeScript check.
