---
title: Continuous Profiler
section: APM
order: 60
app: /profiling
description: The profiles Datadog's profilers upload every minute, merged into flame graphs per service, version and endpoint, to find where code spends CPU, time and memory.
---

# Continuous Profiler

Traces tell you which request was slow; profiles tell you which lines of code
spent the time. Datadog's profilers, built into the ddtrace libraries, sample
the running process continuously and upload a profile every minute. Robine
Dog stores those uploads, merges the profiles of a period into one flame
graph per sample type, and links them to the services and spans of APM.

## Send profiles

Turn on the profiler in the tracer, with the usual
[unified service tags](tagging.md):

```sh
DD_PROFILING_ENABLED=true \
DD_SERVICE=shop-api DD_ENV=prod DD_VERSION=1.4.2 \
DD_AGENT_HOST=localhost DD_TRACE_AGENT_PORT=8126 \
ddtrace-run python app.py
```

Profiles reach Robine Dog by one of two paths:

| Path | Endpoint | Used when |
| --- | --- | --- |
| Straight from the tracer | `POST /profiling/v1/input` on the trace port (`localhost:8126`) | The tracer's agent address points at Robine Dog, as in the example |
| Through the Datadog Agent | `POST /api/v2/profile` on port 8080, with `DD-API-KEY` | The tracer talks to a Datadog Agent, whose profiling proxy forwards to Robine Dog |

When a Datadog Agent sits in between, point its profiling proxy at Robine
Dog (`apm_config.profiling_dd_url`, or `DD_APM_PROFILING_DD_URL`):

```yaml
# datadog.yaml
apm_config:
  apm_dd_url: http://robine-dog:8080
  profiling_dd_url: http://robine-dog:8080/api/v2/profile
```

### Accepted uploads

| Aspect | Supported |
| --- | --- |
| Upload format | Multipart: the v4 format (an `event` JSON part with `attachments`, `start`, `end`, `tags_profiler`) and the older one (`recording-start`, `recording-end`, `tags[]`, `data[0]`…) |
| Profile files | pprof (`profile.proto`), raw, gzip or zstd compressed; lz4 is refused |
| Tags | From `tags_profiler` or `tags[]`, plus the `X-Datadog-Additional-Tags` header |
| Body size | 64 MiB |

The profile's `service`, `env`, `version`, `host`, `language` (or
`runtime`) and `runtime-id` are read from its tags. A profile without a
`service` tag is filed under `unnamed-service`. An upload with no readable
pprof file is refused with a 400 and the reason.

## Retention

Profiles are stored in PostgreSQL, with their original files. They are kept
`ROBINE_DOG_PROFILE_RETENTION_DAYS` days (default `8`, as in Datadog); an
hourly job deletes older ones.

```sh
ROBINE_DOG_PROFILE_RETENTION_DAYS=14
```

## The Profiles page

[Profiles](/profiling) opens on the last hour and refreshes every minute. Pick:

| Control | Effect |
| --- | --- |
| Service | One of the services with profiles in the range |
| Version | All versions, or one |
| Type | The sample type to show (see below); CPU time by default |
| Endpoint | All, or only the samples taken while serving one endpoint |
| Highlight a function | Frames whose name contains the text stand out, others fade |

The header shows how many profiles were merged and the total value. Up to
300 profiles of the range (the most recent) are merged.

### Flame graph

The root (`all`) is on top and callees are below; each frame's width is its
share of the total. Frames are colored by package. Hover a frame for its
total and self values; click it to zoom in, and use the breadcrumb above the
graph to zoom back out.

### Top functions and endpoints

- **Top functions (self)**: the 50 functions with the most self value (time
  or memory spent in the function itself), with their total value (including
  their callees).
- **Endpoints**: the value per endpoint, from the `trace endpoint` label the
  profilers put on samples taken during a traced request. Click an endpoint
  to show only its samples.

### Profiles list

The latest 50 profiles of the selection: when they were received, host,
version, env, sample types, size, and their pprof files. Click a file name to
download it and open it with `go tool pprof` or another pprof viewer.

## Sample types

The types depend on the language and the profiler's settings:

| Type | Shown as |
| --- | --- |
| `cpu-time`, `cpu` | CPU time |
| `wall-time`, `wall` | Wall time |
| `alloc-space` | Allocated memory |
| `alloc-samples` | Allocations |
| `inuse-space` | Heap live size |
| `inuse-objects` | Heap live objects |
| `lock-acquire-wait` | Lock wait time |
| `lock-release-hold` | Lock hold time |
| `exception-samples` | Thrown exceptions |

Other types are listed under their own name. When profiles are merged,
amounts over a period (CPU, allocations…) are summed, and snapshots (heap in
use, live objects) are averaged over the profiles.

## From traces and services

- On a [service page](services.md), **View Profiles** opens the profiler on
  that service.
- In the [trace view](trace-explorer.md#the-trace-view), a span's
  **Profiles** button opens the service's profiles around the span's time (a
  minute before and after); for a root span, it also selects its endpoint.

## Differences from Datadog

- Profiles are merged by service, version and endpoint; there is no search
  by arbitrary tag, no comparison view and no timeline view per thread.
- Each upload keeps the 5,000 largest stacks per sample type; the rest are
  summed into an `[other]` frame.
- There are no profiling metrics (`profiling.*`) and no code hotspots per
  span beyond the endpoint filter.
