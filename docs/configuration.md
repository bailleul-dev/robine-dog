---
title: Configuration
section: Reference
order: 10
description: Every environment variable Robine Dog reads, with its default, and how it batches writes and shuts down.
---

# Configuration

Robine Dog is configured entirely through environment variables. At startup
it also loads a `.env` file from its working directory, if there is one;
variables already set in the environment win. `.env.example` at the
repository root lists the common ones. An empty value counts as unset.

A missing required setting, or a listen address that does not parse, stops
the server at startup with a message naming the variable. A number that does
not parse falls back to the default.

## Listeners

| Variable | Default | Description |
| --- | --- | --- |
| `ROBINE_DOG_LISTEN` | `127.0.0.1:8080` | The interface, the API, sign-in and the HTTP intakes |
| `ROBINE_DOG_TRACE_LISTEN` | `127.0.0.1:8126` | The trace agent API for the ddtrace libraries. `off` disables it |
| `ROBINE_DOG_DOGSTATSD_LISTEN` | `127.0.0.1:8125` | DogStatsD over UDP. `off` disables it |

The Docker image sets all three to `0.0.0.0` on the same ports.

## Storage

| Variable | Default | Description |
| --- | --- | --- |
| `ROBINE_DOG_GREPTIME_URL` | `http://127.0.0.1:14000` | GreptimeDB's HTTP address, where logs, spans, metric points and the other telemetry are stored |
| `ROBINE_DOG_GREPTIME_DB` | `robine_dog` | The GreptimeDB database, created at startup if needed |
| `ROBINE_DOG_GREPTIME_USER`, `ROBINE_DOG_GREPTIME_PASSWORD` | Unset | GreptimeDB credentials, when it requires them |
| `ROBINE_DOG_POSTGRES_URL` | `postgres://robine_dog:robine_dog@127.0.0.1:15432/robine_dog` | PostgreSQL, for the configuration (monitors, dashboards, pipelines, keys, hosts…). Connections are made without TLS |

## Retention

| Variable | Default | Applies to |
| --- | --- | --- |
| `ROBINE_DOG_RETENTION_DAYS` | `15` | Logs, spans, RUM events and service check runs; also LLM Observability spans |
| `ROBINE_DOG_METRICS_RETENTION_DAYS` | `455` (15 months) | Metric points, cost points, events and monitor values |
| `ROBINE_DOG_PROFILE_RETENTION_DAYS` | `8` | Continuous Profiler profiles |

The first two are applied as GreptimeDB table TTLs at every startup, so
changing them takes effect on the next restart. [Log indexes](log-indexes.md)
can keep their logs for a shorter time.

## Keys

| Variable | Default | Description |
| --- | --- | --- |
| `ROBINE_DOG_API_KEY` | Unset | The API key the intakes and the public API require. When unset, any key is accepted (a warning is logged at startup), and the MCP server only answers when sign-in is disabled |
| `ROBINE_DOG_CLIENT_TOKEN` | Unset | The public key of the RUM browser SDK and of Sentry DSNs. Also turns on RUM for Robine Dog's own interface |
| `ROBINE_DOG_TRACE_REQUIRE_KEY` | Off | `true` or `1`: the trace port requires a `DD-API-KEY` header |

See [API and application keys](api-keys.md).

## Sign-in

| Variable | Default | Description |
| --- | --- | --- |
| `ROBINE_DOG_OIDC_ISSUER` | Required | The OpenID Connect issuer (Robine ID) |
| `ROBINE_DOG_OIDC_CLIENT_ID` | Required | The client id |
| `ROBINE_DOG_OIDC_CLIENT_SECRET` | Required | The client secret |
| `ROBINE_DOG_OIDC_SUBJECTS` | Empty | Comma-separated `sub` values allowed to sign in. Nobody else gets in |
| `ROBINE_DOG_SESSION_SECRET` | Required | At least 32 characters; signs the session cookie |
| `ROBINE_DOG_BASE_URL` | `http://<ROBINE_DOG_LISTEN>` | The public address of the interface; the OIDC callback is `<base>/auth/callback` |
| `ROBINE_DOG_AUTH` | Unset | `disabled` turns sign-in off: everyone is a local user, the OIDC variables and the session secret are not needed. Refused unless `ROBINE_DOG_LISTEN` is a loopback address |

