---
title: Log Explorer
section: Logs
order: 10
app: /logs
description: Search, filter and analyse your logs. The Log Explorer lists matching logs, graphs them, groups them into patterns and links each log to its trace.
---

# Log Explorer

The [Log Explorer](/logs) is where you investigate the logs Robine Dog has
stored. Type a query in the [search syntax](search-syntax.md), narrow it with
facets, open a log to read its attributes, then switch the visualization to
count, graph or cluster what you found. Everything on the page (query, time
range, visualization, selected log) is kept in the URL, so you can share a
link to exactly what you see.

Logs reach the explorer after their [pipelines](log-pipelines.md), the
[Sensitive Data Scanner](sensitive-data-scanner.md) and the
[indexes](log-indexes.md). A log dropped by an index exclusion filter is not
stored and does not appear here, though it still counts in
[log-based metrics](generate-metrics.md) and is still
[archived](log-archives.md).

## Time range

The time picker at the top right sets the period searched. The default is
the past 15 minutes.

- Pick a preset: 5 minutes, 15 minutes, 1 hour, 4 hours, 1, 2, 3, 7 or 15 days.
- Type a relative duration (`45m`, `3h`, `2d`, `1w`) or an absolute range
  (`Nov 16 10:00 - Nov 16 12:00`) and press Enter.
- Drag across the histogram, or any timeseries, to zoom into that period.

A relative range is live: it moves forward every 10 seconds. The pause
button freezes it; the play button makes it live again. The refresh button
runs the query again.

