---
title: Workflow Automation and App Builder
section: Service Management
order: 60
app: /workflows
description: Workflows chain HTTP calls, Kubernetes actions, cases, downtimes, pages and approvals, started by a monitor, a security signal, a schedule, a click or the API. App Builder puts forms in front of them.
---

# Workflow Automation and App Builder

A workflow is a list of **steps** that Robine Dog runs one after the other
when something **triggers** it: a monitor that names `@workflow-<name>` in
its message, a new security signal, a cron schedule, a click on **Run**, an
App Builder form, or the API. Steps call any HTTP endpoint (a Slack,
Discord or ntfy webhook, your own API), restart or scale a Kubernetes
workload, open a [case](case-management.md), schedule a
[downtime](downtimes.md), mute a [monitor](monitors.md), page an
[on-call](on-call.md) team, wait, or stop for a human approval.

Each step's parameters are templates: they can use what triggered the
workflow (`{{ trigger.* }}`) and what earlier steps returned
(`{{ steps.<name>.* }}`). Every run is kept with its trigger, the
parameters each step used, its output or error.

## Creating a workflow

On [Workflow Automation](/workflows), click **+ New Workflow**.

| Field | Description |
| --- | --- |
| Name | Required and unique: letters, digits, `_`, `-` and `.`. It is the `@workflow-<name>` handle. |
| Description | Optional. |
| Enabled | A disabled workflow is not started by monitors, signals or its schedule. **Run**, apps and the API still start it. |
| On new security signals | Start the workflow for each new [security signal](siem.md); with a rule id, only for that rule's signals. |
| Schedule | A cron expression, see [Schedules](#schedules). |
| Steps | The steps, in order. **+ Step** adds one. |

Each step has a **name** (unique in the workflow, used in
`{{ steps.<name>.* }}`), an **action**, its **parameters** as JSON, an
optional **if** condition and a **continue on error** box. Picking an
action fills the parameters with an example to edit.

## Triggers

| Trigger | How | `trigger_type` | `trigger` contains |
| --- | --- | --- | --- |
| Monitor | The monitor's message names `@workflow-<name>` | `monitor` | `monitor_id`, `monitor_name`, `title`, `message`, `status`, `previous`, `group`, `tags`, `value` |
| Security signal | **On new security signals**, or the detection rule's message names `@workflow-<name>` | `security_signal` | `rule_id`, `rule_name`, `severity`, `group`, `tags`, `value`, `detected_at` |
| Schedule | The cron expression matches the current minute | `schedule` | `scheduled_at` (epoch milliseconds) |
| Run button, API | **Run** on the workflow's row, or `POST /api/v2/workflows/{id}/instances` | `api` | The request's payload (empty for **Run**) |
| App | A button of an [App Builder](#app-builder) app | `app` | `app` (the app's name), `values` (what was typed) |

### Monitors

A monitor starts the workflow on **every notification** it sends: when a
group alerts, warns, has no data, recovers, and when it renotifies. In the
trigger, `status` is `Alert`, `Warn`, `No Data` or `OK`, and `previous` the
state before. Use a step's `if` to act on some of them only:

```text
Disk almost full on {{host.name}} ({{value}}%). @workflow-clean-tmp
```

```json
{"name": "only-alerts", "action": "http", "if": "{{ trigger.status }} == Alert", "params": {"url": "https://ntfy.example.com/ops", "body": "{{ trigger.title }}"}}
```

Notifications silenced by a mute or a [downtime](downtimes.md) start no
workflow. The handle is read from the rendered message, so a handle inside
`{{#is_alert}}…{{/is_alert}}` only fires on alerts (see
[Notifications](notifications.md)). A handle naming no enabled workflow is
ignored and logged.

> **Note:** the handle takes every letter, digit, `_`, `-` and `.` after
> `@workflow-`. Do not end a sentence right after it: `@workflow-restart.`
> looks for a workflow named `restart.`.

### Security signals

A workflow runs when a signal is **created**, not each time an open signal
is seen again. It runs if its **On new security signals** box is checked
(and its rule id is empty or matches), or if the rule's message names
`@workflow-<name>`.

### Schedules

The schedule is a five-field cron expression, in **UTC**:
`minute hour day-of-month month day-of-week`. Each field takes `*`, a
number, a range (`8-18`), a list (`1,15`) and a step (`*/15`, `0-45/10`).
Day of week runs from `0` (Sunday) to `6`; `7` is Sunday too. Names such as
`MON` are not supported.

| Expression | Runs |
| --- | --- |
| `*/15 * * * *` | Every 15 minutes |
| `0 8 * * 1-5` | At 08:00 UTC on weekdays |
| `30 2 1 * *` | At 02:30 UTC on the first day of each month |

Robine Dog checks the schedules once a minute.

## Step actions

| Action | Parameters | Output |
| --- | --- | --- |
| `http` | `url` (required, `http://` or `https://`), `method` (default `POST`), `headers` (object), `body` (a string, or a JSON value sent as JSON), `allow_errors` (default `false`) | `status`, `body` (parsed as JSON when it is JSON, otherwise the first 10,000 characters of text) |
| `k8s.restart` | `namespace` (default `default`), `kind` (`deployment`, `statefulset` or `daemonset`; default `deployment`), `name` | `status`, `kind`, `name` |
| `k8s.scale` | `namespace`, `kind` (`deployment` or `statefulset`), `name`, `replicas` | `status`, `kind`, `name` |
| `k8s.delete_pod` | `namespace` (default `default`), `name` | `status`, `kind`, `name` |
| `case.create` | `title` (required), `description`, `priority`, `project`, `assignee`, as in the [cases API](case-management.md) | `id`, `key` |
| `downtime.create` | `scope` (default `*`), `monitor_id` or `monitor_tags`, `message`, `minutes` (default `60`) | `id`, `end` |
| `monitor.mute` | `monitor_id` (required), `scope` (default `*`), `minutes` (none: muted until unmuted) | `muted` |
| `oncall.page` | `team`, `title`, `message` | `id`, `status` |
| `approval` | `message` | The run waits for **Approve** or **Reject** |
| `sleep` | `seconds` (default `1`, at most `3600`) | `slept` |

Details:

- `http`: a request times out after 30 seconds. A string `body` gets a
  `Content-Type` of `application/json` when it parses as JSON, `text/plain`
  otherwise, unless `headers` sets one. A response status of 400 or more
  fails the step, unless `allow_errors` is `true`.
- `k8s.restart` sets the `kubectl.kubernetes.io/restartedAt` annotation on
  the pod template, as `kubectl rollout restart` does. See
  [Kubernetes steps](#kubernetes-steps) for the permissions they need.
- `monitor_id`, `minutes`, `replicas` and `seconds` can be numbers or
  strings, so `"{{ trigger.monitor_id }}"` works.

## Templates

In parameters, every string is a template. `{{ path }}` is replaced by the
value at `path` in the run's context:

| Path | Value |
| --- | --- |
| `trigger.*` | What started the run (see [Triggers](#triggers)) |
| `inputs.*` | The same as `trigger.*` |
| `steps.<name>.*` | The output of an earlier step, such as `steps.call.status` or `steps.call.body.items[0].id` |
| `workflow.id`, `workflow.name` | The workflow itself |

Paths use dots for keys and `[n]` for list items. A path that does not
exist renders as an empty string. A number or an object renders as JSON.
Add `| json` to render a value as JSON, quoted when it is a string, to build
a JSON body safely:

```json
{"body": "{\"text\": {{ trigger.title | json }}, \"host\": {{ trigger.tags.host | json }}}"}
```

## Conditions and errors

A step's `if` is rendered as a template, then evaluated:

| Form | True when |
| --- | --- |
| `a == b` | Both sides are equal, as text, after trimming |
| `a != b` | They differ |
| `a contains b` | `b` is part of `a` |
| `a` | `a` is not empty, `false`, `0` or `null` |

```text
{{ trigger.status }} == Alert
{{ steps.check.status }} != 200
{{ trigger.group }} contains env:prod
```

A step whose condition is false is marked `skipped`, and has no output in
`steps.<name>`.

When a step fails, the run stops and is `failed`, with the step's name and
error. With **continue on error**, the run goes on and the step's output is
`{"error": "<message>"}`.

## Approvals

An `approval` step pauses the run in the `waiting_approval` status, with its
rendered `message`. Open the run and click **Approve** to continue with the
next step, or **Reject** to end the run as `rejected`. The step records who
decided. Approvals are answered in the UI only.

## Runs

The **Runs** table on [Workflow Automation](/workflows) lists the latest
200 runs, newest first, and refreshes every 5 seconds: when each started,
the workflow, the trigger type, the status, the number of steps and the
error. A run's status is `running`, `waiting_approval`, `succeeded`,
`failed` or `rejected`.

Clicking a run opens it in a side panel: run number, trigger type, start
time and duration, the error, **Approve** and **Reject** when it waits, the
**Trigger** as JSON, and each step with its status (`succeeded`, `failed`,
`skipped`, `waiting`, `approved`, `rejected`), duration, rendered
parameters and output. Failed and waiting steps open by themselves.

## Kubernetes steps

Kubernetes steps call the API server of the cluster Robine Dog runs in,
with its pod's service account (`KUBERNETES_SERVICE_HOST` and the token in
`/var/run/secrets/kubernetes.io/serviceaccount`). Outside a cluster, they
fail with *Kubernetes actions need robine-dog to run in a cluster*.

What the steps may touch is decided by the service account's permissions,
not by Robine Dog: grant them only in the namespaces workflows may act on.
With silo01's `robine_dog` Ansible role, list those namespaces in
`robine_dog_workflow_namespaces`; when it is empty, the role mounts no
token and Kubernetes steps fail. Otherwise, create a Role and a RoleBinding
like these in each namespace:

```yaml
apiVersion: rbac.authorization.k8s.io/v1
kind: Role
metadata:
  name: robine-dog-workflows
  namespace: shop
rules:
  - apiGroups: ["apps"]
    resources: ["deployments", "statefulsets", "daemonsets"]
    verbs: ["get", "patch"]
  - apiGroups: ["apps"]
    resources: ["deployments/scale", "statefulsets/scale"]
    verbs: ["get", "patch"]
  - apiGroups: [""]
    resources: ["pods"]
    verbs: ["get", "delete"]
---
apiVersion: rbac.authorization.k8s.io/v1
kind: RoleBinding
metadata:
  name: robine-dog-workflows
  namespace: shop
roleRef:
  apiGroup: rbac.authorization.k8s.io
  kind: Role
  name: robine-dog-workflows
subjects:
  - kind: ServiceAccount
    name: robine-dog
    namespace: robine-dog
```

> **Warning:** a workflow can restart, scale or delete pods wherever the
> service account is allowed to. Anyone who can edit workflows can use
> those permissions.

## App Builder

An app is a form in front of workflows. On [App Builder](/apps), click
**+ New App** (it needs at least one workflow) and give it:

| Field | Description |
| --- | --- |
| Name | Required |
| Description | Shown under the app's title |
| Fields | Text fields, comma-separated: `namespace, deployment` |
| Buttons | A label and the workflow it runs. An app can have several buttons. |

Each app is a card with its fields and buttons. A click on a button starts
its workflow with `trigger_type` `app`; what was typed is in
`{{ inputs.values.<field> }}` (or `{{ trigger.values.<field> }}`), and the
app's name in `{{ inputs.app }}`. The run opens in a side panel, where you
can follow it and answer its approvals.

A "Restart a deployment" app, with fields `namespace, deployment`, can run
this workflow:

```json
[
  {"name": "confirm", "action": "approval",
   "params": {"message": "Restart {{ inputs.values.deployment }} in {{ inputs.values.namespace }}?"}},
  {"name": "restart", "action": "k8s.restart",
   "params": {"namespace": "{{ inputs.values.namespace }}", "kind": "deployment", "name": "{{ inputs.values.deployment }}"}}
]
```

## API

The endpoints accept the `DD-API-KEY` header. Bodies can be the plain
object or Datadog's `{"data": {"attributes": {…}}}` envelope.

| Method and path | Description |
| --- | --- |
| `GET /api/v2/workflows` | List workflows; `actions` lists the step actions |
| `POST /api/v2/workflows` | Create a workflow |
| `GET /api/v2/workflows/{id}` | Get a workflow |
| `PUT`, `PATCH /api/v2/workflows/{id}` | Replace a workflow (both replace the whole definition) |
| `DELETE /api/v2/workflows/{id}` | Delete a workflow |
| `POST /api/v2/workflows/{id}/instances` | Start a run: `{"meta": {"payload": {…}}}`. Returns `{"data": {"id": "<run id>"}}` |
| `GET /api/v2/workflows/{id}/instances` | The workflow's latest 200 runs |
| `GET /api/v2/workflows/{id}/instances/{run id}` | One run, with its steps |
| `GET`, `POST /api/v2/app-builder/apps` | List or create apps |
| `PATCH`, `DELETE /api/v2/app-builder/apps/{id}` | Replace or delete an app |
| `POST /api/v2/app-builder/apps/{id}/run` | Press a button: `{"button": 0, "values": {"namespace": "shop"}}` |

Create a workflow that posts monitor alerts to a webhook and opens a case
when the alert is about production:

```sh
curl -X POST http://localhost:8080/api/v2/workflows \
  -H "DD-API-KEY: $ROBINE_DOG_API_KEY" -H "Content-Type: application/json" \
  -d '{
    "name": "triage",
    "description": "Notify the team channel, open a case for prod",
    "enabled": true,
    "triggers": {"security_signal": false, "schedule": ""},
    "steps": [
      {"name": "notify", "action": "http",
       "params": {"method": "POST", "url": "https://hooks.slack.com/services/T000/B000/XXXX",
                  "body": "{\"text\": {{ trigger.title | json }}}"}},
      {"name": "case", "action": "case.create",
       "if": "{{ trigger.group }} contains env:prod",
       "params": {"title": "{{ trigger.title }}", "description": "{{ trigger.message }}", "priority": "P2", "project": "OPS"}}
    ]
  }'
```

Then add `@workflow-triage` to a monitor's message, or start it yourself:

```sh
curl -X POST http://localhost:8080/api/v2/workflows/<id>/instances \
  -H "DD-API-KEY: $ROBINE_DOG_API_KEY" -H "Content-Type: application/json" \
  -d '{"meta": {"payload": {"title": "Manual test", "message": "Testing the workflow", "group": "env:prod"}}}'
```

## Differences from Datadog

- There is no visual editor and no integration catalog: steps are the ten
  actions above, with JSON parameters. Any service with a webhook or an
  HTTP API is reached with an `http` step.
- Templates use `{{ trigger.* }}`, `{{ steps.<name>.* }}` and `| json`;
  Datadog's JavaScript expressions and functions are not supported.
- Steps run in sequence; there are no branches, loops or parallel steps.
  `if` skips a single step.
- Approvals are answered in the UI, not through Slack or Teams.
- Workflow and app bodies are flat (`name`, `steps`, `triggers`, `fields`,
  `buttons`), not Datadog's `spec` and app definition formats.
