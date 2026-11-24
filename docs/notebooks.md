---
title: Notebooks
section: Dashboards
order: 20
app: /notebook/list
description: Notebooks mix Markdown text with metric graphs and log graphs in one document, for investigations, runbooks and postmortems.
---

# Notebooks

A notebook is a page of cells, read from top to bottom: text that explains
what you are looking at, and graphs that show it. Where a
[dashboard](dashboards.md) is a screen you keep open, a notebook is a
document you write: the notes of an investigation, a runbook, a
postmortem.

## The notebook list

[Notebooks](/notebook/list) lists every notebook with its number of cells
and when it was last modified, the most recent first. **New Notebook**
creates one with a first text cell and opens it.

## Writing a notebook

The header holds the notebook's name (click to edit it), **All notebooks**,
**Delete** and a time picker. Every change is saved as soon as you make it:
a notebook has no draft and no save button.

The buttons under the last cell add a cell:

| Button | Cell type | What you write | What it shows |
| --- | --- | --- | --- |
| **Text** | `markdown` | Markdown: `**bold**`, `` `code` ``, `[links](https://…)`, `#` titles, `-` lists | The formatted text |
| **Metric graph** | `metric` | A metric query, such as `avg:system.cpu.idle{*} by {host}` | A line graph of the query |
| **Log graph** | `logs` | A log search, such as `service:shop-api status:error` | The matching logs over time, stacked by status, with **Open in Logs** |

Metric graphs accept the whole [metric query syntax](metrics.md); log graphs
accept the [search syntax](search-syntax.md).

Each cell has buttons to move it up or down, to edit it (**Edit**, then
**Done** to save the cell) and to delete it. A new cell opens in edit mode.

All the graphs of a notebook share the time picker's range. Dragging across
a graph zooms every graph to the selected range.

## API

Notebooks are available through Datadog's notebook API, with an API key in
`DD-API-KEY`.

| Method and path | Does |
| --- | --- |
| `GET /api/v1/notebooks` | Lists the notebooks, with their cells |
| `POST /api/v1/notebooks` | Creates a notebook |
| `GET /api/v1/notebooks/{id}` | Reads a notebook |
| `PUT /api/v1/notebooks/{id}` | Replaces a notebook |
| `DELETE /api/v1/notebooks/{id}` | Deletes a notebook |

Cells are translated to and from Datadog's cell definitions:

| Robine Dog cell | Datadog cell definition |
| --- | --- |
| Text | `{"type": "markdown", "text": "…"}` |
| Metric graph | `{"type": "timeseries", "requests": [{"q": "…"}]}`; the first query of `requests[0].queries` is read too |
| Log graph | `{"type": "log_stream", "query": "…"}` |

```sh
curl -X POST http://localhost:8080/api/v1/notebooks \
  -H "DD-API-KEY: $ROBINE_DOG_API_KEY" \
  -H "DD-APPLICATION-KEY: $ROBINE_DOG_APP_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "data": {
      "type": "notebooks",
      "attributes": {
        "name": "Checkout latency, 12 March",
        "time": {"live_span": "1h"},
        "cells": [
          {"type": "notebook_cells", "attributes": {"definition":
            {"type": "markdown", "text": "## What happened\n\np95 latency of checkout doubled after the 14:00 deploy."}}},
          {"type": "notebook_cells", "attributes": {"definition":
            {"type": "timeseries", "requests": [{"q": "avg:trace.http.request.duration{service:checkout} by {version}"}]}}},
          {"type": "notebook_cells", "attributes": {"definition":
            {"type": "log_stream", "query": "service:checkout status:error"}}}
        ]
      }
    }
  }'
```

A notebook needs a `name`. Its id is a number, as in Datadog.

The MCP server at `/v1/mcp` (see [API](api.md)) exposes notebooks to AI assistants
with the tools `search_datadog_notebooks`, `get_datadog_notebook`,
`create_datadog_notebook` and `append_datadog_notebook`.

## Differences from Datadog

- Cells are text, metric graphs and log graphs. Any other cell Datadog's API
  sends (a top list, a heatmap…) is kept as a text cell holding its JSON
  definition in a code block.
- A notebook has no time range of its own: the API always reports
  `live_span: 1h`, and the UI opens on the past hour.
- There are no notebook templates, comments or snapshots; the API reports
  every notebook as `published`.
