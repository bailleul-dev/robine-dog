---
title: Notifications
section: Monitors and Alerting
order: 20
app: /monitors/webhooks
description: A monitor's message is a Datadog template. The @-handles it names receive it when a group changes state: webhooks, e-mail addresses, on-call teams and workflows.
---

# Notifications

When a group of a [monitor](monitors.md) changes state, Robine Dog renders
the monitor's message for that group and sends it to every handle the
rendered message names. The message uses Datadog's template syntax, so a
message written for Datadog works unchanged, as long as its handles exist
here.

| Handle | Sends to |
| --- | --- |
| `@webhook-<name>` | The [webhook](#webhooks) of that name |
| `@<address>` (`@ops@example.com`) | That e-mail address, through your [SMTP relay](#e-mail) |
| `@oncall-<team>` | The team's [on-call](on-call.md) escalation policy |
| `@workflow-<name>` | Starts the [workflow](workflows.md) of that name |

## When a notification is sent

A notification is sent for a group when:

- its state changes: `Triggered` (to Alert), `Warn`, `No Data`, `Recovered`
  (back to OK from any of them);
- it is still not OK `renotify_interval` minutes after its last
  notification: `Re-Triggered`, `Re-Warn` or `Re-No Data`. The
  `escalation_message`, rendered with the same variables, is added to the
  message of these reminders.

A new group that starts OK sends nothing. A group that is muted, or covered
by an active [downtime](downtimes.md), sends nothing either, but its state
change is still recorded in the monitor's events and posted to the
[event stream](events.md).

The notification's title is the transition, the group and the name, with the
priority first when there is one:

```text
[P2] [Triggered on {host:web-1,env:prod}] High CPU
```

Each state change is listed on the monitor's page under **Events**, with the
handles reached in **Notified**. A webhook that failed shows as
`@webhook-ops (failed: HTTP 500 Internal Server Error)`.

> **Note:** handles are read from the *rendered* message. A handle inside
> `{{#is_alert}}…{{/is_alert}}` only receives alerts, not recoveries. Put
> `@oncall-<team>` outside the conditional sections, or in an
> `{{#is_recovery}}` section too, so that the recovery resolves the page.

## Message template

### Conditional sections

`{{#name}}…{{/name}}` keeps its content when the condition holds;
`{{^name}}…{{/name}}` keeps it when it does not. Sections can be nested.

| Section | True when |
| --- | --- |
| `is_alert` | The group is in Alert |
| `is_warning` | The group is in Warn |
| `is_no_data` | The group is in No Data |
| `is_recovery` | The group is back to OK from any other state |
| `is_alert_recovery` | Back to OK from Alert |
| `is_warning_recovery` | Back to OK from Warn |
| `is_no_data_recovery` | Back to OK from No Data |
| `is_alert_to_warning` | From Alert to Warn |
| `is_warning_to_alert` | From Warn to Alert |
| `is_renotify` | The notification is a reminder |
| `is_match "<variable>" "<a>" "<b>"…` | The variable contains one of the values |
| `is_exact_match "<variable>" "<a>"…` | The variable equals one of the values |

### Variables

| Variable | Value |
| --- | --- |
| `{{value}}` | The group's evaluated value (`91.5`; `N/A` without data) |
| `{{threshold}}` | The alert threshold of the query |
| `{{warn_threshold}}` | `thresholds.warning` |
| `{{alert_recovery_threshold}}`, `{{warn_recovery_threshold}}` | The recovery thresholds |
| `{{comparator}}` | `>`, `>=`, `<`, `<=`, `==` or `!=` |
| `{{last_triggered_at}}` | When the group last went to Alert (`2026-11-24 14:02:00 UTC`) |
| `{{last_triggered_at_epoch}}` | The same, in epoch milliseconds |
| `{{monitor_id}}` or `{{monitor.id}}` | The monitor's id |
| `{{monitor_name}}` or `{{monitor.name}}` | The monitor's name |
| `{{group}}` | The group (`host:web-1,env:prod`; `*` without groups) |
| `{{<key>.name}}` | The value of a group key: `{{host.name}}`, `{{service.name}}`, `{{[kube_namespace].name}}` |

An unknown variable renders as an empty string. Numbers are printed
without trailing zeros, with four decimals at most.

### Example

```text
{{#is_alert}}
CPU is at {{value}}% on {{host.name}} (threshold {{threshold}}%).
{{#is_match "host.name" "db-"}}This is a database host. @oncall-dba{{/is_match}}
{{/is_alert}}
{{#is_warning}}CPU is above {{warn_threshold}}% on {{host.name}}.{{/is_warning}}
{{#is_recovery}}CPU is back to normal on {{host.name}}.{{/is_recovery}}
@webhook-ops-alerts @ops@example.com
```

## Webhooks

Webhooks are named HTTP endpoints, configured like Datadog's Webhooks
integration. Create them on [Webhooks](/monitors/webhooks) (**New Webhook**)
or through the API; **Test** on a row sends a sample notification and shows
the HTTP status.

| Field | Meaning |
| --- | --- |
| `name` | Letters, digits, `_`, `-` and `.`; the monitor message names it as `@webhook-<name>` |
| `url` | An `http://` or `https://` URL; the request is a `POST` |
| `payload` | The body, with `$VARIABLES` replaced; empty uses Datadog's default payload (below) |
| `custom_headers` | A JSON object as a string, `{"Authorization": "Bearer …"}`; variables are replaced in it too |
| `encode_as` | `json` (default) or `form` |

With `json`, the body is sent as `application/json` and the variables are
escaped for JSON, so `"$EVENT_MSG"` stays valid JSON when the message has
quotes or line breaks. With `form`, the payload is still written as a JSON
object: its keys are sent as `application/x-www-form-urlencoded` fields. A
payload that is not an object is sent as one `payload` field.

Requests time out after 10 seconds, with the user agent
`robine-dog-webhooks`. A response other than 2xx counts as a failure; it is
not retried.

The default payload, used when `payload` is empty:

```json
{"body": "$EVENT_MSG", "last_updated": "$LAST_UPDATED", "event_type": "$EVENT_TYPE", "title": "$EVENT_TITLE", "date": "$DATE", "org": {"id": "$ORG_ID", "name": "$ORG_NAME"}, "id": "$ID"}
```

### Payload variables

| Variable | Value |
| --- | --- |
| `$EVENT_TITLE`, `$ALERT_TITLE` | The notification's title |
| `$EVENT_MSG`, `$TEXT_ONLY_MSG` | The rendered message |
| `$ALERT_TRANSITION` | `Triggered`, `Warn`, `No Data`, `Recovered`, `Re-Triggered`, `Re-Warn`, `Re-No Data` |
| `$ALERT_TYPE` | `error` (Alert), `warning`, `info` (No Data) or `success` (OK) |
| `$ALERT_STATUS` | The group, comparator, threshold, state and value: `host:web-1 > 80 (Alert: 91.5)` |
| `$ALERT_ID`, `$AGGREG_KEY` | The monitor's id |
| `$ALERT_QUERY` | The monitor's query |
| `$ALERT_METRIC` | The metric of a metric monitor, empty otherwise |
| `$ALERT_SCOPE` | The group (`host:web-1`), empty without groups |
| `$ALERT_PRIORITY` | `P1` to `P5`, or empty |
| `$ALERT_CYCLE_KEY` | `<monitor id>-<group>` |
| `$EVENT_TYPE` | The monitor type with underscores and `_monitor`: `metric_alert_monitor`, `log_alert_monitor` |
| `$LINK` | The monitor's page, from `ROBINE_DOG_BASE_URL` |
| `$HOSTNAME` | The group's `host`, if it has one |
| `$TAGS` | The monitor's tags and the group's, comma separated |
| `$DATE`, `$LAST_UPDATED`, `$ID` | The time of sending, in epoch milliseconds |
| `$ORG_ID`, `$ORG_NAME` | `1` and `robine-dog` |
| `$PRIORITY` | `normal` |
| `$USER`, `$USERNAME`, `$SNAPSHOT` | Empty |

A payload for an [ntfy](https://ntfy.sh) topic, with a custom header:

```sh
curl -X POST http://localhost:8080/api/v1/integration/webhooks/configuration/webhooks \
  -H "DD-API-KEY: $ROBINE_DOG_API_KEY" -H "Content-Type: application/json" \
  -d '{
    "name": "ops-alerts",
    "url": "https://ntfy.sh/my-topic",
    "payload": "{\"title\": \"$EVENT_TITLE\", \"message\": \"$EVENT_MSG\", \"click\": \"$LINK\"}",
    "custom_headers": "{\"X-Priority\": \"4\"}",
    "encode_as": "json"
  }'
```

### Webhooks API

| Method and path | What it does |
| --- | --- |
| `POST /api/v1/integration/webhooks/configuration/webhooks` | Creates a webhook; the name must be new |
| `GET /api/v1/integration/webhooks/configuration/webhooks/{name}` | Reads one |
| `PUT /api/v1/integration/webhooks/configuration/webhooks/{name}` | Changes the fields given; a new `name` renames it |
| `DELETE /api/v1/integration/webhooks/configuration/webhooks/{name}` | Deletes it |

## E-mail

An address preceded by `@` in the message (`@ops@example.com`) receives the
notification as a plain-text e-mail: the title is the subject, the body is
the rendered message followed by a link to the monitors page. Each address
is sent to once per notification.

E-mail needs an SMTP relay:

| Variable | Meaning |
| --- | --- |
| `ROBINE_DOG_SMTP_URL` | `smtp://host:25` (no TLS), `smtp://user:password@host:587?tls=required` (STARTTLS), or `smtps://user:password@host:465` (TLS) |
| `ROBINE_DOG_SMTP_FROM` | The sender; default `robine-dog <robine-dog@localhost>` |

```sh
export ROBINE_DOG_SMTP_URL="smtps://alerts%40example.com:$SMTP_PASSWORD@smtp.example.com:465"
export ROBINE_DOG_SMTP_FROM="Robine Dog <alerts@example.com>"
```

Encode `@` and other special characters of the user name and password
(`%40`). Without `ROBINE_DOG_SMTP_URL`, e-mail handles are ignored and a
warning is logged. See [Configuration](configuration.md).

## On-call and workflows

`@oncall-<team>` opens a page for the team when a group leaves OK (Alert,
Warn or No Data), unless that group already has an open page for the team.
The page follows the team's escalation policy until someone acknowledges it,
and is resolved when the group recovers. See [On-Call](on-call.md).

`@workflow-<name>` starts the enabled workflow of that name with each
notification, reminders and recoveries included. The run's trigger holds
`monitor_id`, `monitor_name`, `title`, `message`, `status`, `previous`,
`group`, `tags` and `value`. See [Workflow Automation](workflows.md).

## Synthetic tests

A [synthetic test](synthetics.md) notifies the `@webhook-` handles and
e-mail addresses of its message when it starts failing and when it passes
again. Its message is sent as written, without template rendering, and its
webhooks get `$EVENT_TYPE` `synthetics_alert` and a `$LINK` to the test.

## Differences from Datadog

- The integrations handles of Datadog (`@slack-…`, `@pagerduty-…`,
  `@teams-…`) do not exist; use a webhook to reach those services.
- Template variables are not rendered in the monitor name, only in the
  message.
- Variables of the triggering event, such as `{{log.message}}` or
  `{{span.attributes…}}`, are not available; group keys are.
- A failed webhook is not retried.
