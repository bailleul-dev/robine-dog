---
title: Search syntax
section: Search
order: 10
description: One query language filters logs, spans, RUM events, events, security signals and monitors. It is Datadog's search syntax.
---

# Search syntax

The search bars of the Log, Trace, RUM and Event explorers, the queries of
monitors and dashboard widgets, and the `query` field of the API all use the
same syntax. A query is made of **terms** combined with **boolean operators**.

```text
service:shop-api status:error @http.status_code:>=500 -env:dev "payment declined"
```

This finds the error logs of `shop-api` that have a 5xx status code, are not
in the `dev` environment and contain the phrase *payment declined*.

## Terms

| Query | Matches |
| --- | --- |
| `timeout` | Events whose message contains the word *timeout* |
| `"payment declined"` | The exact phrase |
| `service:web` | A reserved attribute or a tag equal to a value |
| `@http.method:POST` | An attribute (the `@` prefix), with dots for nested keys |
| `env:prod` | A tag (`key:value` without `@` on anything that is not a reserved attribute) |

Free text matches **whole words**, case-insensitively: `time` does not match
*timeout*. Add a wildcard (`time*`) to match part of a word.

### Reserved attributes

These keys are columns, not tags or attributes, and are the fastest to
filter on.

| Logs | Spans |
| --- | --- |
| `service`, `host`, `source`, `status`, `trace_id`, `message` | `service`, `resource_name`, `operation_name`, `env`, `version`, `trace_id`, `span_id`, `parent_id`, `span.type`, `status`, `@duration` |

For logs, `status:` accepts the usual aliases: `status:warning` finds
`warn`, `status:err` finds `error`.

For spans, an attribute is looked up in the span's tags first, then in its
numeric metrics, so `@http.status_code:500` and `http.status_code:500` find
the same spans.

## Boolean operators

| Operator | Example | Meaning |
| --- | --- | --- |
| *(space)* or `AND` | `service:web env:prod` | Both terms |
| `OR` | `status:error OR status:warn` | Either term |
| `-` or `NOT` | `-env:dev`, `NOT env:dev` | Exclude |
| `( )` | `service:web (status:error OR @http.status_code:>=500)` | Grouping |
| `key:(a OR b)` | `service:(web OR api)` | Any of several values of one key |

Operators are written in capitals; `and`, `or` and `not` in lower case are
plain words.

## Wildcards and existence

| Query | Matches |
| --- | --- |
| `host:web-*` | Values that start with `web-` |
| `@usr.email:*@example.com` | Values that end with `@example.com` |
| `service:*` | Events that have the key at all |
| `-@error.kind:*` | Events that do not have it |

On tags, `*` stops at a comma: `env:p*` matches `env:prod` and not the next
tag.

## Numerical values

| Query | Matches |
| --- | --- |
| `@http.status_code:>=500` | Comparisons: `>`, `>=`, `<`, `<=` |
| `@retries:[1 TO 5]` | A range, both bounds included |
| `@duration:>1.5s` | Span durations, with a unit: `ns`, `us`, `ms`, `s`, `m`, `h` |

A number with no unit on `@duration` is in nanoseconds, as Datadog stores
it.

## Escaping

Put a value in double quotes when it contains a space or a special
character, or escape the character with `\`:

```text
@url:"/api/v1/orders?page=2"
@path:\/var\/log\/app.log
```

## Where to use it

| Where | What the query filters |
| --- | --- |
| [Log Explorer](log-explorer.md), Live Tail | Logs; `index:<name>` picks an [index](log-indexes.md) or a rehydrated archive |
| [Trace Explorer](trace-explorer.md) | Spans |
| [RUM Explorer](rum.md) | Views, actions, errors, resources, long tasks |
| [Events](events.md) | Monitor alerts, Agent events, deployments, incidents |
| [Log monitors](monitors.md#log-monitors), [APM monitors](monitors.md#apm-monitors) | The events counted by the monitor |
| Dashboard widgets | The logs, spans or RUM events of a query |
| Log and Trace explorers, their analytics, facets and patterns | `@usr.id:reference_table(customers, tier:gold)` keeps the events whose value is in a [reference table](reference-tables.md)'s matching rows; monitors and pipeline filters do not expand it |

Metric queries filter with a different syntax, between braces:
`avg:system.cpu.user{env:prod,!host:web-1}`. See [Metrics](metrics.md#querying-metrics).

## Facets

Clicking a value in an event's side panel offers **Filter by**,
**Exclude** and **Copy as query**, which write the term for you. Facets on
the left of the explorers show the values of an attribute in the current
results and filter with a click; see [Facets](log-explorer.md#facets).
