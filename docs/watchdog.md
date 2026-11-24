---
title: Watchdog
section: Monitors and Alerting
order: 40
app: /watchdog
description: Watchdog finds anomalies in each service's error logs, failed requests and latency without any configuration, explains them with root cause and impact analysis, and points out what stands out in the errors of your searches.
---

# Watchdog

Watchdog watches every service without a monitor to set up. Every five
minutes it compares the last quarter of an hour of each service with that
service's previous day. When something is far from usual, it opens a
**story** on the [Watchdog](/watchdog) page and posts an
[event](events.md). From a story you can look for the root cause among the
service's dependencies, see which users were affected, and declare an
[incident](incidents.md).

Watchdog also has **Insights**: in the Log, Trace and RUM explorers, it
shows the facet values that are over-represented in the errors of your
search.

## What Watchdog detects

| Kind | What is measured, per service and per 15 minutes | Minimum to alert |
| --- | --- | --- |
| Error logs | Logs with status `error`, `critical`, `alert` or `emergency` (synthetic test logs excluded) | 20 error logs |
| APM errors | Failed service entry spans (requests) | 10 failed requests |
| APM latency | The 95th percentile of service entry span durations, in buckets with at least 20 requests | 50 ms |

The current quarter of an hour is an anomaly when its value is above all
of these:

- the mean of the previous day's 96 quarters plus four standard deviations;
- three times that mean;
- the minimum of the table above, so that quiet services with a handful of
  errors do not alert.

A quarter with no error log or failed request counts as zero. For latency,
only quarters with at least 20 requests count, and at least 8 of them are
needed in the previous day. Logs and spans without a `service` are not
watched.

## Stories

An anomaly opens a story for its kind and service. The story stays
**ongoing** while each run finds the anomaly again, and keeps the worst
value seen. It is resolved once a run has not found it for a quarter of an
hour. Stories are kept 30 days.

Each new story posts an event with `source:watchdog`, status `warn`, the
service, and the tag `watchdog_kind:<error_logs|apm_errors|apm_latency>`.
Its text gives the value against the usual one:

```text
Error logs of shop-api are well above usual
212 error logs per 15 min against 3.4 usually.
```

To be notified, create an [event monitor](monitors.md) on these events:

```text
events("source:watchdog service:shop-api").rollup("count").last("15m") > 0
```

The [Watchdog](/watchdog) page lists the **Ongoing** stories, then those of
the **Past 30 days**. Each card shows when the anomaly started and ended,
its peak and the usual value. An error logs story shows a graph of the
service's error logs around it. The buttons depend on the story:

| Button | What it does |
| --- | --- |
| **Root cause** | APM stories: the [root cause analysis](#root-cause-analysis) |
| **Impact** | APM stories: the [impact analysis](#impact-analysis) |
| **Declare Incident** | Ongoing stories: opens the **Declare Incident** dialog of [Incidents](/incidents), with the story's title |
| **View error logs** | Error logs stories: the Log Explorer on `service:<name> status:error` around the story |
| **Service page**, **View traces** | APM stories: the [service page](services.md), and the Trace Explorer on the service (its errors for an APM errors story) |

**Run detection now** runs the detection at once instead of waiting for the
next five-minute run.

## Root cause analysis

A service is often slow or failing because something it calls is. Root
cause analysis reads the [service map](service-map.md) over the story's
time range (from a quarter of an hour before it started to its end) and
walks the services downstream of the story's service. Among them, it keeps
those that had a story of their own at the same time.

The **likely root cause** is the deepest of those anomalous dependencies,
and among equally deep ones, the one whose anomaly started first. When no
dependency had an anomaly, the cause is likely in the service itself. The
analysis also lists the services that call the story's service, marking
those that had an anomaly too: they are impacted.

## Impact analysis

Impact analysis finds the people behind an anomaly. It takes the traces of
the story's service during the anomaly (up to 2,000): those with an error,
or for a latency story, those with a span at least twice the usual p95.
Then it looks for [RUM](rum.md) events carrying those trace ids, and shows:

- how many RUM sessions and identified users (`usr.name`, `usr.email` or
  `usr.id`) made those requests;
- the views (`view.url_path`) they came from, with their sessions and
  requests, the 20 most affected first.

> **Note:** RUM links a request to its trace only when the browser SDK's
> `allowedTracingUrls` covers the API it calls. Without it, the analysis
> finds the traces but no session.

## Watchdog Insights

Above the results of the [Log Explorer](log-explorer.md), the
[Trace Explorer](trace-explorer.md) and the [RUM Explorer](rum.md), a
**Watchdog Insights** bar shows the facet values that stand out in the
errors of the current search and time range:

```text
version:3.2.0  63% of errors  vs 4%
```

Clicking one adds the filter to your search; **×** hides the bar. The
[Insights](/watchdog/insights) page shows the same for the last hour, over
all logs, spans and RUM events.

The errors are `status:error` for logs and spans and `@type:error` for RUM.
A value stands out when it has at least 5 errors, makes up at least 25% of
the errors and at least 1.5 times its share of all events. Up to two values
per facet and six in all are shown, the largest gap first. The facets
compared are:

| Explorer | Facets |
| --- | --- |
| Logs | `service`, `host`, `env`, `source`, `version`, `@http.status_code` |
| Traces | `service`, `resource_name`, `env`, `version`, `host`, `@http.status_code` |
| RUM | `service`, `version`, `@view.name`, `@browser.name`, `@os.name`, `@geo.country` |

## Differences from Datadog

- Watchdog watches three things per service (error logs, failed requests,
  p95 latency) against the previous day. It does not look at
  infrastructure metrics, log patterns or seasonality beyond a day.
- Stories have no API outside the UI's own, and no `@`-handle notification
  of their own: alert on their `source:watchdog` events with an event
  monitor.
- Insights compare errors with everything; they do not find latency
  outliers.
