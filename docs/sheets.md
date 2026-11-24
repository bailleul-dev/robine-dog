---
title: Sheets
section: Dashboards
order: 40
app: /sheets
description: Saved tables over logs, spans or RUM events, grouped by facets, with measures, computed columns and a pivot on a second group.
---

# Sheets

A sheet is a saved table built from an aggregation: pick logs, spans or RUM
events, a search, the facets to group by and the measures to compute, then
add columns calculated from the others. It answers questions such as *the
p95 latency of each endpoint in milliseconds* or *the errors of each
service on each host* in a table you can keep and reopen.

Sheets use the same aggregations as the **Visualize as Table** view of the
[Log Explorer](log-explorer.md) and the [Trace Explorer](trace-explorer.md),
and the same [search syntax](search-syntax.md).

## Creating a sheet

On [Sheets](/sheets), click **New Sheet**. The left column lists the saved
sheets; click one to open it, or its trash icon to delete it. A sheet has
these settings:

| Setting | Meaning |
| --- | --- |
| Name | The sheet's name in the list |
| Dataset | **Logs**, **Spans** or **RUM** |
| Search query | The events to aggregate, `*` for all |
| Time span | The period, ending now: `1h`, `4h`, `1d` (the default), `1w` or `1mo` |
| Group by | Facets separated by commas, such as `service, @http.status_code`; at most 4. Each group keeps its 100 top values. |
| Measures | Aggregations separated by commas (see below) |
| Pivot the 2nd group | Turns the second group into columns |
| Computed columns | One per line, `name = expression` |

**Run** computes the table with the current settings without saving them.
**Save** saves the sheet and runs it. A sheet is computed again each time
you open it, over the time span ending at that moment.

## Measures

A measure is an aggregation, followed by `:` and a measure attribute when
the aggregation needs one.

| Measure | Value |
| --- | --- |
| `count` | The number of matching events |
| `cardinality:@usr.id` | The number of distinct values of an attribute |
| `sum:@amount`, `avg:@duration`, `min:@duration`, `max:@duration` | The sum, average, minimum or maximum of a numerical attribute |
| `median:@duration` | The median |
| `pc95:@duration` | A percentile: `pc75`, `pc90`, `pc95`, `pc99`, or any `pcNN` from 0 to 100 |

Each measure is a column of the table, named by its text (`count`,
`avg:@duration`). Span durations are in nanoseconds.

## Computed columns

A computed column is an arithmetic expression over the other columns:
`+`, `-`, `*`, `/`, parentheses, numbers and column names in square
brackets. A column can use the measures and the computed columns defined on
the lines above it.

```text
avg ms = [avg:@duration] / 1000000
p95 ms = [pc95:@duration] / 1000000
p95 over avg = [p95 ms] / [avg ms]
```

When a value is missing, or an expression divides by zero, the cell shows
`—`. Values are rounded to two decimals.

## Pivot

With **Pivot the 2nd group** checked and exactly two groups, the sheet
becomes a cross table: one row per value of the first group, one column per
value of the second, and the first measure in each cell. Computed columns
are not shown in a pivot.

For example, the error logs of each service on each host over the past day:

| Setting | Value |
| --- | --- |
| Dataset | Logs |
| Search query | `status:error` |
| Time span | `1d` |
| Group by | `service, host` |
| Measures | `count` |
| Pivot the 2nd group | checked |

## Example

The latency of each endpoint of a service, in milliseconds:

| Setting | Value |
| --- | --- |
| Dataset | Spans |
| Search query | `service:shop-api env:prod` |
| Time span | `1w` |
| Group by | `resource_name` |
| Measures | `count, avg:@duration, pc95:@duration` |
| Computed columns | `avg ms = [avg:@duration] / 1000000` and `p95 ms = [pc95:@duration] / 1000000` |

## Differences from Datadog

- A sheet is one aggregation and its computed columns: there are no
  lookups between tables, no references to reference tables and no
  formulas over individual events.
- The time span always ends now; a sheet cannot be pinned to a fixed
  period.
- Sheets are managed in the UI only: there is no public API for them.
