---
title: Audit Trail
section: Security
order: 50
app: /audit
description: Every change made through the UI or the API, with who made it and whether it succeeded, kept 90 days.
---

# Audit Trail

The Audit Trail answers "who changed this, and when?". Robine Dog records
every request that changes something, from the UI or from the API: a
monitor created, a pipeline edited, a dashboard deleted, a signal archived,
an API key revoked. Browse it in [Audit Trail](/audit), or read it with the
API.

## What is recorded

A request is recorded when it changes something:

| Method | Recorded |
| --- | --- |
| `PUT`, `PATCH`, `DELETE` | Always |
| `POST` | Unless the last segment of its path marks a read, such as `search`, `aggregate`, `timeseries`, `scalar`, `query`, `histogram`, `values`, `validate`, `preview`, `patterns`, `funnel`, `test` or `trigger` |
| `GET` | Never |

Requests to the UI's API (`/api/ui/…`, after sign-in) and to the Datadog
API (`/api/v1/…`, `/api/v2/…` routes that take an API key) are recorded.
Data intakes are not: sending logs, metrics, traces, RUM events, test
results or deployments does not write to the Audit Trail.

Each event holds:

| Field | Value |
| --- | --- |
| Date | When the request was made |
| User | The signed-in user's e-mail (or OIDC subject) and name; `api-key` for a request authenticated by an API key |
| Source | `ui` or `api` |
| Request | Method and path, such as `DELETE /api/v1/monitor/42` |
| Status | The HTTP status of the answer: a change that failed is recorded too, with its 4xx or 5xx status |

The request body is not recorded: the Audit Trail says what was changed,
not the content of the change.

## Retention

Events are kept **90 days**. Older events are removed by an hourly purge.

## Search the Audit Trail

[Audit Trail](/audit) shows the events of the selected time range (the
last day by default), newest first, up to 500. Type in the filter box and
press Enter to keep the events whose user, method or path contains the
text, case-insensitively:

| Filter | Finds |
| --- | --- |
| `alice@example.com` | Alice's changes |
| `DELETE` | Every deletion |
| `/monitor` | Changes to monitors, through the UI or the API |
| `api-key` | Changes made with an API key |

## API

`GET /api/v2/audit/events` returns the same events, with `DD-API-KEY`:

| Parameter | Meaning |
| --- | --- |
| `from`, `to` (or `filter[from]`, `filter[to]`) | Epoch milliseconds; the last 7 days by default |
| `query` (or `filter[query]`) | Text contained in the user, method or path |

```sh
curl -G http://localhost:8080/api/v2/audit/events \
  -H "DD-API-KEY: $ROBINE_DOG_API_KEY" \
  -H "DD-APPLICATION-KEY: $ROBINE_DOG_APP_KEY" \
  --data-urlencode "query=DELETE"
```

```json
{
  "data": [
    {
      "id": "1842",
      "type": "audit",
      "attributes": {
        "timestamp": 1764000000000,
        "service": "robine-dog",
        "message": "DELETE /api/v1/monitor/42",
        "attributes": {
          "evt": {"name": "Request", "source": "api"},
          "http": {"method": "DELETE", "url_details": {"path": "/api/v1/monitor/42"}, "status_code": 200},
          "usr": {"id": "api-key", "name": null}
        }
      }
    }
  ]
}
```

At most 500 events are returned, newest first.

## Differences from Datadog

- Events are HTTP requests (method, path, status), not typed events with the changed asset and its before/after values.
- The search is a substring match on user, method and path, not the [search syntax](search-syntax.md); there are no facets, analytics or saved views.
- Requests made with an API key are attributed to `api-key`, not to the key's name or owner.
- The retention of 90 days is fixed; archiving the Audit Trail is not implemented.