> **Note:** logs are kept 15 days by default, and an index can keep its
> logs for less. See [Retention](log-indexes.md#retention).

## Search

The search bar takes Datadog's [search syntax](search-syntax.md):

```text
service:shop-api status:error @http.status_code:>=500 -env:dev
```

As you type, the bar suggests facets, and the attributes and tags seen in
the current results, then the most frequent values of the key you picked. Arrow keys move
through the suggestions; Enter runs the search unless you moved into the list
first. An invalid query shows its error under the bar.

Two special keys change where the explorer searches:

| Query | Effect |
| --- | --- |
| `index:<name>` | Logs kept by that [index](log-indexes.md) (every kept log carries the `index` tag) |
| `index:<view>` | A [historical view](log-archives.md#rehydrate-archived-logs) rehydrated from an archive instead of the live logs |
| `@usr.id:reference_table(customers, tier:gold)` | Logs whose value is in a [reference table](reference-tables.md)'s matching rows |

## Facets

Facets are the panel on the left. Each one shows the most frequent values of
an attribute or tag in the current search and time range, with their counts.

- **Core** facets are always there: Source, Host, Service and Status.
- Other facets are the ones you created. They start collapsed unless the
  query already filters on them. Expanding one loads its values.
- The counts of a facet ignore the facet's own clause in the query, so
  selecting a value does not hide the others.

To filter with a facet:

| Action | Query written |
| --- | --- |
| Click a value (or its **only** button) | `service:shop-api`; click again to clear it |
| Tick several checkboxes | `service:(shop-api OR checkout)` |
| Untick every value | The clause is removed |

A facet lists 10 values; **Show more** lists up to 50. The **Search facets**
box at the top filters the panel by name or path. The Status facet sorts
values by severity and colors them.

### Creating a facet

You can add a facet in two ways:

- Click **Add** in the panel's last group header, then type a path
  (`@http.status_code` for an attribute, `env` for a tag), a display name
  and a group. The group defaults to *Attributes*.
- In a log's side panel, click an attribute value and choose **Create facet
  for @path**. The path is filled in for you.

Facets are saved in PostgreSQL and shared by every user. The trash icon on a
facet's header removes it. Attributes that are facets are marked *facet* in
the side panel.

### Measures

Robine Dog has no separate measure type: any numeric attribute is a measure.
Type its path (`@duration`, `@http.response_time`, `@order.total`) in the
Analytics controls to sum, average or take percentiles of it.

## The log list

Below the histogram, the list shows matching logs newest first, 100 at a
time; scrolling down loads more until **End of results**. Each row shows
the status color, the date, the host, the service and the content: the
message, or the log's attributes as JSON when it has no message. The
toolbar shows how many logs match in total.

The histogram above the list counts matching logs over the time range in
about 80 bars, stacked by status.

## The log side panel

Click a log to open its side panel:

- The header shows the status, the service and the full date, with
  previous and next buttons.
- Host, Service, Source, Status and Trace ID.
- **View Trace**, when the log has a trace id.
- The message.
- The tags.
- **Event Attributes**, as a tree you can expand.

With the panel open, `j` or the down arrow opens the next log, `k` or the up
arrow the previous one. The selected log is in the URL (`event=<id>`), so a
link to it opens the panel directly.

### Attribute actions

Click any value in the panel (a core field, a tag or an attribute) to open
its menu:

| Action | What it does |
| --- | --- |
| **Filter by** | Adds `path:value` to the query |
| **Exclude** | Adds `-path:value` to the query |
| **Create facet for** | Opens the new facet form for that attribute (attributes that are not facets yet) |
| **Copy value** | Copies the value to the clipboard |
| **Copy as query** | Copies `path:value` to the clipboard |

Filtering on a value that is excluded in the query, or the reverse, replaces
the opposite clause instead of adding a contradiction.

## Logs and traces

A log is linked to a trace when it carries `dd.trace_id` (and `dd.span_id`),
which the ddtrace libraries add with `DD_LOGS_INJECTION=true`; a top-level
`trace_id` attribute works too. Both 64-bit decimal ids and 128-bit
hexadecimal ids are accepted: the low 64 bits are kept, which is how spans
are stored, so both forms correlate.

- **View Trace** in the log side panel opens the trace in the
  [Trace Explorer](trace-explorer.md), 15 minutes either side of the log, with
  the log's span selected.
- The **Logs** tab of a span lists the logs of its trace; each opens back in
  the Log Explorer.
- `trace_id:<id>` in the search bar finds every log of a trace.

## Live Tail

[Live Tail](/logs/livetail) shows new logs matching a query as they arrive.
It polls every second and keeps the latest 1,000 logs on screen, newest at
the top. The header shows how many logs arrived in the last second, and
**Pause** stops the stream while you read. Click a log to open the same side
panel as in the explorer.

> **Note:** Live Tail reads stored logs, so it does not show logs that an
> index exclusion filter dropped. In Datadog, Live Tail shows every log
> before indexing.

## Analytics

**Visualize as** above the list switches between views:

| View | Shows |
| --- | --- |
| List | The logs, with the histogram |
| Patterns | Logs grouped by message shape (see [Patterns](#patterns)) |
| Timeseries | One line, or bars, per group over the time range |
| Top List | Groups ranked by their value, with a bar each |
| Table | Groups and their value; with two facets, a row per combination |

The controls read *Show [aggregation] of [measure] by [facet] limit to top N*:

| Aggregation | Needs a measure |
| --- | --- |
| Count of | No: counts matching logs |
| Unique count of | Yes: the number of distinct values (`@usr.id`) |
| Avg, Sum, Min, Max, Median of | Yes: a numeric attribute |
| pc75, pc90, pc95, pc99 of | Yes |

- **by** takes any facet, tag or attribute; empty means a single group.
- The Table view adds **then by** for a second facet. Groups are
  hierarchical, as in Datadog: the top N values of the second facet within
  each of the top N values of the first.
- **limit to top** keeps the top 5, 10, 25, 50 or 100 groups.
- Click a legend entry, a top list row or a table cell to add that value to
  the query.

The settings live in the URL (`viz`, `agg`, `measure`, `by`, `by2`, `limit`)
and are saved with [saved views](#saved-views).

## Patterns

The Patterns view takes the latest 10,000 logs matching the query and
clusters them by the shape of their message, with a Drain-style algorithm:

- Only the first line of a message is used, up to 64 words.
- Words that contain a digit are variables from the start.
- A line joins a pattern with the same number of words and the same first
  word when at least half of the pattern's positions hold the same fixed
  word; the words that differ become `*` wildcards.

Each row shows the pattern's count, its main services, its most frequent
status (the color bar) and the pattern itself, with a sample log as a
tooltip. At most 500 patterns are listed. Click a pattern to go back to the
list with a query that finds its logs, built from its longest fixed phrases.

## Saved views

The **Views** menu left of the search bar saves the explorer's state under a
name: query, time range, visualization and its settings. Pick a view to
restore it; the button shows the name of the view you are on. The trash icon
deletes a view. Views are shared by every user.

## Watchdog Insights

Above the list, a **Watchdog Insights** banner appears when some facet value
is over-represented in the error logs of your search. Robine Dog compares
`status:error` logs with all logs matching the query, on `service`, `host`,
`env`, `source`, `version` and `@http.status_code`. A value is shown when it
appears in at least 5 error logs, makes up at least 25% of the errors, and
its share of errors is at least 1.5 times its share of all logs. The banner
reads, for example, *version:3.2.0 63% of errors vs 4%*.

Up to six insights are shown, the largest gap first. Click one to add
`status:error <facet>:<value>` to the query; × hides the banner. See
[Watchdog](watchdog.md) for anomaly detection.

## API

These endpoints take `DD-API-KEY` and use Datadog's request and response
formats.

| Endpoint | Purpose |
| --- | --- |
| `POST /api/v2/logs/events/search` | Search logs: `filter.query`, `filter.from`, `filter.to`, `page.limit` (1 to 1,000, default 10), `page.cursor` |
| `GET /api/v2/logs/events` | The same, with `filter[query]`, `filter[from]`, `filter[to]`, `page[limit]`, `page[cursor]` |
| `POST /api/v2/logs/analytics/aggregate` | Counts and aggregations, as a total or a timeseries, grouped by facets |

`from` and `to` take `now`, `now-15m`, `now-2h`, `now-1d`, an ISO 8601 date
or epoch milliseconds. Results are newest first; the next page's cursor is in
`meta.page.after`.

```sh
curl -X POST http://localhost:8080/api/v2/logs/events/search \
  -H "DD-API-KEY: $ROBINE_DOG_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"filter": {"query": "service:shop-api status:error", "from": "now-1h", "to": "now"},
       "page": {"limit": 50}}'
```

The aggregate request takes `compute` (an `aggregation` among `count`,
`cardinality`, `sum`, `avg`, `min`, `max`, `median`, `pcNN`; a `metric` for
all but `count`; `type` `total` or `timeseries`; an optional `interval` such
as `5m`), and up to four `group_by` facets, each with a `limit` (default
10, up to 1,000), a `sort` and a `missing` value for logs without the facet.

```sh
curl -X POST http://localhost:8080/api/v2/logs/analytics/aggregate \
  -H "DD-API-KEY: $ROBINE_DOG_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"compute": [{"aggregation": "count", "type": "timeseries", "interval": "5m"}],
       "filter": {"query": "service:shop-api", "from": "now-1h", "to": "now"},
       "group_by": [{"facet": "status", "limit": 5}]}'
```

Each bucket comes back as `{"by": {"status": "error"}, "computes": {"c0": …}}`,
`c0` being a number for a total or a list of `{time, value}` for a
timeseries.

## Differences from Datadog

- The list has fixed columns (date, host, service, content); you cannot add
  attribute columns.
- There is no export to CSV, and no option to download the list.
- Facets have no type: any numeric attribute is usable as a measure, and
  there is no unit or range slider for log measures.
- Live Tail shows stored logs only (after index exclusion filters).
- Patterns cluster the latest 10,000 logs; there is no pattern inspector or
  per-pattern histogram.
- Facets and saved views are shared by all users; there are no personal
  views.
