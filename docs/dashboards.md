---
title: Dashboards
section: Dashboards
order: 10
app: /dashboard/lists
description: Graph metrics, logs, spans and RUM events side by side on a grid of widgets. Dashboards are stored as Datadog's dashboard JSON, so you can import and export them between Robine Dog and Datadog.
---

# Dashboards

A dashboard is a set of widgets, each running one or more queries over the
same time frame. Robine Dog stores dashboards in **Datadog's dashboard JSON
format** (`title`, `layout_type`, `widgets`, `template_variables`), so a
dashboard exported from Datadog can be pasted into Robine Dog and the other
way round.

When there is no dashboard at all, Robine Dog installs a starter dashboard
named **Overview**: requests, error rate and error logs, hits and p95 latency
by service, logs by status and the services with the most errors, with a
`$service` template variable.

## The dashboard list

[Dashboard List](/dashboard/lists) shows every dashboard with its number of
widgets and when it was created and last modified.

- **New Dashboard** creates an empty dashboard and opens it.
- **Search dashboards** filters the list by title.
- The left column switches between **All dashboards**, your **Favorites**
  and the dashboard lists. **New list** creates a list.
- Tick dashboards to **Add them to a list**, or, inside a list, to
  **Remove them** from it. **Rename list** and the trash icon act on the
  current list; deleting a list does not delete its dashboards.
- The star next to a dashboard adds it to your favorites. Favorites are kept
  per user.

## Building a dashboard

The header of a dashboard holds its title (click it to rename the
dashboard), the favorite star, the **Layout** selector, **Share**, **JSON**,
**Clone**, the trash icon and the time picker. Changes are saved as you
make them; there is no separate edit mode.

**Add Widget** opens the widget editor. Its top half is a live preview of
the widget with the dashboard's current time frame and variables. The
bottom half has two tabs:

- **Edit**: the visualization, the title, the queries, the formulas and the
  options of the widget type.
- **JSON**: the widget's `definition`, to edit by hand. **Apply JSON**
  updates the preview.

Each widget has a menu to edit, clone or delete it.

### Layouts

| Layout | `layout_type` | How widgets are placed |
| --- | --- | --- |
| Grid layout | `ordered` | A 12-column grid with rows of 92 px. `layout.x` and `layout.width` are in columns (0 to 12), `layout.y` and `layout.height` in rows. |
| Free layout | `free` | A grid of square units, at least 96 units wide. Drag a widget by its title bar to move it and by its corner to resize it; it snaps to the units. |

Switching the layout converts the positions: one grid column or row is 8
units of the free layout. On a screen narrower than 760 px, both layouts
show one widget per row.

### Queries and formulas

Widgets that graph data are made of **queries**, each named with a letter,
and **formulas** over them. A query reads one data source:

| Data source | Query | Example |
| --- | --- | --- |
| `metrics` | A metric query; see [Metrics](metrics.md) | `avg:system.cpu.user{env:prod} by {host}` |
| `logs` | A [search](search-syntax.md), an aggregation and group-by facets | `service:shop-api status:error`, `count` by `service` |
| `spans` | The same, over spans | `service:shop-api`, `pc95` of `@duration` by `resource_name` |
| `rum` | RUM events (in the JSON only) | `@type:view`, `count` by `@geo.country_iso_code` |
| `events` | [Events](events.md) (in the JSON only) | `source:alert`, `count` |

The editor offers metrics, logs and spans; `rum` and `events` queries work
when you write them in the widget's JSON.

For logs, spans, RUM and events, the aggregations are `count`,
`cardinality`, `avg`, `sum`, `min`, `max`, `median` and `pcNN` (`pc75`,
`pc90`, `pc95`, `pc99`…). Every aggregation but `count` needs a measure, such
as `@duration`. For a single value (query value, top list, table…), a metric
query is reduced over the time frame with its `aggregator`: `avg` (the
default), `sum`, `min`, `max` or `last`.

A formula combines queries with `+ - * /` and parentheses, and functions:

| Kind | Functions |
| --- | --- |
| Arithmetic | `abs`, `log2`, `log10`, `sqrt`, `default_zero`, `clamp_min`, `clamp_max`, `cutoff_min`, `cutoff_max` |
| Rate and count | `per_second`, `per_minute`, `per_hour`, `diff`, `derivative`, `monotonic_diff`, `cumsum`, `integral`, `count_nonzero`, `count_not_null` |
| Smoothing and gaps | `moving_rollup`, `fill`, `exclude_null`, `trend_line`, `robust_trend`, `piecewise_constant` |
| Timeshift | `timeshift(a, -3600)`, `hour_before`, `day_before`, `week_before`, `month_before` |
| Rank | `top(a, 10, 'mean', 'desc')`, `bottom(…)` |
| Algorithms | `anomalies`, `outliers`, `forecast` |

