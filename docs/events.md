---
title: Events
section: Service Management
order: 50
app: /event/explorer
description: Events record what happened, such as a monitor changing state, an Agent noticing a container die, a deployment or an incident. Search them in the Event Explorer, post your own through the API and alert on them with event monitors.
---

# Events

An event is a dated record of something that happened: a monitor started
alerting, a container died on a host, version 1.4.2 was deployed, an
incident was declared. Robine Dog collects the events its own features
produce, the events Datadog Agents send and the events you post through the
API, and shows them all in the [Event Explorer](/event/explorer).

Events are stored like logs, in a table of their own, so they are searched
with the same [search syntax](search-syntax.md). They are kept as long as
metrics: 455 days by default (`ROBINE_DOG_METRICS_RETENTION_DAYS`, see
[Configuration](configuration.md)).

## Event fields

| Field | Search with | Content |
| --- | --- | --- |
| Status | `status:` | The alert type: `error`, `warn`, `info` or `ok` |
| Source | `source:` | Where the event comes from, see [Sources](#sources) |
| Message | Free text | The title, then the text on the following lines |
| Service | `service:` | The `service:` tag of the event, if any |
| Host | `host:` | The host named by the event |
| Tags | `env:prod` | The event's tags |
| Attributes | `@evt.title`, `@evt.text`, `@evt.priority`… | The title, text and other fields of the event; some sources add their own |

An API event's `alert_type` becomes its status: `error` stays `error`,
`warning` becomes `warn`, `success` becomes `ok` and anything else `info`.

## Sources

| `source:` | Posted when | Extra attributes |
| --- | --- | --- |
| `alert` | A [monitor](monitors.md) group changes state | `@monitor.id`, `@monitor.name`, `@monitor.group`, `@monitor.from`, `@monitor.to`, `@monitor.value`; the tag `monitor_id:<id>` |
| `synthetics` | A [synthetic test](synthetics.md) starts failing or recovers | `@synthetics.check_id`, `@synthetics.test_name`, `@synthetics.notified`; the tag `check_id:<id>` |
| `watchdog` | [Watchdog](watchdog.md) opens a story | The tag `watchdog_kind:<kind>` |
| `incident` | An [incident](incidents.md) is declared or changes state | `@incident.id`, `@incident.public_id`; the tags `severity:` and `incident_state:` |
| The Agent's source, such as `docker` | A Datadog [Agent](agent.md) reports an event in its metadata payload | — |
| `source_type_name`, else `api` | You post an event through the API | `@evt.priority`, `@evt.aggregation_key`, `@evt.event_type`, `@evt.device_name` when given |
| `user` | You post an event from the Event Explorer | — |

A monitor event's status follows the new state: `error` for `Alert`, `warn`
for `Warn`, `ok` for `OK` and `info` for `No Data`. It also carries the
monitor's tags and the group's tags, and its host and service when the group
has them.

## The Event Explorer

[Event Management](/event/explorer) shows the events of the selected time
range, the past day by default:

- The **search bar** takes the [search syntax](search-syntax.md):

  ```text
  source:alert status:error service:shop-api @monitor.id:12
  ```

- The **histogram** counts events per status over time. Drag across it to
  zoom on a period.
- The **list** shows each event's date, status, source, title, the start of
  its text and its tags, newest first, 100 at a time (**Load more** for the
  next page).
- Click an event to open its **side panel**: the full text, the tags (click
  one to search for it) and every attribute.

**Post Event** opens a form with a title, a text, a type (`info`,
`success`, `warning`, `error`) and tags. The event is posted with
`source:user`.

## Post events through the API

`POST /api/v1/events` takes Datadog's v1 event format, with a `DD-API-KEY`
header (or a `dd-api-key` or `api_key` query parameter):

```sh
curl -X POST http://localhost:8080/api/v1/events \
  -H "DD-API-KEY: $ROBINE_DOG_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "title": "Deployed shop-api 1.4.2",
    "text": "Rolled out by the CI pipeline #812",
    "alert_type": "success",
    "source_type_name": "deploy",
    "tags": ["service:shop-api", "env:prod", "version:1.4.2"]
  }'
```

| Field | Description |
| --- | --- |
| `title` | Required; the request fails with `400` without one |
| `text` | The body |
| `alert_type` | `error`, `warning`, `info` (default) or `success` |
| `date_happened` | Epoch seconds; default now |
| `host` | The host the event is about |
| `tags` | A list of `key:value` tags; `service:<name>` also sets the service |
| `source_type_name` | The source; default `api` |
| `priority`, `aggregation_key`, `event_type`, `device_name` | Kept as `@evt.*` attributes |

The response is `202 Accepted` with the stored title, date, tags and host.
Events are written in batches, so a new event takes a moment to show up in
searches.

## Search events through the API

These endpoints require a `DD-API-KEY` header.

| Method and path | Description |
| --- | --- |
| `POST /api/v2/events/search` | Datadog's v2 search: `{"filter": {"query", "from", "to"}, "page": {"limit", "cursor"}}`, newest first |
| `GET /api/v2/events` | The same with `filter[query]`, `filter[from]`, `filter[to]`, `page[limit]`, `page[cursor]` |
| `GET /api/v1/events` | Datadog's v1 stream: `start` and `end` (epoch seconds, required), `tags` and `sources` (comma-separated) |

In the v2 endpoints, `from` and `to` take `now`, `now-15m` (units `s`,
`m`, `h`, `d`, `w`, `mo`), epoch seconds or milliseconds, or an ISO 8601
date; the defaults are `now-15m` and `now`.
`limit` defaults to 10 and goes up to 1,000; the response's
`meta.page.after` is the cursor of the next page.

```sh
curl -X POST http://localhost:8080/api/v2/events/search \
  -H "DD-API-KEY: $ROBINE_DOG_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"filter": {"query": "source:alert status:error", "from": "now-1d", "to": "now"},
       "page": {"limit": 50}}'
```

The v1 endpoint returns up to 1,000 events, each with its `title`, `text`,
`date_happened`, `alert_type`, `host`, `source` and `tags`.

## Event monitors

An event monitor counts the events that match a search and alerts above a
threshold. Its query has the same form as a
[log monitor's](monitors.md#log-monitors), with `events(…)`:

```text
events("source:deploy env:prod status:error").rollup("count").by("service").last("1h") > 0
```

Its monitor type is `event-v2 alert`. See [Monitors](monitors.md) for
thresholds, groups and notifications.

## Differences from Datadog

- DogStatsD events (`_e{…}` datagrams) are ignored; post events through the
  API instead.
- `priority` is stored but not used to filter: the v1 endpoint returns
  every event as `normal`.
- Events are kept 455 days by default, like metrics, rather than Datadog's
  fixed retention.
