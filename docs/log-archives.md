---
title: Archives and rehydration
section: Logs
order: 40
app: /logs/archives
description: Archives write every ingested log to an S3-compatible bucket or a local directory in Datadog's archive format. Rehydration loads a period of an archive back into a historical view you can search.
---

# Archives and rehydration

Searchable logs are kept for days; archives keep them for as long as your
bucket does. An **archive** writes the logs Robine Dog ingests, as gzipped
JSON lines in Datadog's layout, to an S3-compatible bucket or a directory on
the server. When you need old logs again, **rehydration** reads a period of
an archive back into a **historical view**, a separate index you search with
`index:<view>`.

Archives see every log, including the logs that [index](log-indexes.md)
exclusion filters and quotas drop. They receive the logs after the
[pipelines](log-pipelines.md) and the
[Sensitive Data Scanner](sensitive-data-scanner.md), so archived logs are
already parsed and redacted.

## How logs are archived

Archives are ordered. Each log goes to the **first archive whose query
matches it**, and to no other. A log that matches no archive is not
archived.

Matching logs are buffered in memory and written:

- every `ROBINE_DOG_ARCHIVE_FLUSH_SECONDS` seconds (default `300`);
- earlier, when an archive's buffer reaches 64 MiB;
- when Robine Dog stops on `SIGTERM`, and before an archive is changed or
  deleted.

Each write creates one file per archive and per hour of log timestamps:

```text
<path>/dt=20261120/hour=14/archive_143502.1230.<id>.json.gz
```

The `dt=YYYYMMDD/hour=HH` directories are the UTC date and hour of the logs
they contain; the file name carries the time of the write. This is the
layout Datadog uses.

Each line is one log:

```json
{"_id":"…","date":"2026-11-20T14:31:07.412Z","host":"web-1","service":"shop-api","source":"python","status":"error","message":"payment declined","tags":["env:prod","team:pay"],"trace_id":"5827214587453260137","attributes":{"http":{"status_code":502}}}
```

`tags` is left out when the archive's **Include tags** option is off.
`index:` tags are never written.

> **Warning:** when a write fails, the logs of that file are not retried.
> The archive shows `FAILING` and the error; fix the destination quickly.

## Destinations

| Type | Fields | Notes |
| --- | --- | --- |
| `s3` | `bucket` (required), `path`, `region`, `endpoint` | Amazon S3 or any S3-compatible store (MinIO, Garage, R2…). `path` is a key prefix. |
| `local` | `path` | An absolute directory on the Robine Dog server, created if it does not exist. |

For S3, credentials come from the server's environment:
`AWS_ACCESS_KEY_ID` and `AWS_SECRET_ACCESS_KEY`, or the instance's role. Set
`endpoint` for a store other than AWS; Robine Dog then uses path-style
requests, and plain `http://` endpoints are allowed. Requests are retried
twice, for at most 20 seconds.

When you create or change an archive, Robine Dog checks the destination by
writing a small `.robine-dog-check` object to it. If that fails, the archive
is not saved and the error is returned.

## Archive settings

| Setting | API field | Description |
| --- | --- | --- |
| Name | `name` | Required. |
| Archive logs matching | `query` | A [search query](search-syntax.md). Empty archives every log. |
| Destination | `destination` | `type`, then the fields of the table above. |
| Include tags | `include_tags` | Writes each log's tags. Default `true`. |
| Tags added to rehydrated logs | `rehydration_tags` | Tags added to every log rehydrated from this archive, such as `rehydrated:true`. |
| Maximum scan size | `rehydration_max_scan_size_in_gb` | API only. A rehydration stops once it has read this many GiB of files. |

## Manage archives in the interface

[Archives](/logs/archives) lists the archives in order:

| Column | Shows |
| --- | --- |
| Order | Arrows to move the archive up or down. |
| Archive | The name, and the last write error if any. |
| Filter | The archive query. |
| Destination | `s3://bucket/path` with its endpoint, or the local directory. |
| Written | Logs archived, files and bytes written, and the time of the last write, since Robine Dog started. |

