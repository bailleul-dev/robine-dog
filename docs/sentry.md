---
title: Sentry SDKs
section: Send Data
order: 60
app: /error-tracking
description: Keep your application's Sentry SDK and change its DSN, so its errors become error logs and Error Tracking issues in Robine Dog.
---

# Sentry SDKs

Datadog's Error Tracking accepts events from Sentry SDKs, and so does Robine
Dog. You keep the SDK your application already uses and change its DSN.
Each error event becomes an error log with Datadog's standard error
attributes (`error.kind`, `error.message`, `error.stack`), which
[Error Tracking](error-tracking.md) groups into issues. The logs are
searchable in the [Log Explorer](log-explorer.md) like any other.

## Set the DSN

A DSN carries a key and a project; Robine Dog adds an optional service name
in the path:

```text
http://<client token>@localhost:8080/sentry/<service>/<project>
```

```python
import sentry_sdk

sentry_sdk.init(
    dsn="http://pub0123456789abcdef@localhost:8080/sentry/my-api/1",
    environment="prod",
    release="my-api@1.4.2",
)
```

| DSN form | Endpoints the SDK posts to | Service of the events |
| --- | --- | --- |
| `http://<key>@localhost:8080/sentry/my-api/1` | `/sentry/my-api/api/1/envelope/`, `/sentry/my-api/api/1/store/` | `my-api` |
| `http://<key>@localhost:8080/1` | `/api/1/envelope/`, `/api/1/store/` | The event's `service` tag, else `sentry-1` |

Use the first form: it names the service whatever the code tags. Current SDKs
post envelopes to `/envelope/`; older ones post a single event to `/store/`.
Both may be gzip or deflate compressed.

### The key

The DSN's public key must be the client token (`ROBINE_DOG_CLIENT_TOKEN`) or
the configured API key (`ROBINE_DOG_API_KEY`). The client token is meant to
be public: use it in browser and mobile applications. Named API keys created
in the interface are not accepted here. When neither variable is set, any key
is accepted. See [API and application keys](api-keys.md).

Robine Dog reads the key from the `sentry_key` query parameter, the
`X-Sentry-Auth` (or `Authorization`) header, or the `dsn` in the envelope
header, wherever the SDK puts it. A wrong key gets
`403 {"detail": "invalid sentry_key"}`.

Browser SDKs post from another origin: the endpoints answer `OPTIONS`
preflight requests and allow any origin.

## What becomes what

| Sentry event | Robine Dog log |
| --- | --- |
| The last exception of `exception.values` (the one thrown) | `error.kind` (its type), `error.message` (its value); the message is `<type>: <value>` |
| The exception chain and their stack frames | `error.stack`: the thrown exception first, its frames with the crashing one first, then each cause after `Caused by:` |
| `exception.mechanism.handled` | `error.handled` |
| `level` | `status` (default `error`) |
| `message` or `logentry` (`capture_message`) | The message |
| `timestamp` | The log's date |
| `environment` | `env` tag |
| `release` | `version` tag |
| `server_name` | `host` |
| `platform` | `source` (`sentry` when the platform is `other` or absent) |
| `tags` | Tags, except `service`, `env` and `version` |
| `user.id`, `user.email`, `user.username` | `usr.id`, `usr.email`, `usr.name` |
| `user.ip_address` | `network.client.ip` |
| `request.method`, `request.url`, the `User-Agent` header | `http.method`, `http.url`, `http.useragent` |
| `contexts.trace.trace_id`, `contexts.trace.span_id` | The log's trace and span ids, which link it to an APM trace |
| `contexts`, `extra` | Kept as they are |
| `event_id`, `platform`, `transaction`, `logger`, `sdk.name`, `sdk.version`, `dist` | `sentry.event_id`, `sentry.platform`, `sentry.transaction`, `sentry.logger`, `sentry.sdk_name`, `sentry.sdk_version`, `sentry.dist`; plus `sentry.project` |
| `breadcrumbs` | `sentry.breadcrumbs`, the last 50 |

A message captured at level `error` or above (`capture_message("…",
level="error")`) also gets `error.kind: Message`, and its stack trace when the
SDK attached one, so it becomes an issue of its own, as in Sentry. Messages
at a lower level are plain logs.

The resulting logs then go through the usual processing: [host
tags](tagging.md#host-tags-and-aliases), [pipelines](log-pipelines.md),
[Sensitive Data Scanner](sensitive-data-scanner.md), indexes and archives.

Find them with:

```text
source:python status:error @sentry.project:1
```

## Dropped items

Envelopes can carry many kinds of items. Only `event` items are read;
everything else is accepted and dropped:

| Dropped | Use instead |
| --- | --- |
| Transactions and spans (performance monitoring) | The Datadog tracers or [OpenTelemetry](opentelemetry.md), for [APM](tracing.md) |
| Sessions and session aggregates (release health) | |
| Attachments, minidumps | |
| Client reports, profiles, check-ins, replays | |

The endpoint answers `200` with `{"id": "<event id>"}` either way, so the
SDK does not retry or report an error.

## Differences from Datadog

- The service can be set in the DSN path (`/sentry/<service>/…`), which
  Datadog's DSNs do not have.
- Source maps and debug symbols are not applied: stack traces are shown as
  the SDK sent them.
- The project number is kept in `sentry.project` but does not select
  anything.