The session cookie lasts 12 hours. See [Getting started](getting-started.md#sign-in).

## Notifications

| Variable | Default | Description |
| --- | --- | --- |
| `ROBINE_DOG_SMTP_URL` | Unset | The mail server for e-mail notifications: `smtp://user:password@host:587` (STARTTLS), `smtps://user:password@host:465`, or `smtp://host:25?tls=none`. Without it, no e-mail is sent |
| `ROBINE_DOG_SMTP_FROM` | `robine-dog <robine-dog@localhost>` | The sender of notification e-mails |

See [Notifications](notifications.md).

## Log archives

| Variable | Default | Description |
| --- | --- | --- |
| `ROBINE_DOG_ARCHIVE_FLUSH_SECONDS` | `300` | How often buffered logs are written to their archives. An archive with 64 MiB buffered is written early |
| `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY` | Unset | Credentials for S3 and S3-compatible archive buckets |

Archive buckets are opened with the AWS conventions of the `object_store`
library: besides the two variables above, `AWS_SESSION_TOKEN` and
`AWS_DEFAULT_REGION` are read, and an instance or pod role is used when no
key is set. The region and endpoint set on an archive's destination take
precedence. See [Log Archives](log-archives.md).

## Other features

| Variable | Default | Description |
| --- | --- | --- |
| `ROBINE_DOG_CHROMIUM` | The first of `chromium`, `chromium-browser`, `headless_shell`, `google-chrome`, `google-chrome-stable` found in `PATH` | The browser that runs [Synthetic](synthetics.md) browser tests and prints [reports](reports.md). `/usr/bin/chromium` in the Docker image |
| `ROBINE_DOG_CHROMIUM_ARGS` | Unset | Extra Chromium flags, space-separated. `--no-sandbox` in the Docker image |
| `ROBINE_DOG_OSV_URL` | `https://api.osv.dev` | The OSV API that [Code Security](code-security.md) checks libraries against, once a day. Set it empty to turn the checks off |
| `ROBINE_DOG_REPLAY_SAMPLE_RATE` | `0` | The percentage (0 to 100) of Robine Dog's own interface sessions recorded by [Session Replay](session-replay.md). Needs `ROBINE_DOG_CLIENT_TOKEN` |
| `DD_ENV` | Unset | The `env` of Robine Dog's own interface sessions in RUM |
| `ROBINE_DOG_STATIC_DIR` | `web/dist` | The built interface. `/app/web` in the Docker image |
| `KUBERNETES_SERVICE_HOST`, `KUBERNETES_SERVICE_PORT` | Set by Kubernetes | The API server that [Workflow](workflows.md) Kubernetes steps call, with the pod's service account |

## Robine Dog's own logs

| Variable | Default | Description |
| --- | --- | --- |
| `ROBINE_DOG_LOG_FORMAT` | Text | `json` writes one JSON object per line, with `level` and `message` at the top: what a log collector, Robine Dog's own intake included, parses without a pipeline |
| `RUST_LOG` | `robine_dog=info,tower_http=warn` | Log levels, as a `tracing` filter: `RUST_LOG=robine_dog=debug` |

The version is set when the image is built (`ROBINE_DOG_VERSION`, from the
release tag or the commit) and reported by `GET /health`:

```sh
curl http://localhost:8080/health
```

```json
{"status": "ok", "version": "v0.2.0"}
```

## Writes, batching and shutdown

The intakes answer as soon as the data is handed over; writing happens
behind them:

| What | When it is written |
| --- | --- |
| Logs, spans, metric points, RUM events, check runs | In batches per table: after 200 ms, or as soon as 5,000 rows are waiting |
| DogStatsD samples and trace metrics | Aggregated in memory into 10-second buckets, written every 10 seconds |
| Log archives | Every `ROBINE_DOG_ARCHIVE_FLUSH_SECONDS`, or at 64 MiB buffered |
| API key last use | Every minute |

When GreptimeDB falls behind, the batch queues fill and the intakes wait
instead of piling data up in memory. A failed insert is retried 6 times,
with a delay starting at 500 ms and doubling; after that the batch is dropped
and an error is logged.

On `SIGTERM` or Ctrl-C, the server stops accepting connections, then
writes the metric buckets, the archives and every pending batch before it
exits. Nothing that was acknowledged is lost on a normal restart.

> **Note:** a few things are kept only in memory and start over after a
> restart: live processes, the last values of OpenTelemetry cumulative
> counters, and Database Monitoring's active sessions.
