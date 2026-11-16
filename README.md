# robine-dog

A personal, self-hosted log and trace explorer to practise Datadog's Logs and
APM workflows on real data: the same search syntax, facets, explorers, trace
views and service pages, fed by the **real Datadog clients** (ddtrace
libraries, the logs HTTP intake). Sign-in is through Robine ID.

Rust (axum, SQLite in WAL mode) + React (Vite, TanStack Query). One binary
serves the UI, the API and both intakes.

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

Not there yet: metrics, monitors, dashboards, log pipelines/patterns, the
Datadog Agent's own trace forwarding protocol (`/api/v0.2/traces`, protobuf),
OTLP.

## Run it

Requirements: Rust 1.85+, Node 22+.

```sh
cp .env.example .env    # then fill in the Robine ID client (see below)
make dev                # server :8080 + :8126, UI http://localhost:5173
make demo               # in another terminal: an hour of history, then live traffic
```

Without Robine ID, on loopback only: `make dev-noauth`.

`make demo` simulates a small shop (web-store → shop-api → cart-service /
payment-service → Postgres, Redis, Stripe) with ~8% declined payments and ~3%
provider timeouts, using the same wire formats as ddtrace and the Agent.

## Sign-in with Robine ID

robine-dog is a confidential OIDC client: authorization code with PKCE S256
and a nonce, `client_secret_post`, ID token RS256-verified against the JWKS,
then a 12-hour HMAC-signed session cookie. Only the `sub` values listed in
`ROBINE_DOG_OIDC_SUBJECTS` are let in.

Declare the clients in silo01's `robine_id_applications` (ready-made entries in
[`deploy/robine-id-clients.yml`](deploy/robine-id-clients.yml)), run
`make robine-id` there, and put the secret in `.env`. The callback is
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

## Configuration

All in the environment (see [`.env.example`](.env.example)): listen addresses,
`ROBINE_DOG_DB`, `ROBINE_DOG_RETENTION_DAYS` (default 15, purged every 10
minutes), `ROBINE_DOG_API_KEY`, `ROBINE_DOG_TRACE_REQUIRE_KEY`, the OIDC
client, `ROBINE_DOG_SESSION_SECRET`.

## Layout

```text
crates/robine-dog/src
  main.rs            listeners, routes, retention
  query/             Datadog search syntax: parser → SQL
  intake/logs.rs     /api/v2/logs (remapping, JSON messages, tags, trace ids)
  intake/traces.rs   trace-agent API (v0.4/v0.5 msgpack), service entry spans
  api/               UI API: logs, spans, facets, APM
  auth.rs            Robine ID OIDC + session
crates/robine-dog-demo  demo traffic generator
web/src                React UI (pages/, components/, lib/)
```

`make check` runs rustfmt, clippy, the tests and the TypeScript check.
