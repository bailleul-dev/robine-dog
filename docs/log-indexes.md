---
title: Indexes
section: Logs
order: 30
app: /logs/indexes
description: Indexes decide which logs are stored and searchable, for how long, and how many per day. Exclusion filters and daily quotas keep noisy logs out without losing them for metrics and archives.
---

# Indexes

Every log that comes out of its [pipelines](log-pipelines.md) is routed to an
**index**. The index decides whether the log is stored, and for how long it
stays searchable. Indexes let you drop debug noise, sample chatty services,
cap a team's daily volume and keep some logs for less time than others,
without changing what your applications send.

Robine Dog's indexes work like Datadog's and use the same API, so Terraform
and the Datadog API clients can manage them.

## What happens to a log

After the pipelines, each log goes through these steps, in this order:

1. The [Sensitive Data Scanner](sensitive-data-scanner.md) redacts it.
2. [Log-based metrics](generate-metrics.md) count it.
3. [Custom destinations](log-forwarding.md) that match it queue it.
4. The first [archive](log-archives.md) whose query matches it buffers it.
5. The indexes decide whether it is stored.

Only the last step drops anything. A log excluded from the indexes is still
counted by log-based metrics, forwarded and archived, as in Datadog.

## How a log is routed

Indexes are ordered. A log goes to the **first index whose filter matches
it**; the following indexes never see it. Then, in that index:

1. If an enabled **exclusion filter** matches it, the first such filter
   decides: the log is dropped with the filter's probability (all of the
   matching logs at 100%).
2. If the index has a **daily quota** and has already stored that many logs
   since its last reset, the log is dropped.
3. Otherwise the log is stored, with the tag `index:<name>`.

A log that matches no index filter is not stored.

On a first start, Robine Dog creates one index, `main`, with an empty filter
(every log) and the retention of the logs table. If you delete every index,
every log is stored again, without an `index:` tag.

## Index settings

