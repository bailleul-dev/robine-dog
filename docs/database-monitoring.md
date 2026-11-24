---
title: Database Monitoring
section: APM
order: 70
app: /databases
description: The normalized queries, samples, explain plans and active sessions the Datadog Agent collects from PostgreSQL and MySQL, to find the queries that cost the most.
---

# Database Monitoring

APM shows how long your services wait on the database; Database Monitoring
shows what the database does. The Datadog Agent's PostgreSQL and MySQL
checks, with Database Monitoring turned on, read the database's own
statistics (`pg_stat_statements`, `performance_schema`) and send:

- **Query metrics**: for each normalized query (literals replaced by `?`),
  the calls, time spent and rows over each collection interval.
- **Samples**: individual executions of a query, with their explain plan.
- **Activity**: the sessions running right now, with their state and wait
  event.

Robine Dog receives these payloads on Datadog's intake paths and shows them
on one page.

## Setup

Database Monitoring is set up in the Agent, as Datadog documents it: create
the monitoring user and enable `pg_stat_statements` (PostgreSQL) or
`performance_schema` (MySQL), then turn on `dbm` in the check:

```yaml
# /etc/datadog-agent/conf.d/postgres.d/conf.yaml
init_config:
instances:
  - dbm: true
    host: db-1.internal
    port: 5432
    username: datadog
    password: "<password>"
    tags:
      - env:prod
```

Then point the Agent's three Database Monitoring endpoints at Robine Dog, the
same way as its other endpoints (see [Datadog Agent](agent.md)):

```yaml
# datadog.yaml
database_monitoring:
  metrics:
    dd_url: robine-dog:8080
  samples:
    dd_url: robine-dog:8080
  activity:
    dd_url: robine-dog:8080
```

| Endpoint | Receives |
| --- | --- |
| `POST /api/v2/dbmmetrics` | Query metrics (`postgres_rows`, `mysql_rows`) |
| `POST /api/v2/databasequery` | Samples with plans, and activity snapshots |

Both are on port 8080, need the `DD-API-KEY` header, and accept JSON
compressed with gzip, deflate or zstd.

### What is read

| | PostgreSQL | MySQL |
| --- | --- | --- |
| Query text | `query` | `digest_text` |
| Database | `datname` | `schema_name` |
| User | `rolname` | none |
| Calls | `calls` | `count_star` |
| Time | `total_exec_time` (or `total_time`), in ms | `sum_timer_wait`, converted from picoseconds |
| Rows | `rows` | `sum_rows_sent` |

Rows without a `query_signature`, or with no calls in the interval, are
ignored.

## The Database Monitoring page

[Database Monitoring](/databases) opens on the last hour and refreshes every
30 seconds. Use the host selector to look at one database host.

### Top queries

The 200 normalized queries that took the most total time in the range:

| Column | Meaning |
| --- | --- |
| Normalized query | The query text, literals replaced |
| Database | The database name and the engine (`postgres` or `mysql`) |
| Calls | Executions in the range |
| Avg latency | Total time divided by calls |
| Total time | Time spent executing the query |
| Rows | Rows returned or affected |

Click a query to open its panel:

- **Calls and average latency** over the range, in about 60 points (one
  minute at least).
- **Samples and explain plans**: the latest 50 samples of the query, with
  host, database, statement, duration and, when the Agent collected one, the
  explain plan as JSON.

### Active sessions

The latest activity snapshot of each host, if it arrived in the last two
minutes: host, user, state, wait event and the query running. This list is
kept in memory and is empty after a restart until the Agent sends the next
snapshot.

## Storage

Query metrics and samples are stored in PostgreSQL (tables
`dbm_query_metrics` and `dbm_samples`). Robine Dog does not delete them
automatically.

## Differences from Datadog

- Only PostgreSQL and MySQL payloads are read; SQL Server, Oracle and
  MongoDB are not.
- There are no recommendations, schema views or wait-event graphs; the
  check's regular metrics (`postgresql.*`, `mysql.*`) are in the
  [Metrics Explorer](metrics.md) like any other.
- Spans are not linked to the queries they ran (no DBM and APM
  correlation).
- Query metrics and samples have no retention period.
