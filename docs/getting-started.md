---
title: Getting started
section: Getting Started
order: 10
description: Run Robine Dog on your machine, load the demo shop, and follow one request from its logs to its trace, its service, its metrics and a monitor.
---

# Getting started

This page gets a development instance of Robine Dog running, fills it with
an hour of realistic traffic, and walks you through the main explorers the
way you would discover them in Datadog. When you are done, read
[Unified service tagging](tagging.md) and the [Send data](README.md#send-data)
pages to connect your own applications.

## Requirements

| Tool | Version | Used for |
| --- | --- | --- |
| Rust | 1.85 or later | The server and the demo generator |
| Node.js | 22 or later | The web interface (Vite) |
| Docker | Any recent version | GreptimeDB and PostgreSQL in development |

## Run Robine Dog

1. Copy the example configuration. `make dev` reads `.env` from the
   repository root.

   ```sh
   cp .env.example .env
   ```

2. Start everything:

   ```sh
   make dev
   ```

   This starts GreptimeDB (`127.0.0.1:14000`) and PostgreSQL
   (`127.0.0.1:15432`) with Docker Compose, then the server and the Vite
   development server. Open the interface at `http://localhost:5173`; it
   proxies `/api` and `/auth` to the server.

The server listens on three ports:

| Port | Protocol | What it receives |
| --- | --- | --- |
| 8080 | HTTP | The API, the logs, series, Agent, OTLP, RUM and Sentry intakes |
| 8126 | HTTP | The trace agent API that the ddtrace libraries report to |
| 8125 | UDP | DogStatsD |

`make deps` starts the two databases alone and `make deps-down` stops them,
keeping their volumes.

### Without sign-in

`make dev` signs you in with Robine ID (see [Sign-in](#sign-in)). To try
Robine Dog without an identity provider, run:

```sh
make dev-noauth
```

This sets `ROBINE_DOG_AUTH=disabled`: every visitor is a "Local user". The
server refuses to start in this mode unless `ROBINE_DOG_LISTEN` is a
loopback address, so it cannot be exposed by mistake.

> **Warning:** keep `ROBINE_DOG_API_KEY` set to a random value in `.env`.
> When it is unset, every intake accepts any key (the server logs a warning
> at startup). See [API and application keys](api-keys.md).

## Load the demo shop

In a second terminal, start the demo traffic generator:

```sh
make demo
```

It backfills the last 60 minutes, then sends about 3 requests per second
until you stop it (`make demo` runs `robine-dog-demo --backfill-minutes 60
--rate 3`; add `--no-live` to stop after the backfill). It uses the same wire
formats as the real clients: ddtrace's v0.4 msgpack traces on port 8126,
JSON logs on `/api/v2/logs`, DogStatsD over UDP, the browser SDK's RUM
events, profiler uploads and LLM Observability spans.

The demo simulates a small e-commerce system:

| Service | Language | Role |
| --- | --- | --- |
| `web-store` | Python | The storefront, rendering pages |
| `shop-api` | Go | The API behind the storefront |
| `cart-service` | Node.js | The cart, backed by Redis |
| `payment-service` | Java | Payments, calling Stripe |
| `order-worker` | Go | Background jobs |

Its traffic has realistic faults: about 8% of payments are declined by the
card issuer and about 3% time out at the payment provider, each with
correlated error logs. It also sends host metrics (`system.cpu.user`,
`system.mem.used`, `system.load.1`), business metrics (`shop.orders.placed`,
`shop.cart.value`, `shop.queue.depth`), service checks (`datadog.agent.up`,
and an `http.can_connect` check that `payment-service` fails a few minutes
every hour), 14 days of browser sessions and production (`env:prod`) and
staging (`env:staging`) traffic.

The demo reads its targets from the environment: `ROBINE_DOG_URL`
(default `http://127.0.0.1:8080`), `DD_TRACE_AGENT_URL`
(default `http://127.0.0.1:8126`), `DD_DOGSTATSD_URL`
(default `udp://127.0.0.1:8125`) and `DD_API_KEY` or `ROBINE_DOG_API_KEY`.

## A first tour

### 1. Find an error in the logs

Open [Logs](/logs). The histogram at the top shows the volume of logs over
the time range; drag across it to zoom in. Type this in the search bar:

```text
service:payment-service status:error
```

The facets on the left (**Source**, **Host**, **Service**, **Status**)
count the values of the current results: click a host to narrow down, click
it again to clear. Click a log to open its side panel with its attributes. The search language is described
in [Search syntax](search-syntax.md).

### 2. Jump to its trace

The demo's logs carry `dd.trace_id` and `dd.span_id`, as ddtrace's log
injection adds them. In the log's side panel, click **View Trace**. The trace
view shows the request as a **Flame Graph**, a **Waterfall** or a **Span
List**; the failing span is marked as an error. Select a span: its **Info**
tab lists its tags and metrics, **Logs** lists the logs of the trace,
and **Errors** shows the stack trace. See [Trace Explorer](trace-explorer.md).

### 3. Open the service page

Open [APM](/apm/services) and click `payment-service`. The service page
graphs its requests, errors and latency percentiles, lists its resources
(endpoints) and the services it calls, and tracks each `version` it has
served. See [Service pages](services.md).

### 4. Graph a metric

Open the [Metrics Explorer](/metrics/explorer) and enter:

```text
avg:system.cpu.user{env:prod} by {host}
```

Each host is a line. Every service also has trace metrics computed from its
spans; try `sum:trace.servlet.request.errors{service:payment-service}.as_count()`.
[Metrics Summary](/metrics/summary) lists every metric with its type, tags
and hosts. See [Metrics](metrics.md).

### 5. Create a monitor

Open [New Monitor](/monitors/create), choose **Metric**, and enter:

```text
avg(last_5m):avg:system.cpu.user{env:prod} by {host} > 80
```

The editor evaluates the query as you type. Monitors are evaluated every
minute, per group (here, per host). Add `@webhook-<name>` or an e-mail
address to the message to be notified. See [Monitors](monitors.md) and
[Notifications](notifications.md).

## Sign-in

Robine Dog signs users in with Robine ID, an OpenID Connect provider. It is
a confidential client (authorization code flow with PKCE S256 and a nonce,
`client_secret_post`), checks the RS256 ID token against the issuer's JWKS,
then sets a session cookie signed with HMAC that lasts 12 hours.

Only the subjects (`sub` claims) listed in `ROBINE_DOG_OIDC_SUBJECTS` are let
in; anyone else is refused, even with a valid Robine ID account. Set these in
`.env`:

| Variable | Value |
| --- | --- |
| `ROBINE_DOG_OIDC_ISSUER` | The issuer URL |
| `ROBINE_DOG_OIDC_CLIENT_ID`, `ROBINE_DOG_OIDC_CLIENT_SECRET` | The client registered for Robine Dog |
| `ROBINE_DOG_OIDC_SUBJECTS` | Comma-separated `sub` values allowed in |
| `ROBINE_DOG_SESSION_SECRET` | At least 32 characters; signs the session cookie |
| `ROBINE_DOG_BASE_URL` | The public address of the interface; the callback is `<base>/auth/callback` |

In development, `ROBINE_DOG_BASE_URL` is `http://localhost:5173`, so the
callback registered with the provider is `http://localhost:5173/auth/callback`.

The session protects the interface and its internal API. The intakes and
Datadog's public API use keys instead; see [API and application keys](api-keys.md).

## Where to go next

- [Unified service tagging](tagging.md): set `service`, `env` and `version` so logs, traces and metrics line up.
- [Datadog Agent](agent.md): connect hosts, containers or a Kubernetes cluster.
- [APM tracers](tracing.md): trace your own application.
- [Log collection](log-collection.md) and [DogStatsD](dogstatsd.md): send logs and metrics directly.
- [Configuration](configuration.md): every setting, for a production deployment.
