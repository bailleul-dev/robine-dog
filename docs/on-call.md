---
title: On-Call
section: Service Management
order: 30
app: /on-call
description: Rotation schedules and escalation policies per team. A monitor that names @oncall-<team> pages the team by e-mail, step after step, until someone acknowledges.
---

# On-Call

On-Call decides who is woken up when a monitor alerts. You describe
**schedules** (who takes turns being on call) and, for each team, an
**escalation policy** (whom to reach first, and whom next if nobody answers).
A [monitor](monitors.md) whose message names `@oncall-<team>` then opens a
**page** for that team when it alerts: Robine Dog e-mails the people of the
policy's first step, moves to the next step after a delay, and stops when
someone acknowledges the page. The page resolves by itself when the monitor
recovers.

Pages are sent by e-mail, so On-Call needs an SMTP relay: set
`ROBINE_DOG_SMTP_URL` (see [Before you start](#before-you-start)).

## Before you start

| Variable | Purpose |
| --- | --- |
| `ROBINE_DOG_SMTP_URL` | The SMTP relay: `smtp://user:pass@host:587?tls=required` (STARTTLS), `smtps://user:pass@host:465` (TLS), or `smtp://host:25` (no TLS) |
| `ROBINE_DOG_SMTP_FROM` | The sender address. Default: `robine-dog <robine-dog@localhost>` |
| `ROBINE_DOG_BASE_URL` | The address of Robine Dog that the e-mails link to, for the **Acknowledge** link |

Without `ROBINE_DOG_SMTP_URL`, pages are still opened and escalate, but
nobody receives them: each step's timeline entry is followed by *Some
e-mails were not sent (is ROBINE_DOG_SMTP_URL set?)*. See
[Configuration](configuration.md) for the other settings.

## Schedules

A schedule is a list of members (e-mail addresses) who take turns, in
order, for shifts of a fixed length.

| Field | Description |
| --- | --- |
| `name` | Required. Steps of a policy can name the schedule by its name or its id. |
| `members` | Required, at least one. E-mail addresses, in rotation order. |
| `length_hours` | Length of a shift, in hours. Default `168` (one week). |
| `start` | When the first member's first shift starts, in epoch milliseconds. Default: the time the schedule is created. |

The member on call at a given time is
`members[floor((now - start) / length_hours) mod len(members)]`. With
`length_hours: 24` and three members, the first is on call on day one, the
second on day two, the third on day three, then the first again.

In the UI, click **+ Schedule** on [On-Call](/on-call), enter the members
one per line and the shift length. The **Schedules** card shows each
schedule's rotation (`a@example.com → b@example.com`), its shift length and
who is **On call** right now. Editing a schedule keeps its `start`, so the
rotation does not jump.

## Escalation policies

An escalation policy belongs to a **team**: its `team` field is the handle
monitors use. A policy with `team: payments` is paged by `@oncall-payments`.

| Field | Description |
| --- | --- |
| `name` | Required. |
| `team` | Required. The handle: letters, digits, `_`, `-` and `.`, ending with a letter, digit or `_`. |
| `steps` | Required, at least one. Each step has `targets` and `delay_minutes`. |
| `steps[].targets` | Whom the step reaches: `schedule:<id or name>` (whoever is on call in that schedule at that moment), `user:<email>`, or a bare e-mail address. |
| `steps[].delay_minutes` | How long the step waits for an acknowledgement before the next step. Default `5`. |
| `repeat` | How many times the whole policy starts over after its last step. Default `0` through the API; the UI's form proposes `1`. |

In the UI, click **+ Escalation Policy**. Steps are written one per line,
as `<minutes>: <targets>`, targets separated by commas:

```text
5: schedule:primary
10: schedule:secondary, lead@example.com
```

The form lists your schedules with their `schedule:<id>` to copy. The
**Escalation policies** card shows each policy with its `@oncall-<team>`
handle and its steps.

If several policies have the same team, the first one found is used.

## Paging a team from a monitor

Name the team in the monitor's message:

```text
{{#is_alert}}Checkout errors above {{threshold}} on {{service.name}}.{{/is_alert}}
{{#is_recovery}}Checkout errors are back to normal.{{/is_recovery}}
@oncall-payments
```

When the monitor notifies:

- **Alert, Warn or No Data**, and the team has no open page for this
  monitor group: a page is opened. There is one page per monitor group and
  team, so a monitor grouped `by {host}` can open one page per host.
- **OK** (recovery): the team's open page for that group, triggered or
  acknowledged, is resolved with the timeline entry *Resolved: the monitor
  recovered*.
- A renotification, or a change between Alert and Warn, while a page is
  already open: nothing new. The open page keeps escalating.

Notifications silenced by a [mute or a downtime](downtimes.md) open no page.

> **Note:** handles are looked for in the message after its template is
> rendered. Keep `@oncall-<team>` outside `{{#is_alert}}…{{/is_alert}}`
> blocks, otherwise the recovery message does not name the team and the
> page is not resolved when the monitor recovers. See
> [Notifications](notifications.md) for the template syntax.

## How a page escalates

1. The page opens in the `triggered` status, at step 1. Robine Dog e-mails
   the step's targets right away.
2. Every 30 seconds, Robine Dog looks at the triggered pages. When the
   current step has waited its `delay_minutes`, the page moves to the next
   step and e-mails its targets.
3. After the last step, the policy starts over from step 1, `repeat` times.
   A policy of N steps sends at most N × (1 + `repeat`) rounds of e-mails;
   the page then stays triggered without further e-mails.
4. Acknowledging the page stops the escalation. Resolving it closes it.

The e-mail's subject is `[Page] <monitor notification title>`, its body
the rendered monitor message followed by
`Acknowledge: <ROBINE_DOG_BASE_URL>/on-call?page=<id>`. The link opens
On-Call with the page highlighted.

If the team has no escalation policy, the page is opened with the timeline
entry *No escalation policy for team <team>* and reaches nobody.

## Pages

The **Pages** table on [On-Call](/on-call) lists the latest 200 pages,
newest first, and refreshes every 15 seconds:

| Column | Shows |
| --- | --- |
| Triggered | When the page was opened |
| Team | The `@oncall-<team>` handle |
| Title | The monitor notification's title, or the title typed for a manual page |
| Status | `triggered`, `acknowledged` or `resolved` |
| Latest | The last timeline entry, such as `Step 2: paged b@example.com` |

**Acknowledge** (on triggered pages) records who acknowledged and stops the
escalation. **Resolve** (on any page not yet resolved) closes it. Both are
written in the page's timeline with the name of the person signed in, or
`api` through the API.

## API

The endpoints accept the `DD-API-KEY` header. Request bodies can be the
plain object or Datadog's `{"data": {"attributes": {…}}}` envelope.

| Method and path | Description |
| --- | --- |
| `GET /api/v2/on-call/schedules` | List schedules, each with `on_call`: who is on call now |
| `POST /api/v2/on-call/schedules` | Create a schedule |
| `PUT /api/v2/on-call/schedules/{id}` | Replace a schedule |
| `DELETE /api/v2/on-call/schedules/{id}` | Delete a schedule |
| `GET`, `POST /api/v2/on-call/escalation-policies` | List or create escalation policies |
| `PUT`, `DELETE /api/v2/on-call/escalation-policies/{id}` | Replace or delete a policy |
| `GET /api/v2/on-call/teams/{team}/on-call` | Who each step of the team's policy reaches right now |
| `GET /api/v2/on-call/pages` | The latest 200 pages |
| `POST /api/v2/on-call/pages` | Page a team by hand |
| `POST /api/v2/on-call/pages/{id}/acknowledge` | Acknowledge a page |
| `POST /api/v2/on-call/pages/{id}/resolve` | Resolve a page |

Create a weekly schedule and a policy for the `payments` team:

```sh
curl -X POST http://localhost:8080/api/v2/on-call/schedules \
  -H "DD-API-KEY: $ROBINE_DOG_API_KEY" -H "Content-Type: application/json" \
  -d '{"name": "payments-primary", "members": ["alice@example.com", "bob@example.com"], "length_hours": 168}'

curl -X POST http://localhost:8080/api/v2/on-call/escalation-policies \
  -H "DD-API-KEY: $ROBINE_DOG_API_KEY" -H "Content-Type: application/json" \
  -d '{"name": "Payments", "team": "payments", "repeat": 1,
       "steps": [{"targets": ["schedule:payments-primary"], "delay_minutes": 5},
                 {"targets": ["lead@example.com"], "delay_minutes": 10}]}'
```

Page the team by hand (`team` can also be given as
`target.identifier`, and the message as `description` or `message`):

```sh
curl -X POST http://localhost:8080/api/v2/on-call/pages \
  -H "DD-API-KEY: $ROBINE_DOG_API_KEY" -H "Content-Type: application/json" \
  -d '{"data": {"attributes": {"team": "payments", "title": "Card payments failing", "description": "The PSP returns 502 since 14:05."}}}'
```

A [workflow](workflows.md) can page a team too, with its `oncall.page` step.

## Differences from Datadog

- Pages are sent by **e-mail only**: no SMS, phone call or mobile push.
  Acknowledging happens in the UI or through the API, not by replying.
- A schedule is a single rotation of fixed-length shifts. There are no
  layers, overrides, hand-off times or time zones.
- The team handle is the policy's `team` field; it is not tied to a team
  object.
- The API paths follow Datadog's, but the bodies are simpler than Datadog's
  On-Call API: no relationships, layers or rotations objects.