For example, an error rate in percent from two span queries:

```json
{
  "response_format": "scalar",
  "queries": [
    {"data_source": "spans", "name": "a", "search": {"query": "service:shop-api status:error"}, "compute": {"aggregation": "count"}},
    {"data_source": "spans", "name": "b", "search": {"query": "service:shop-api"}, "compute": {"aggregation": "count"}}
  ],
  "formulas": [{"formula": "a / b * 100", "alias": "error rate"}]
}
```

Series are matched between queries by their group tags; an ungrouped query
applies to every group of the other. Requests in Datadog's older format, a
single `q` string such as `"q": "avg:system.load.1{*} by {host}"`, are read
as metric queries.

### Template variables

Template variables narrow every widget at once. **+ Add variables** (or
**Edit variables**) defines them; each has a name, a prefix (the tag or
facet it filters) and a default value, `*` for everything.

In queries, `$name` becomes `prefix:value`, or nothing when the value is
`*`; `$name.value` becomes the value alone:

| Variable | Value | `avg:system.cpu.user{$env} by {host}` | `$env status:error` |
| --- | --- | --- | --- |
| `env`, prefix `env` | `prod` | `avg:system.cpu.user{env:prod} by {host}` | `env:prod status:error` |
| `env`, prefix `env` | `*` | `avg:system.cpu.user{*} by {host}` | `status:error` |

The selectors above the widgets list the variable's `available_values` when
the definition has some, and otherwise the 50 most frequent values of the
facet in the spans and logs of the time frame. The selected values are kept
in the URL (`?tpl_var_env=prod`), so a link opens the dashboard on them.

**Saved views** name a set of variable values. Pick values, click **Save
view** and type a name; the views selector switches between them and
**Default view** goes back to the defaults. Views are stored in the
dashboard's `template_variable_presets`, as in Datadog.

### Importing and exporting

**JSON** shows the whole dashboard in Datadog's format, without its `id`,
`url` and dates. **Copy** copies it. To import a dashboard exported from
Datadog, open the JSON of a new dashboard, paste the export and **Save**.
**Clone** makes a copy named `<title> (cloned)`.

## Widgets

| Widget | `type` | What it shows |
| --- | --- | --- |
| Timeseries | `timeseries` | Lines or bars over time (`display_type`: `line`, `bars`), with a legend |
| Query Value | `query_value` | One number, with decimals, a unit and conditional formatting |
| Top List | `toplist` | Groups ranked by value |
| Table | `query_table` | Groups in rows, one column per formula |
| Note | `note` | Markdown text on a colored background |
| Group | `group` | Widgets nested under one title (from imported JSON) |
| Heatmap | `heatmap` | The distribution of a timeseries' groups over time |
| Distribution | `distribution` | A histogram of a measure (`@duration`) or of a metric query's groups |
| Pie Chart | `sunburst` | The share of each group |
| Treemap | `treemap` | Groups as nested rectangles sized by value |
| Scatter Plot | `scatterplot` | Two queries, one per axis, a point per group |
| Change | `change` | Each group's value against an hour, a day, a week or a month before, absolute or relative |
| Geomap | `geomap` | A world map colored by ISO country code (`@geo.country_iso_code`) |
| Host Map | `hostmap` | Hosts as tiles colored by a metric, grouped by tags |
| List | `list_stream` | The latest logs, spans, RUM events, events or Error Tracking issues of a query |
| Event Stream, Event Timeline | `event_stream`, `event_timeline` | Events matching a query, as a list or along time |
| SLO | `slo` | An [SLO](slos.md)'s status and error budget over its time windows |
| Monitor Summary | `manage_status` | The [monitors](monitors.md) matching a search, counted by status and listed |
| Alert Graph, Alert Value | `alert_graph`, `alert_value` | A monitor's query as a graph with its thresholds, or its current value |
| Check Status | `check_status` | A service check's status, counted per status or the worst one |
| Service Summary | `trace_service` | A [service](services.md)'s hits, errors and latency |
| Funnel | `funnel` | RUM sessions through a sequence of steps, such as `@view.name` values |
| Split Graph | `split_graph` | One small graph per value of a tag (`one_graph_per: host`) |
| Free Text, Image, IFrame | `free_text`, `image`, `iframe` | Text, a picture or a web page, without a title bar |

Conditional formatting on a query value compares the value (`>`, `>=`,
`<`, `<=`) with a number and colors it with a palette: `white_on_red`,
`white_on_yellow`, `white_on_green`, `white_on_gray`, `red_on_white`,
`yellow_on_white`, `green_on_white`, `black_on_light_red`,
`black_on_light_yellow`, `black_on_light_green`.

A widget type Robine Dog does not render shows *The `<type>` widget is not
supported yet*; it is kept in the JSON.