**New Archive** opens the editor; a new archive is added last. Deleting an
archive writes what it has buffered, then stops archiving; the files already
written stay in the destination.

## Rehydrate archived logs

Click **Rehydrate** on the Archives page and fill in:

| Field | Description |
| --- | --- |
| Historical view name | Letters, digits, `-` and `_`, unique. You search the view with `index:<name>`. |
| Archive | The archive to read. |
| Logs matching | A search query; only matching logs are loaded. Empty loads everything in the period. |
| From, To | The period to read, at most 31 days. |
| Max logs | Stops after this many logs. Default 10,000,000. |

Robine Dog reads every `.json.gz` file of the period's hourly directories,
keeps the logs within the period that match the query, and stores them in
a table of their own. Each rehydrated log gets the tag `index:<view>` and
the archive's rehydration tags. Any file in this layout and line format can
be read, not only the files Robine Dog wrote.

The **Historical views** table shows each view's archive, query, period,
status (`running`, `done` or `failed`, with the error) and progress: files
and bytes scanned, logs read and logs rehydrated. It refreshes every two
seconds while a view is running. Click a view's name to open the
[Log Explorer](log-explorer.md) on `index:<view>` over the view's period.

```text
index:incident-2026-11-20 service:shop-api status:error
```

A query that names a historical view searches only that view, not the
current logs. Historical views are not subject to the logs' retention: they
stay until you delete them, which drops their logs. A view cannot be deleted
while it is running.

> **Note:** a rehydration interrupted by a restart is marked `failed`. Delete
> the view and start it again.

## API

Archives use Datadog's API and JSON:API bodies, with the `DD-API-KEY`
header (see [API and application keys](api-keys.md)).

| Method | Endpoint | Description |
| --- | --- | --- |
| `GET` | `/api/v2/logs/config/archives` | Every archive, in order |
| `GET` | `/api/v2/logs/config/archives/{id}` | One archive |
| `POST` | `/api/v2/logs/config/archives` | Creates an archive, last in the order |
| `PUT` | `/api/v2/logs/config/archives/{id}` | Replaces an archive's settings |
| `DELETE` | `/api/v2/logs/config/archives/{id}` | Deletes an archive; its files stay |
| `GET` | `/api/v2/logs/config/archive-order` | `data.attributes.archive_ids` |
| `PUT` | `/api/v2/logs/config/archive-order` | Sets the order; the list must name every archive once |

Archive the production logs to a Garage bucket:

```sh
curl -X POST http://localhost:8080/api/v2/logs/config/archives \
  -H "DD-API-KEY: $ROBINE_DOG_API_KEY" \
  -H "DD-APPLICATION-KEY: $ROBINE_DOG_APP_KEY" \
  -H "Content-Type: application/json" \
  -d '{"data": {"type": "archives", "attributes": {
        "name": "prod",
        "query": "env:prod",
        "destination": {"type": "s3", "bucket": "logs-archive", "path": "robine-dog",
                        "region": "garage", "endpoint": "http://garage:3900"},
        "include_tags": true,
        "rehydration_tags": ["rehydrated:true"]
      }}}'
```

Each archive's `attributes` also carry `state` (`WORKING` or `FAILING`) and
a `status` object: `archived`, `files`, `bytes`, `last_flush` (epoch
milliseconds) and `last_error`. These counters are kept in memory and start
from zero after a restart.

Rehydration has no endpoint under `/api/v2`. The interface uses
`/api/ui/logs/rehydrations` (`GET` to list views, `POST` with `name`,
`archive_id`, `query`, `from`, `to` in epoch milliseconds and `max_logs`)
and `DELETE /api/ui/logs/rehydrations/{id}`.

## Differences from Datadog

- Only `s3` (and S3-compatible) and `local` destinations exist; `gcs` and
  `azure` are refused.
- S3 credentials come from the server's environment. Datadog's
  `integration` (AWS account and role) is accepted and stored, but unused.
- Rehydration reads at most 31 days at once and is managed from the
  interface only.
