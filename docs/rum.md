---
title: Real User Monitoring
section: Digital Experience
order: 10
app: /rum/performance-monitoring
description: What real visitors experience on your website, from Datadog's browser SDK pointed at Robine Dog - page performance and web vitals, sessions, errors, frustration signals, funnels and retention.
---

# Real User Monitoring

Real User Monitoring (RUM) records what happens in your visitors' browsers:
the pages they view and how fast those pages load, what they click, the
requests the page makes, the JavaScript errors they hit and the long tasks
that freeze the page. You install Datadog's browser SDK on your site,
unchanged, and point it at Robine Dog with its `proxy` option.

## Set up the browser SDK

Install the SDK as Datadog documents it and add `proxy`: the address of
your Robine Dog followed by `/rum/proxy`.

### npm

```sh
npm install @datadog/browser-rum
```

```js
import { datadogRum } from "@datadog/browser-rum";

datadogRum.init({
  applicationId: "shop-web",
  clientToken: "<client token>",
  proxy: "http://localhost:8080/rum/proxy",
  service: "shop-web",
  env: "prod",
  version: "1.4.2",
  sessionSampleRate: 100,
  sessionReplaySampleRate: 20,
  trackUserInteractions: true,
  trackResources: true,
  trackLongTasks: true,
  defaultPrivacyLevel: "mask-user-input",
});
```

### CDN

```html
<script src="https://www.datadoghq-browser-agent.com/us1/v6/datadog-rum.js" type="text/javascript"></script>
<script>
  window.DD_RUM &&
    window.DD_RUM.init({
      applicationId: "shop-web",
      clientToken: "<client token>",
      proxy: "http://localhost:8080/rum/proxy",
      service: "shop-web",
      env: "prod",
      sessionSampleRate: 100,
      trackUserInteractions: true,
    });
</script>
```

The SDK then sends everything to `/rum/proxy?ddforward=<Datadog path>`.
CORS preflight requests are answered, so your site can be on another origin.

### The client token

`clientToken` must be the value of `ROBINE_DOG_CLIENT_TOKEN` or of
`ROBINE_DOG_API_KEY`; when neither is set, any key is accepted. The token is
visible in your site's code, so use `ROBINE_DOG_CLIENT_TOKEN`, not the API
key. See [API and application keys](api-keys.md).

### Browser logs and traces

- **Browser logs**: `@datadog/browser-logs` with the same `clientToken` and
  `proxy` sends its logs through the same proxy; they become logs with
  `source:browser` in the [Log Explorer](log-explorer.md).
- **Traces**: with the SDK's `allowedTracingUrls`, the RUM resource keeps
  the backend trace's id (`_dd.trace_id`) as its `trace_id`.
- **Direct intake**: without the proxy, post newline-delimited JSON events
  to `POST /api/v2/rum?ddsource=browser&dd-api-key=<client token>`.

## What is collected

Each SDK event is stored as a RUM event whose `source` is its type:

| Type | Message | Notable attributes |
| --- | --- | --- |
| `view` | The view name or URL path | `@view.url_path`, `@view.url_host`, `@view.loading_time`, `@view.largest_contentful_paint`, `@view.interaction_to_next_paint`, `@view.cumulative_layout_shift`, `@view.error.count` |
| `action` | Action type and target, such as `click Pay` | `@action.type`, `@action.target.name`, `@action.frustration.type` |
| `resource` | Method and URL | `@resource.url`, `@resource.status_code`, `@resource.duration` |
| `error` | The error message | `@error.type`, `@error.message`, `@error.stack`, `@error.source` |
| `long_task` | `long task of N ms` | `@long_task.duration` |

All events carry `@session.id`, `@view.id`, `@usr.*` (when you call
`setUser`), `service`, `env` and `version`, plus the SDK's `ddtags`. The SDK's
own telemetry is dropped.

Robine Dog adds `@view.url_path` and `@view.url_host` from the view's URL,
and parses the request's `User-Agent` into `@browser.name`,
`@browser.version_major`, `@os.name`, `@os.version_major` and
`@device.type` (`desktop`, `mobile`, `tablet`, `bot`, `other`) when the SDK
did not set them. A view is sent again each time it changes; pages that
count views keep its latest values.

