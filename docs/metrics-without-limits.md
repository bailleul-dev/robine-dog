---
title: Metrics without Limits
section: Metrics
order: 20
app: /metrics/tags
description: Choose which tag keys a metric keeps when its points are written. The other tags are aggregated away, which bounds the number of series the metric stores.
---

# Metrics without Limits

Every distinct combination of host and tags is a series. A metric tagged
with `pod`, `container_id` or `user_id` can produce thousands of series,
most of which no query ever groups by. Metrics without Limits lets you keep
only the tag keys you query, per metric. Points are rewritten when they are
stored: the tags you drop are removed, and the series that become identical
are merged into one.

Use the [Volume](/metrics/volume) page to find the metrics with the most
series, then configure them here.

## How tags are aggregated away

A tag configuration names a metric, its type and a list of tag keys:

| Mode | Keeps |
| --- | --- |
| Include (default) | Only the listed tag keys |
| Exclude (`exclude_tags_mode: true`) | Every tag key except the listed ones |

`host` counts as a tag key: in include mode, list `host` to keep the host
of each series; otherwise the host is dropped too.

After the tags are reduced, points with the same timestamp, host and
remaining tags are merged according to the type you configure:

| `metric_type` | Merged points |
| --- | --- |
| `count`, `rate` | Summed |
| `gauge` | Averaged |
| `distribution` | Not merged: every sample is kept with its weight, without the dropped tags, so percentiles stay exact |

For example, with `req.count` configured as a `count` keeping only `env`,
these three points of one 10-second bucket:

```text
req.count  host:h  env:prod,pod:a  1
req.count  host:h  env:prod,pod:b  2
req.count  host:h  env:prod,pod:c  3
```

are stored as one:

```text
req.count  env:prod  6
```

`sum:req.count{env:prod}` returns the same value as before; `by {pod}` no
longer splits it.

> **Note:** the type in the configuration decides how points merge, not the
> type the metric reports. Set it to the metric's real type, as shown in the
> [Summary](/metrics/summary). It defaults to `gauge`.

The configuration applies to every source of points: DogStatsD and the
metrics Robine Dog derives (trace, log-, span- and RUM-based metrics),
`/api/v1/series` and `/api/v2/series`, the Agent's series and sketches, and
OTLP metrics.
Points are merged within one write: one flush of the 10-second aggregator,
or one series request.

> **Warning:** dropped tags are not stored. Points written while a
> configuration is active cannot be split by those tags later, even after you
> remove it. Points written before keep all their tags.

## Configure a metric in the app

[Metrics without Limits](/metrics/tags) lists the configured metrics with
their type, the tags kept ("only …") or dropped ("all but …"), and the
series ingested and stored in the current hour.

1. Click **Configure a metric**.
2. Enter the metric name and choose its **Type**.
3. Enter the **Tag keys**, separated by commas or spaces (`env, service`).
4. Check **Drop these tags instead of keeping only them** for exclude mode.
5. Click **Save**. The configuration applies to the next points written.

Click a row to edit it. The bin icon removes the configuration: the metric
keeps all its tags again.

## API

The routes take `DD-API-KEY`, like Datadog's.

| Endpoint | What it does |
| --- | --- |
| `GET /api/v2/metrics/{name}/tags` | The metric's tag configuration |
| `POST /api/v2/metrics/{name}/tags` | Creates or replaces it |
| `PATCH /api/v2/metrics/{name}/tags` | Updates it; attributes left out keep their value |
| `DELETE /api/v2/metrics/{name}/tags` | Removes it |
| `GET /api/v2/metrics/{name}/volumes` | Distinct series this hour, before and after the configuration |

| Attribute | Meaning |
| --- | --- |
| `tags` | The tag keys (required) |
| `exclude_tags_mode` | `true` to drop the listed keys instead of keeping them; `false` by default |
| `metric_type` | `gauge` (default), `count`, `rate` or `distribution` |

```sh
curl -X POST http://localhost:8080/api/v2/metrics/shop.checkout.requests/tags \
  -H "DD-API-KEY: $ROBINE_DOG_API_KEY" \
  -H "DD-APPLICATION-KEY: $ROBINE_DOG_APP_KEY" \
  -H "Content-Type: application/json" \
  -d '{"data": {"type": "manage_tags", "id": "shop.checkout.requests",
       "attributes": {"tags": ["env", "service", "status"], "metric_type": "count"}}}'
```

```sh
curl http://localhost:8080/api/v2/metrics/shop.checkout.requests/volumes \
  -H "DD-API-KEY: $ROBINE_DOG_API_KEY"
```

```json
{
  "data": {
    "type": "metric_volumes",
    "id": "shop.checkout.requests",
    "attributes": { "ingested_volume": 1840, "indexed_volume": 12 }
  }
}
```

`ingested_volume` is the number of distinct series received this hour,
`indexed_volume` the number stored after the tags are reduced.

## Differences from Datadog

- Volumes are counted for configured metrics only, over the current hour.
  They are held in memory: they restart from zero every hour and when Robine
  Dog restarts.
- `POST` and `PATCH` both create or update a configuration.
- The configuration has no `aggregations` or `include_percentiles`
  attributes: counts and rates are always summed and gauges averaged, and
  distributions keep their samples, so every percentile stays queryable.
