---
title: Incidents and status pages
section: Service Management
order: 20
app: /incidents
description: Declare incidents, follow them through a timeline of notes and changes, and publish a public status page whose components take their state from synthetic tests and monitors.
---

# Incidents and status pages

An incident records something that is wrong and how it is handled: its
severity, its state, who leads the response, the services affected and a
timeline of what was seen and done. Declaring an incident and changing its
state post [events](events.md), so they show next to monitor alerts and
deployments.

The **status page** is a public page at `/status` that tells your users
which parts of your product work, from the state of
[synthetic tests](synthetics.md) and [monitors](monitors.md), and lists the
incidents you choose to make public.

## Incident fields

| Field | Values |
| --- | --- |
| ID | A number shown as `IR-12`, one more than the highest so far |
| Title | Required |
| Severity | `SEV-1` to `SEV-5`, or `UNKNOWN` (the default) |
| State | `active` (on declaration), `stable` or `resolved` |
| Commander | Free text; the person who declares the incident by default |
| Services | The services affected, such as `shop-api, shop-web` |
| What we know | A summary: impact, cause, what is being done |
| Show on the status page | Whether the incident is public |

## Declare an incident

- From [Incidents](/incidents), click **Declare Incident**, then enter a
  title, a severity, the services and what you know.
- From a [Watchdog](watchdog.md) story that is still ongoing, click
  **Declare Incident**: the form opens with the story's title.
- Through the API, see [API](#api).

The new incident opens on its page.

## Manage an incident

The incident list shows every incident, ongoing ones first, with its state,
severity, commander, when it was declared and how long it lasted (or has
lasted so far). It refreshes every minute.

An incident's page has:

- **What we know**: the summary, saved when you leave the field.
- **Timeline**: the notes, newest first. Type a note and click **Add**.
  The timeline also records the declaration and every change of state or
  severity, with the time and the person who made it.
- The side panel: **State**, **Severity**, **Commander**, **Services** and
  **Show on the status page**, each saved when changed; how long the
  incident has lasted; and for each service a link to its error logs from
  an hour before the declaration to the resolution.
- **Resolve** sets the state to `resolved`. Setting the state back to
  `active` or `stable` reopens the incident.
- **Delete** removes the incident.

## Events posted

An event is posted with `source:incident` when an incident is declared and
each time its state changes:

| When | Event title | Status |
| --- | --- | --- |
| Declared | `IR-12 declared: Checkout errors` | `error` |
| State changes to `stable` | `IR-12 is stable: Checkout errors` | `error` |
| Resolved | `IR-12 resolved: Checkout errors` | `ok` |

Each event carries the tags `severity:<sev>` (in lower case, `severity:sev-2`)
and `incident_state:<state>`, the incident's first service as `service`, and
the attributes `@incident.id` and `@incident.public_id`. Find them in the
[Event Explorer](/event/explorer) with:

```text
source:incident
```

Changing the severity alone records it in the timeline without posting an
event.

Incidents that name services also count as failures in
[DORA metrics](cd-visibility.md): a deployment followed by an incident on
its service before the next deployment counts toward the change failure
rate.

## Status page

Configure the page in [Status Pages](/status-pages):

1. Check **Publish the status page**. Once published, anyone can read
   `http://localhost:8080/status` without signing in.
2. Set a **Title** (default `Status`) and a **Description**.
3. **Add a component** for each part of your product: a name shown to
   users, and the synthetic test or monitor its state comes from.
4. Click **Save**.

A component's state:

| Source | Operational | Degraded | Outage | Unknown |
| --- | --- | --- | --- | --- |
| Synthetic test | Its last run passed | — | Its last run failed | No run yet |
| Monitor | `OK` | `Warn` | `Alert` | No group yet, or `No Data` |

The settings page shows each component's current state, and its uptime for
a synthetic test.

### What the public page shows

- A banner: *Some systems are down* when a component is in outage, *Some
  systems are degraded* when one is degraded, *An incident is in progress*
  when a public incident is not resolved, otherwise *All systems
  operational*.
- **Ongoing incidents**: the public incidents not yet resolved, with their
  state, start time, summary and timeline.
- **Components**: each with its state. Components backed by a synthetic
  test also show 30 daily bars and their uptime over those 30 days (the
  share of runs that passed): a day is green when no run failed, amber when
  less than 5% of runs failed, red above that, and grey without runs.
- **Past 30 days**: public incidents resolved in the past 30 days.

The page is plain HTML with no script to load, so it stays readable when the
rest of the application is struggling. It reloads itself every 60 seconds.
`/status` returns `404` while the page is not published.

> **Warning:** the timeline of a public incident is published with it,
> including its notes (severity changes are left out). Keep internal
> details out of the notes of incidents you show on the status page.

## API

Robine Dog implements Datadog's v2 incidents API, with a `DD-API-KEY`
header.

| Method and path | Description |
| --- | --- |
| `GET /api/v2/incidents` | List every incident |
| `POST /api/v2/incidents` | Declare an incident (`201`) |
| `GET /api/v2/incidents/{id}` | One incident, by its id or its number (`12`) |
| `PATCH /api/v2/incidents/{id}` | Change its title, severity, state, summary, services or visibility |
| `DELETE /api/v2/incidents/{id}` | Delete it (`204`) |

The attributes map to the incident's fields:

| Attribute | Field |
| --- | --- |
| `title` | Title |
| `fields.severity.value` (or `severity`) | Severity |
| `fields.state.value` (or `state`) | State |
| `fields.summary.value` (or `customer_impact_scope` on read) | What we know |
| `fields.services.value` | Services |
| `customer_impacted` | Show on the status page |

Declare an incident:

```sh
curl -X POST http://localhost:8080/api/v2/incidents \
  -H "DD-API-KEY: $ROBINE_DOG_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"data": {"type": "incidents", "attributes": {
        "title": "Checkout errors",
        "fields": {
          "severity": {"type": "dropdown", "value": "SEV-2"},
          "services": {"type": "autocomplete", "value": ["shop-api"]}
        }}}}'
```

Mark incident 12 stable:

```sh
curl -X PATCH http://localhost:8080/api/v2/incidents/12 \
  -H "DD-API-KEY: $ROBINE_DOG_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"data": {"type": "incidents", "attributes": {"fields": {"state": {"type": "dropdown", "value": "stable"}}}}}'
```

Incidents are also available to AI agents through the
[MCP server](api.md) (`search_datadog_incidents`, `get_datadog_incident`).

## Differences from Datadog

- One status page per organization, configured in the UI; Datadog's status
  page API is not implemented.
- Incidents have a fixed set of fields. Custom fields, roles other than the
  commander, tasks, todos, postmortems and integrations (Slack channels,
  Jira tickets) are not implemented.
- The API cannot set the commander or add timeline notes; use the UI.
- Monitors do not declare incidents by themselves.
