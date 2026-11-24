---
title: Downtimes
section: Monitors and Alerting
order: 30
app: /monitors/downtimes
description: Downtimes silence the notifications of some monitors, or of some of their groups, for a planned period such as a maintenance window or a deploy.
---

# Downtimes

A downtime silences [monitors](monitors.md) between a start and an end.
While it is active, the monitors it covers are still evaluated and their
groups still change state, but nothing is notified: no webhook, e-mail,
on-call page or workflow. The state changes are still recorded in each
monitor's **Events** and posted to the [event stream](events.md), so you can
see what happened during the maintenance.

A downtime covers a group of a monitor when all of these hold:

| Field | Covers |
| --- | --- |
| `monitor_id` | Only this monitor; absent covers every monitor |
| `monitor_tags` | Only monitors that carry all these tags (`team:shop`) |
| `scope` | `*` for every group, or `key:value` terms that must all match |
| `start`, `end` | From `start` (now if absent) until `end` (until canceled if absent), in epoch seconds |

Each term of the scope matches when the monitor has that tag, or when the
group has that value: `host:web-1` silences the `host:web-1` group of a
monitor grouped `by {host}`, and `team:shop` silences every group of the
monitors tagged `team:shop`. The scope can be a string (`"host:web-1,env:prod"`)
or a list of terms (`["host:web-1", "env:prod"]`).

> **Note:** a downtime only stops the notifications sent while it is active.
> When it ends, a group that went to Alert during the downtime is not
> notified again, unless the monitor has a `renotify_interval`: its next
> reminder is sent as usual.

To silence a monitor without a schedule, mute it instead: see
[Muting](monitors.md#muting).

## Schedule a downtime

On [Manage Downtimes](/monitors/downtimes), **Schedule Downtime** asks for:

- **Monitor**: one monitor, or **All monitors**.
- **Scope**: group or monitor tags, comma separated (`host:web-1,env:prod`),
  or empty for everything.
- **Start**: a date and time in your browser's time zone, now by default.
- **Duration**: 1 hour, 4 hours, 1 day, 1 week or **Until canceled**.
- **Message**: why, for whoever reads the list.

The list shows each downtime's status, monitor, scope, start, end and
message:

| Status | Meaning |
| --- | --- |
| `scheduled` | It starts later |
| `active` | It silences its monitors now |
| `ended` | Its end has passed |
| `canceled` | It was canceled before its end |

**Cancel** on a scheduled or active downtime ends it now and keeps it in the
list as `canceled`. **Remove** on an ended or canceled downtime deletes it.

## API

The routes take a `DD-API-KEY` header and follow Datadog's v1 downtime API.

| Method and path | What it does |
| --- | --- |
| `GET /api/v1/downtime` | Lists downtimes, latest start first, each with its computed `status` |
| `POST /api/v1/downtime` | Creates a downtime |
| `PUT /api/v1/downtime/{id}` | Replaces a downtime with the body given |
| `DELETE /api/v1/downtime/{id}` | Cancels a scheduled or active downtime; deletes an ended or canceled one |

Silence the `shop-api` monitors on one host for two hours, from now:

```sh
curl -X POST http://localhost:8080/api/v1/downtime \
  -H "DD-API-KEY: $ROBINE_DOG_API_KEY" -H "Content-Type: application/json" \
  -d "{
    \"scope\": [\"host:web-1\"],
    \"monitor_tags\": [\"service:shop-api\"],
    \"end\": $(( $(date +%s) + 7200 )),
    \"message\": \"Kernel upgrade on web-1\"
  }"
```

The response is the downtime with its `id` and `status`:

```json
{
  "id": "q3Zk9vT1xN0bR8cW2mLp4A",
  "scope": "host:web-1",
  "monitor_id": null,
  "monitor_tags": ["service:shop-api"],
  "start": 1764000000,
  "end": 1764007200,
  "message": "Kernel upgrade on web-1",
  "canceled": null,
  "status": "active"
}
```

A downtime whose `end` is not after its `start` is refused.

## Differences from Datadog

- Downtimes do not recur: there is no `recurrence` rule. Schedule one
  downtime per window.
- There is no v2 downtime API (`/api/v2/downtime`).
- `PUT` replaces the whole downtime rather than merging the fields given.
- No notification is sent when a downtime starts or ends.
