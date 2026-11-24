---
title: Robine Dog documentation
section: Getting Started
order: 0
description: A self-hosted observability platform that speaks Datadog's protocols. Send it data with the clients you already use, then search, graph, alert and investigate the way you would in Datadog.
---

# Robine Dog documentation

Robine Dog receives logs, traces, metrics, browser sessions, profiles, test
results and security events from the **real Datadog clients**: the ddtrace
libraries, DogStatsD, the Datadog Agent, the browser SDK, `datadog-ci`. It
also accepts OpenTelemetry and Sentry SDKs. It shows that data with
Datadog's explorers, query languages and APIs, so what you learn here works
in Datadog, and the other way round.

> **Note:** these pages describe what Robine Dog implements. Where it
> behaves differently from Datadog, the page says so.

## Get started

- [Getting started](getting-started.md): run Robine Dog, load the demo shop and take a first look around.
- [Unified service tagging](tagging.md): `service`, `env`, `version` and `host` tie every kind of data together.
- [API and application keys](api-keys.md): the keys the intakes, the API and the SDKs need.

## Send data

| You have | Read |
| --- | --- |
| Hosts, containers or a Kubernetes cluster | [Datadog Agent](agent.md) |
| An application to trace | [APM tracers](tracing.md) |
| Logs to ship | [Log collection](log-collection.md) |
| Counters, gauges and histograms | [DogStatsD and the series API](dogstatsd.md) |
| OpenTelemetry SDKs or a Collector | [OpenTelemetry](opentelemetry.md) |
| An application reporting to Sentry | [Sentry SDKs](sentry.md) |
| A website | [Real User Monitoring](rum.md) |

## Explore

- [Search syntax](search-syntax.md): one query language for logs, spans, RUM events, events and monitors.
- [Log Explorer](log-explorer.md), [Trace Explorer](trace-explorer.md), [Metrics Explorer](metrics.md).
- [Service pages](services.md) and the [service map](service-map.md) for APM.
- [Dashboards](dashboards.md) and [notebooks](notebooks.md).

## Alert and respond

- [Monitors](monitors.md) evaluate a query every minute and notify through [webhooks, e-mail and on-call](notifications.md).
- [SLOs](slos.md) track error budgets; [incidents](incidents.md), [on-call](on-call.md) and [cases](case-management.md) organise the response.
- [Watchdog](watchdog.md) finds anomalies without any configuration.

## How Robine Dog is built

One Rust binary serves the interface, the API and every intake:

| Port | Protocol | What listens |
| --- | --- | --- |
| 8080 | HTTP | The UI, the API, the logs, series, Agent, OTLP, RUM and Sentry intakes |
| 8126 | HTTP | The trace agent API, as the Datadog Agent exposes it |
| 8125 | UDP | DogStatsD |

Telemetry (logs, spans, metric points) is stored in GreptimeDB, with a
retention of 15 days for logs and spans and 15 months for metrics.
Configuration (monitors, dashboards, pipelines, facets…) is stored in
PostgreSQL. See [Configuration](configuration.md) for every setting.
