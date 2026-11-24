---
title: Log Forwarding
section: Logs
order: 50
app: /logs/forwarding
description: Custom destinations send a copy of the logs matching a query to an HTTP endpoint, Splunk or Elasticsearch, in batches, after the pipelines.
---

# Log Forwarding

Log Forwarding sends a copy of your logs to other tools: a SIEM, a data
lake, a team's own Splunk or Elasticsearch. Each **custom destination** has a
search query and an endpoint; Robine Dog sends it every log the query
matches, in batches.

Forwarded logs are the processed logs: they have gone through the
[pipelines](log-pipelines.md) and the
[Sensitive Data Scanner](sensitive-data-scanner.md), so they are parsed,
enriched and redacted. Forwarding happens before the
[indexes](log-indexes.md), so logs that exclusion filters or quotas drop are
forwarded too.

Unlike [archives](log-archives.md), where a log goes to the first matching
archive only, a log is sent to **every** destination whose query matches it.

## Destination types

| Type | `type` | Request |
| --- | --- | --- |
| HTTP | `http` | `POST` to the endpoint, `Content-Type: application/json`, a JSON array of logs |
| Splunk HEC | `splunk_hec` | `POST` to `<endpoint>/services/collector/event` (the endpoint as is if it already contains `/services/collector`), `Authorization: Splunk <token>`, one HEC event per line |
| Elasticsearch | `elasticsearch` | `POST` to `<endpoint>/_bulk`, `Content-Type: application/x-ndjson`, an `index` action before each log |

The endpoint must be an `http://` or `https://` URL.

Each log is a JSON object in the [archive format](log-archives.md#how-logs-are-archived):
`date`, `host`, `service`, `source`, `status`, `message`, `tags`,
`trace_id`, `span_id` and `attributes`. For Splunk, that object is the
event, with `time` (seconds), `host`, `source` (the log's service) and
`sourcetype` (the log's source) set from it.

### Authentication

| Type | Fields |
| --- | --- |
| `http`, basic auth | `auth.type: "basic"`, `auth.username`, `auth.password` |
| `http`, a header | `auth.type: "custom_header"`, `auth.header_name` (default `Authorization`), `auth.header_value` |
| `splunk_hec` | `access_token`: the HEC token |
| `elasticsearch` | `auth.username`, `auth.password` (basic auth), optional |

### Elasticsearch indexes

`index_name` is the index the logs go to (default `datadog-logs`). With
`index_rotation`, Robine Dog appends the current UTC date to it in that
format, using `yyyy`, `MM`, `dd` and `HH`: `index_name` `logs` with
`index_rotation` `yyyy-MM-dd` writes to `logs-2026-11-20`.

## Destination settings

| Setting | API field | Description |
| --- | --- | --- |
| Name | `name` | Required. |
| Logs to forward | `query` | A [search query](search-syntax.md). Empty forwards every log. |
| Enabled | `enabled` | Default `true`. A disabled destination receives nothing. |
| Forward tags | `forward_tags` | Includes each log's `tags`. Default `true`. API only. |
| Destination | `forwarder_destination` | `type`, `endpoint` and the fields above. |

## Delivery

- Every 5 seconds, Robine Dog sends each destination what it has queued,
  in batches of up to 500 logs, with a 30-second timeout.
- A batch is delivered when the endpoint answers with a 2xx status. For
  Elasticsearch, a response containing `"errors":true` (some documents
  rejected) counts as a failure.
- A failed batch is **not retried**: its logs are counted as dropped, the
  error is recorded, and the destination's other batches wait for the next
  round.
- Each destination queues at most 20,000 logs. When an endpoint is slower
  than the incoming logs, the oldest queued logs are dropped.

> **Warning:** queued logs are kept in memory only. Logs not yet sent when
> Robine Dog stops are not forwarded.

## Manage destinations in the interface

[Log Forwarding](/logs/forwarding) lists the destinations:

| Column | Shows |
| --- | --- |
| Destination | The name and endpoint |
| Type | `http`, `splunk_hec` or `elasticsearch` |
| Query | The query (`*` when empty) |
| Sent | Logs delivered since Robine Dog started |
| Dropped | Logs dropped: failed batches and queue overflow |
| Status | `disabled`, **failing** (hover for the last error), `sent <time> ago`, or `waiting for logs` |

**New destination** opens the editor; click a row to edit it. For HTTP, a
username sets basic auth; without a username, the secret field is sent as
the `Authorization` header. Leave the secret empty when editing to keep the
stored one. The custom header name, `index_rotation` and `forward_tags` are
set through the API.

## API

The endpoints take Datadog's JSON:API bodies and the `DD-API-KEY` header
(see [API and application keys](api-keys.md)).

| Method | Endpoint | Description |
| --- | --- | --- |
| `GET` | `/api/v2/logs/config/custom-destinations` | Every destination |
| `GET` | `/api/v2/logs/config/custom-destinations/{id}` | One destination |
| `POST` | `/api/v2/logs/config/custom-destinations` | Creates a destination |
| `PATCH` | `/api/v2/logs/config/custom-destinations/{id}` | Updates a destination; attributes you leave out keep their value |
| `DELETE` | `/api/v2/logs/config/custom-destinations/{id}` | Deletes a destination |

Forward the security team's logs to Splunk:

```sh
curl -X POST http://localhost:8080/api/v2/logs/config/custom-destinations \
  -H "DD-API-KEY: $ROBINE_DOG_API_KEY" \
  -H "DD-APPLICATION-KEY: $ROBINE_DOG_APP_KEY" \
  -H "Content-Type: application/json" \
  -d '{"data": {"type": "custom_destination", "attributes": {
        "name": "Splunk security",
        "query": "team:security OR source:appsec",
        "enabled": true,
        "forwarder_destination": {
          "type": "splunk_hec",
          "endpoint": "https://splunk.example.com:8088",
          "access_token": "00000000-0000-0000-0000-000000000000"
        }
      }}}'
```

Send the error logs to an Elasticsearch cluster, one index per day:

```sh
curl -X POST http://localhost:8080/api/v2/logs/config/custom-destinations \
  -H "DD-API-KEY: $ROBINE_DOG_API_KEY" \
  -H "DD-APPLICATION-KEY: $ROBINE_DOG_APP_KEY" \
  -H "Content-Type: application/json" \
  -d '{"data": {"type": "custom_destination", "attributes": {
        "name": "Elasticsearch errors",
        "query": "status:error",
        "forwarder_destination": {
          "type": "elasticsearch",
          "endpoint": "http://elasticsearch:9200",
          "index_name": "robine-logs",
          "index_rotation": "yyyy-MM-dd",
          "auth": {"username": "elastic", "password": "changeme"}
        }
      }}}'
```

### Secrets

Responses never include `access_token`, `auth.password` or
`auth.header_value`. A `PATCH` that leaves them out keeps the stored values,
so you can change a destination's query without sending its secrets again.

Each destination's `attributes` also carry a `status` object: `sent`,
`dropped`, `last_error` and `last_sent` (epoch milliseconds). These counters
are kept in memory and start from zero after a restart.

A missing name, an unknown type, an endpoint that is not an HTTP(S) URL or
an invalid query returns `400`.

## Differences from Datadog

- The destination types are `http`, `splunk_hec` and `elasticsearch`; other
  types are refused.
- Failed batches are dropped rather than retried, and queued logs do not
  survive a restart.