Errors are also stored as error logs (`source:browser`, with `error.kind`,
`error.message` and `error.stack`): they go through your
[log pipelines](log-pipelines.md) and are grouped into issues by
[Error Tracking](error-tracking.md). RUM events are kept for
`ROBINE_DOG_RETENTION_DAYS` days (15 by default), like logs and spans.

## Performance

[Performance](/rum/performance-monitoring) opens on the last day. Tiles count
the sessions, page views, actions and errors; the table lists each page
(view name) with its service, views, errors and its web vitals at the 75th
percentile, colored against Google's thresholds:

| Vital | Good up to | Poor from |
| --- | --- | --- |
| Loading time | 2.5 s | 4 s |
| LCP (largest contentful paint) | 2.5 s | 4 s |
| FCP (first contentful paint) | 1.8 s | 3 s |
| INP (interaction to next paint) | 200 ms | 500 ms |
| CLS (cumulative layout shift) | 0.1 | 0.25 |

## Sessions

[Sessions](/rum/sessions) lists the 200 latest sessions: start, service,
entry page, user, duration, views, actions, errors and frustration signals.
Click the entry page for the session's events, or **Replay** to watch it in
[Session Replay](session-replay.md).

## RUM Explorer and analytics

The [RUM Explorer](/rum/explorer) searches RUM events with the
[search syntax](search-syntax.md), over the last hour by default. Buttons
filter by type (`source:view`, `source:action`, `source:error`,
`source:resource`, `source:long_task`). Click an event for its tags and
attributes, and **Show the whole session** to list every event of its
session. Watchdog Insights point out what is over-represented in errors.

```text
source:error @view.url_path:/checkout @browser.name:Safari
```

[Analytics](/rum/analytics) graphs sessions and page views over time, and
the top pages, sites, browsers, operating systems, devices and referrers.

## Frustration signals

The SDK flags clicks that went wrong in `@action.frustration.type`: **rage
clicks** (repeated clicks on one element), **dead clicks** (clicks that do
nothing) and **error clicks** (clicks followed by an error).
[Frustrations](/rum/frustrations) shows the share of sessions with a
signal, the count of each type, the pages and elements with the most
signals, and the latest frustrated clicks.

## Funnels and retention

- [Funnels](/rum/funnels) counts the sessions that went through up to 10
  steps in order. A step is a view path (wildcards allowed, such as
  `/product/*`) or any `@facet:value`, such as `@action.target.name:Pay`;
  an optional filter (`@geo.country:France`) restricts the sessions. The result shows each
  step's sessions, the conversion from the first step and the biggest drop.
- [Retention](/rum/retention) groups users by the day or week of their first
  visit and shows the share that came back each day or week after. Users are
  identified by `@usr.id` by default, or by another attribute; the start and
  return events can each be narrowed by a query.

## RUM-based metrics

Turn RUM events into metrics that count what is ingested from then on, for
dashboards and monitors that last longer than the events. A metric is on one
event type (`session`, `view`, `action`, `error`, `resource`, `long_task`,
`vital`), counts events or takes a distribution of a measure, and is tagged
by attributes. Sessions and views are counted once: when they first match
(`uniqueness.when: match`, the default) or when they end
(`uniqueness.when: end`, with their final values: when the SDK marks them
inactive, or after 15 minutes without an update).

Create them on the [Metrics](/rum/generate-metrics) tab or through
`/api/v2/rum/config/metrics`; see [Generate Metrics](generate-metrics.md).

## API

Search RUM events with `POST /api/v2/rum/events/search` or
`GET /api/v2/rum/events`, and aggregate them with
`POST /api/v2/rum/analytics/aggregate`, as with Datadog's API. Alert on them
with `rum("…")` [monitors](monitors.md).

## Differences from Datadog

- Only the browser SDK is supported; mobile SDKs (iOS, Android, React
  Native, Flutter) are not.
- RUM applications are not created or managed: any `applicationId` is
  accepted, and one client token serves every site.
- There are no heatmaps and no geography from IP addresses.
