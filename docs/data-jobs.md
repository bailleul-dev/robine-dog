---
title: Data Jobs Monitoring
section: APM
order: 90
app: /data-jobs
description: Spark applications, jobs and stages traced by the Java tracer, with their runs, failures and durations, each run linked to its trace.
---

# Data Jobs Monitoring

Batch jobs fail and slow down like services do, but they do not serve
requests, so service pages say little about them. Data Jobs Monitoring
summarizes Spark workloads: how often each application ran, how many runs
failed, how long they took, and which jobs and stages made them slow.

Robine Dog builds this view from the spans the Datadog Java tracer creates
for Spark. There is no separate intake: the spans arrive like any other
trace and are searchable in the [Trace Explorer](trace-explorer.md) too.

## Setup

Attach the Datadog Java tracer to the Spark driver (and executors) with the
Data Jobs integration turned on, and point it at Robine Dog's trace port:

```sh
spark-submit \
  --conf "spark.driver.extraJavaOptions=-javaagent:/opt/dd-java-agent.jar -Ddd.data.jobs.enabled=true -Ddd.service=nightly-orders -Ddd.env=prod -Ddd.agent.host=robine-dog -Ddd.trace.agent.port=8126" \
  --class com.example.NightlyOrders orders.jar
```

The tracer reports each Spark application as a trace whose spans are named:

| Span name | Level |
| --- | --- |
| `spark.application` | One run of the application |
| `spark.job` | A Spark job of the run |
| `spark.stage` | A stage of a job |
| `spark.sql` | A Spark SQL query |

The application is the span's `service`. A run fails when its span has the
error flag.

> **Note:** Data Jobs spans go through the [retention filters](ingestion-control.md)
> like any trace. With the default filters every trace is kept; if you lower
> the default rate, add a filter such as `operation_name:spark.application`
> at 100% to keep every run.

## The Data Jobs page

[Data Jobs Monitoring](/data-jobs) opens on the last day and refreshes every
minute.

### Summary

One row per application and level (`application`, `job`, `stage`, `sql`):

| Column | Meaning |
| --- | --- |
| Application | The service of the spans |
| Level | The span name without `spark.` |
| Runs | The number of spans of that level in the range |
| Failed | Those with the error flag |
| Avg duration, Max duration | Over the range |
| Last run | When the latest span of that level started |

### Application runs

The latest 200 runs (`spark.application` spans): application, resource,
duration, a **failed** badge on errors, when it started, and a **Trace**
link that opens the run's trace with its jobs and stages in the
[trace view](trace-explorer.md#the-trace-view).

## Differences from Datadog

- Only Spark through the Java tracer is supported; Databricks, Airflow, dbt
  and EMR integrations are not.
- There are no cost, resource-utilization or idle-executor views, and no
  data jobs metrics for monitors. Alert on the
  [trace metrics](trace-explorer.md#trace-metrics) (`trace.spark.application.errors`)
  instead.
