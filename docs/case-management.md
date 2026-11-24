---
title: Case Management
section: Service Management
order: 40
app: /cases
description: Cases track a problem to its resolution, with a status, a priority, an assignee, a project key such as SHOP-12 and a timeline of comments and changes.
---

# Case Management

A case is a unit of follow-up work: something that is not an outage worth
an [incident](incidents.md), but that someone has to look at and close. A
case has a status, a priority, an assignee and a timeline where every
change and comment is written down with its author.

You open cases by hand in [Case Management](/cases), from a
[security signal](siem.md), from a [workflow](workflows.md), or through
the API, which follows Datadog's `/api/v2/cases`.

## Case fields

| Field | Values | Description |
| --- | --- | --- |
| `title` | Text | Required. |
| `description` | Text | Optional. |
| `status` | `OPEN`, `IN_PROGRESS`, `CLOSED` | A new case is `OPEN`. Closing a case records `closed_at`. |
| `priority` | `NOT_DEFINED`, `P1`, `P2`, `P3`, `P4`, `P5` | Default `NOT_DEFINED` (the UI's form proposes `P3`). Any other value is refused. |
| `project` | Text | Default `CASE`. Stored in upper case. |
| `key` | `<PROJECT>-<number>` | Set at creation, such as `SHOP-12`. |
| `assignee` | Text | Who handles the case, usually an e-mail address. Empty: unassigned. |
| `type` | Text | Default `STANDARD`. |
| `origin` | `{"type", "id", "url"}` | Where the case comes from. The UI links to `url`. |
| `created_at`, `created_by`, `modified_at` | | Set by Robine Dog. `created_by` is the person signed in, or `api`. |
| `timeline` | List | Every change and comment, with `ts`, `author`, `type` and `text`. |

The number in a key is one sequence shared by every project: if the last
case is `CASE-11`, the next case of project `SHOP` is `SHOP-12`.

## Working with cases in the UI

[Case Management](/cases) lists the cases, newest first: **Key**,
**Title**, **Status**, **Priority**, **Assignee** and **Updated**. The
status menu above the table keeps the cases of one status.

- **+ New Case** asks for a title, a description, a priority and a project,
  then opens the new case.
- Clicking a row opens the case in a side panel. Change its status or
  priority from the menus; type an assignee and click **Assign** (an empty
  field unassigns). Each change is added to the timeline.
- The **Timeline** shows every entry with its time and author. Type in
  **Add a comment** and click **Comment** to add one.
- When the case has an origin, the panel shows *From security signal:
  <id>* with a link back to it.

The URL keeps the open case and the status filter
(`/cases?case=<id>&status=OPEN`), so you can share it.

## Opening a case from a security signal

On a [security signal](siem.md)'s panel, **Create case** opens a case and
takes you to it:

| Field | Value |
| --- | --- |
| Title | `<rule name>: <group>` |
| Description | The signal's message |
| Priority | `P2` for `critical` and `high` signals, `P3` otherwise |
| Project | `SEC` |
| Origin | `security_signal`, the signal's id, a link to the signal |

## Opening a case from a workflow

A [workflow](workflows.md)'s `case.create` step takes the same attributes
as the API and outputs the case's `id` and `key`. Combined with a monitor's
`@workflow-<name>` handle, this opens a case when a monitor alerts:

```json
{
  "name": "open-case",
  "action": "case.create",
  "if": "{{ trigger.status }} == Alert",
  "params": {
    "title": "{{ trigger.title }}",
    "description": "{{ trigger.message }}",
    "priority": "P2",
    "project": "OPS"
  }
}
```

## API

The endpoints accept the `DD-API-KEY` header. Bodies use Datadog's
`{"data": {"attributes": {…}}}` envelope; for creation, a plain object is
accepted too.

| Method and path | Description |
| --- | --- |
| `GET /api/v2/cases` | List cases, highest number first. `filter[status]=OPEN` keeps one status; `filter[query]=checkout` keeps the cases whose fields contain the text (case-insensitive). The response has `meta.page.total`. |
| `POST /api/v2/cases` | Create a case |
| `GET /api/v2/cases/{id}` | Get one case |
| `POST /api/v2/cases/{id}/status` | `{"status": "IN_PROGRESS"}` |
| `POST /api/v2/cases/{id}/priority` | `{"priority": "P1"}` |
| `POST /api/v2/cases/{id}/assign` | `{"assignee_id": "alice@example.com"}` (`assignee` is accepted too) |
| `POST /api/v2/cases/{id}/unassign` | No attribute |
| `POST /api/v2/cases/{id}/comment` | `{"comment": "Rolled back to 3.1.4."}` |
| `POST /api/v2/cases/{id}/archive` | Marks the case `archived: true` |

Each action returns the updated case and adds a timeline entry.

Create a case, then move it forward:

```sh
curl -X POST http://localhost:8080/api/v2/cases \
  -H "DD-API-KEY: $ROBINE_DOG_API_KEY" -H "Content-Type: application/json" \
  -d '{"data": {"type": "case", "attributes": {"title": "Slow checkout on mobile", "priority": "P3", "project": "SHOP"}}}'

curl -X POST http://localhost:8080/api/v2/cases/<id>/status \
  -H "DD-API-KEY: $ROBINE_DOG_API_KEY" -H "Content-Type: application/json" \
  -d '{"data": {"attributes": {"status": "IN_PROGRESS"}}}'

curl -X POST http://localhost:8080/api/v2/cases/<id>/comment \
  -H "DD-API-KEY: $ROBINE_DOG_API_KEY" -H "Content-Type: application/json" \
  -d '{"data": {"attributes": {"comment": "Reproduced on Android, looking at the RUM sessions."}}}'
```

A case's response looks like this:

```json
{
  "data": {
    "id": "6f1c…",
    "type": "case",
    "attributes": {
      "number": 12, "key": "SHOP-12", "title": "Slow checkout on mobile",
      "status": "IN_PROGRESS", "priority": "P3", "project": "SHOP",
      "type": "STANDARD", "assignee": null, "created_by": "api",
      "timeline": [
        {"ts": 1764000000000, "author": "api", "type": "created", "text": "Case created"},
        {"ts": 1764000060000, "author": "api", "type": "status", "text": "Status: OPEN → IN_PROGRESS"}
      ]
    }
  }
}
```

## Differences from Datadog

- Projects are not objects of their own: `project` is a free text that
  gives the key its prefix, and numbers are shared by all projects.
- A case's title and description cannot be changed after creation, and
  cases cannot be deleted. Archived cases stay in the list.
- The assignee is free text, not a user id.
- Cases are not opened automatically from monitors or Error Tracking issues;
  use a [workflow](workflows.md) for monitors.
- There are no custom attributes, no case types other than a label, and no
  notifications on changes.