## Public links

**Share** creates a read-only link that anyone can open without signing
in, at `/p/<token>`. When you create it you choose:

- the **Default time frame**: past 15 minutes, 1 hour, 4 hours, 1 day, 2 days
  or 1 week;
- whether **viewers can change the time frame**;
- which **variables viewers can change**. The others stay on their default
  value.

The Share dialog lists the dashboard's links, with **Copy** and **Revoke**.

> **Warning:** anyone with the link sees the data the dashboard's widgets
> show. Revoke a link you no longer need; deleting the dashboard revokes
> all of its links.

A public page only runs the queries stored in the dashboard's widgets, with
the variable values the link allows, over 30 days at most. Widgets that
read something else than their stored queries stay empty on a public page:
distribution, scatter plot, host map, list, event stream and timeline, SLO,
monitor summary, alert graph and value, check status, service summary,
funnel and split graph.

## API

The dashboard API takes Datadog's JSON and needs an API key in
`DD-API-KEY`.

| Method and path | Does |
| --- | --- |
| `GET /api/v1/dashboard` | Lists the dashboards (id, title, layout, URL, widget count, dates) |
| `POST /api/v1/dashboard` | Creates a dashboard |
| `GET /api/v1/dashboard/{id}` | Reads a dashboard |
| `PUT /api/v1/dashboard/{id}` | Replaces a dashboard |
| `DELETE /api/v1/dashboard/{id}` | Deletes a dashboard, its public links and its place in lists |
| `GET`, `POST /api/v1/dashboard/lists/manual` | Lists or creates dashboard lists (`{"name": "…"}`) |
| `GET`, `PUT`, `DELETE /api/v1/dashboard/lists/manual/{id}` | Reads, renames or deletes a list |
| `GET`, `POST`, `PUT`, `DELETE /api/v2/dashboard/lists/manual/{id}/dashboards` | Lists, adds, replaces or removes a list's dashboards (`{"dashboards": [{"id": "…", "type": "custom_timeboard"}]}`) |
| `POST /api/v1/dashboard/public` | Creates a public link |
| `GET`, `PUT`, `DELETE /api/v1/dashboard/public/{token}` | Reads, changes or revokes a public link |

Dashboard ids look like Datadog's (`abc-def-ghi`). A dashboard needs a
`title`; `layout_type` is `ordered` (the default) or `free`; every widget
needs a `definition`, and widgets without an `id` get one.

```sh
curl -X POST http://localhost:8080/api/v1/dashboard \
  -H "DD-API-KEY: $ROBINE_DOG_API_KEY" \
  -H "DD-APPLICATION-KEY: $ROBINE_DOG_APP_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "title": "Checkout",
    "layout_type": "ordered",
    "template_variables": [{"name": "env", "prefix": "env", "default": "prod"}],
    "widgets": [
      {
        "definition": {
          "type": "timeseries",
          "title": "p95 latency (ms)",
          "requests": [{
            "response_format": "timeseries",
            "display_type": "line",
            "queries": [{"data_source": "spans", "name": "a",
                         "search": {"query": "service:checkout $env"},
                         "compute": {"aggregation": "pc95", "metric": "@duration"}}],
            "formulas": [{"formula": "a / 1000000"}]
          }]
        },
        "layout": {"x": 0, "y": 0, "width": 6, "height": 3}
      }
    ]
  }'
```

A public link takes Datadog's body:

```sh
curl -X POST http://localhost:8080/api/v1/dashboard/public \
  -H "DD-API-KEY: $ROBINE_DOG_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "dashboard_id": "abc-def-ghi",
    "share_type": "open",
    "global_time": {"live_span": "4h"},
    "global_time_selectable_enabled": true,
    "selectable_template_vars": [{"name": "env", "visible_tags": ["prod", "staging"]}]
  }'
```

`live_span` is a number and a unit: `m`, `h`, `d`, `w` or `mo` (`15m`,
`1d`, `1mo`). In `selectable_template_vars`, `visible_tags` limits the
values a viewer may pick; leave it empty to allow any value. The response
holds the `token` and the `public_url`.

Widgets run their queries through Datadog's query API,
`POST /api/v2/query/timeseries` and `POST /api/v2/query/scalar`, which you
can call yourself with the same `queries` and `formulas` and a `from` and
`to` in milliseconds.

## Differences from Datadog

- Public links are only `open` shares: invitation-only shares are refused.
- There are no powerpacks, and the dashboard time picker has no live mode.
- Favorites are not reported by the API: `is_favorite` is always `false`,
  and dashboards in a list have `robine-dog` as their author.
- The widget editor builds metrics, logs and spans queries; other data
  sources are written in the JSON.
- Cloning, favorites and the Share dialog's list of links are features of
  the UI, not of the public API.