| Setting | API field | Description |
| --- | --- | --- |
| Name | `name` | Letters, digits, `-` and `_`. It cannot be changed after creation. |
| Filter | `filter.query` | A [search query](search-syntax.md). Empty matches every log. |
| Exclusion filters | `exclusion_filters` | Ordered list of `name`, `is_enabled` and `filter` (`query`, `sample_rate`). |
| Daily quota | `daily_limit` | The number of logs stored per day. Empty: no limit. |
| Reset at | `daily_limit_reset.reset_time` | When the quota resets, `HH:MM`. Default `00:00`. |
| UTC offset | `daily_limit_reset.reset_utc_offset` | The time zone of the reset time, `+HH:MM` or `-HH:MM`. Default `+00:00`. |
| Warn at % | `daily_limit_warning_threshold_percentage` | Stored for compatibility; see [Differences from Datadog](#differences-from-datadog). |
| Retention (days) | `num_retention_days` | How long the index's logs stay searchable. Default: the logs table's retention. |

### Exclusion filters

An exclusion filter has a query and a `sample_rate` between `0` and `1`: the
share of matching logs that is **dropped**. `1` (100% in the interface)
drops all of them, `0.9` keeps one log in ten, `0` drops nothing. Without a
`sample_rate`, the filter drops everything it matches. Sampling is random per
log.

Only the first enabled exclusion filter that matches a log applies. Put the
most specific filters first. Clear `is_enabled` to stop a filter without
deleting it.

### Daily quotas

The quota counts the logs the index stored since its last reset time, in the
time zone of its UTC offset. With `reset_time` `14:00` and
`reset_utc_offset` `+02:00`, the window starts every day at 12:00 UTC. Once
the count reaches `daily_limit`, the index drops the logs it receives until
the next reset, and the API reports `is_rate_limited: true`.

After a restart, Robine Dog counts the logs already stored in the current
window again, so a quota is not reset by restarting.

### Retention

`num_retention_days` hides an index's logs older than that many days from
every search, graph and monitor. It can only shorten retention: the logs
table itself keeps logs for `ROBINE_DOG_RETENTION_DAYS` (15 by default, see
[Configuration](configuration.md)), and a longer index retention has no
effect.

## Manage indexes in the interface

[Indexes](/logs/indexes) lists the indexes in their routing order:

| Column | Shows |
| --- | --- |
| Order | Arrows to move the index up or down. Order decides which index a log goes to. |
| Index | The name, linking to the [Log Explorer](log-explorer.md) filtered on `index:<name>`. A **quota reached** badge appears when the daily quota is used up. |
| Filter | The index filter (`*` when empty). |
| Exclusions | Each exclusion filter with its percentage, `off` when disabled. |
| Retention | The index retention, or `default`. |
| Today | Logs stored since the last reset (against the quota, with a bar), logs excluded and logs dropped over the quota. |

**New Index** opens the editor; a new index is added last. **Edit** changes
an index's filter, exclusion filters, quota and retention. The page refreshes
every 15 seconds.

## Search an index

Stored logs carry the tag `index:<name>`, so you can restrict any search to
one index:

```text
index:payments status:error
```

The same term with a [historical view](log-archives.md#rehydrate-archived-logs)
name searches the logs rehydrated from an archive.

## API

The endpoints take Datadog's JSON and the `DD-API-KEY` header (see
[API and application keys](api-keys.md)).

| Method | Endpoint | Description |
| --- | --- | --- |
| `GET` | `/api/v1/logs/config/indexes` | Every index, in order: `{"indexes": [...]}` |
| `GET` | `/api/v1/logs/config/indexes/{name}` | One index |
| `POST` | `/api/v1/logs/config/indexes` | Creates an index, last in the order |
| `PUT` | `/api/v1/logs/config/indexes/{name}` | Replaces an index's settings |
| `DELETE` | `/api/v1/logs/config/indexes/{name}` | Deletes an index |
| `GET` | `/api/v1/logs/config/index-order` | `{"index_names": [...]}` |
| `PUT` | `/api/v1/logs/config/index-order` | Sets the order; the list must name every index once |

Create an index for a payment service that drops its debug logs, keeps 10% of
its health checks and stores at most 1,000,000 logs a day:

```sh
curl -X POST http://localhost:8080/api/v1/logs/config/indexes \
  -H "DD-API-KEY: $ROBINE_DOG_API_KEY" \
  -H "DD-APPLICATION-KEY: $ROBINE_DOG_APP_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "name": "payments",
    "filter": {"query": "service:payment-service"},
    "exclusion_filters": [
      {"name": "debug", "is_enabled": true,
       "filter": {"query": "status:debug", "sample_rate": 1.0}},
      {"name": "health checks", "is_enabled": true,
       "filter": {"query": "@http.url_details.path:/health", "sample_rate": 0.9}}
    ],
    "daily_limit": 1000000,
    "daily_limit_reset": {"reset_time": "00:00", "reset_utc_offset": "+01:00"},
    "num_retention_days": 7
  }'
```

Because a new index comes last, move it before `main` so that it receives
the payment logs:

```sh
curl -X PUT http://localhost:8080/api/v1/logs/config/index-order \
  -H "DD-API-KEY: $ROBINE_DOG_API_KEY" \
  -H "DD-APPLICATION-KEY: $ROBINE_DOG_APP_KEY" \
  -H "Content-Type: application/json" \
  -d '{"index_names": ["payments", "main"]}'
```

Responses add `is_rate_limited` and a `usage` object for the current quota
window: `window_start` (epoch milliseconds), `indexed`, `excluded` and
`over_quota`. An invalid query, name, sample rate or reset time returns
`400` with an `error` message.

## Differences from Datadog

- Retention does not delete logs: an index's shorter retention hides its
  older logs from queries, and the logs table's TTL deletes them.
- `daily_limit_warning_threshold_percentage` is stored and returned, but no
  warning is sent when it is reached.
- `num_flex_logs_retention_days` is accepted and stored; there are no Flex
  Logs.
- The `excluded` and `over_quota` counters are kept in memory and start
  from zero after a restart; `indexed` is counted again from the stored logs.
