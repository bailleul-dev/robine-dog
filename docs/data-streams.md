---
title: Data Streams Monitoring
section: APM
order: 80
app: /data-streams
description: Throughput, latency and Kafka consumer lag of the queues between your services, from the pathway stats the tracers send.
---

# Data Streams Monitoring

When services talk through queues (Kafka, RabbitMQ, SQS…), a trace usually
stops at the queue: the consumer's work starts a new trace later. Data
Streams Monitoring follows messages across queues instead. The tracers tag
every message they produce with a pathway hash, measure the time each hop
takes, and send aggregated **pathway stats** to the Agent. Robine Dog
turns them into throughput and latency per service and queue, and into the
lag of each Kafka consumer group.

## Setup

Turn on Data Streams Monitoring in the tracer of every producer and consumer:

```sh
DD_DATA_STREAMS_ENABLED=true \
DD_SERVICE=order-worker DD_ENV=prod \
DD_AGENT_HOST=localhost DD_TRACE_AGENT_PORT=8126 \
ddtrace-run python worker.py
```

The tracer sends its stats to the trace agent. Robine Dog accepts them on
both paths:

| Endpoint | Sent by |
| --- | --- |
| `POST /v0.1/pipeline_stats` on the trace port (`localhost:8126`) | A tracer pointed at Robine Dog |
| `POST /api/v0.1/pipeline_stats` on port 8080, with `DD-API-KEY` | A Datadog Agent forwarding the tracers' stats |

Payloads are msgpack, optionally gzip-compressed, with DDSketch latencies, as
the tracers send them.

## What is collected

Each stats point describes one edge of a pathway over one of the tracer's time buckets:

| From the payload | Used as |
| --- | --- |
| `Service`, `Env` | The service reporting the point |
| `EdgeTags` | The edge: `direction:in` or `direction:out`, `topic:` (or `exchange:`), `type:` (`kafka`, `rabbitmq`…), `group:` |
| `EdgeLatency` | Time from the previous hop to this one; its p50 and p99 |
| `PathwayLatency` | Time since the message's pathway started; its p50 and p99 (end-to-end latency) |
| Sketch count | The number of messages (throughput) |
| `Backlogs` | Kafka offsets: `type:kafka_produce` (latest produced) and `type:kafka_commit` (latest committed, by `consumer_group`), per topic and partition |

Points with an `origin` timestamp type are ignored, as in Datadog, since
they would count messages twice.

## The Data Streams page

[Data Streams Monitoring](/data-streams) opens on the last hour and
refreshes every 30 seconds.

### Edges

One row per service and edge, the busiest first (500 at most):

| Column | Meaning |
| --- | --- |
| Service | The service producing or consuming |
| Direction | **produces to →** (`direction:out`) or **← consumes from** (`direction:in`) |
| Queue | The topic or exchange, with the queue type |
| Consumer group | The `group` tag of consumers |
| Throughput | Messages per second over the range |
| Edge latency p50 / p99 | How long the hop took: for a consumer, from production to consumption |
| End-to-end p50 / p99 | How long since the pathway started, at this point |

Latencies are averaged (p50) or maxed (p99) over the buckets of the range.

### Consumer lag

**Consumer lag (Kafka offsets)** lists, per consumer group and topic, the
number of messages produced and not yet committed: for each partition, the
latest produced offset minus the group's latest committed offset in the
range, summed over the partitions. Lag is only shown when the tracers report
both produce and commit offsets (Kafka integrations that support it).

## Storage

Stats points and offsets are stored in PostgreSQL (tables `dsm_points` and
`dsm_backlogs`). Robine Dog does not delete them automatically.

## Differences from Datadog

- There is no pathway map: edges are listed in a table, and pathways are not
  reassembled from their hashes.
- Data Streams metrics (`data_streams.*`) are not produced, so there are no
  Data Streams monitors; alert on the services' trace metrics instead.
- Lag is in messages; it is not converted to seconds.
