---
title: Reference Tables
section: Logs
order: 80
app: /logs/reference-tables
description: CSV tables of business data, keyed by a primary key, that enrich logs at intake through the lookup processor and filter searches with reference_table().
---

# Reference Tables

Logs carry identifiers: a customer id, a service name, a merchant code.
The information you want to search and group by (the customer's plan, the
team that owns the service) lives somewhere else. A reference table brings
it in: you upload a CSV file, and Robine Dog uses it in two places.

- At intake, the [lookup processor](#enrich-logs-with-the-lookup-processor)
  of a [pipeline](log-pipelines.md) adds the row matching a log's attribute
  to the log.
- In searches, [`reference_table()`](#filter-searches)
  keeps the logs whose attribute is one of the table's keys, optionally only
  the rows matching a condition.

## Create a table

A table is a CSV file with a header row. One column is the **primary key**:
its values identify the rows and must be unique.

```text
customer_id,name,tier,seats,account_manager
c-1001,"Acme, Inc.",gold,120,claire
c-1002,Globex,silver,15,marc
c-1003,Initech,gold,48,claire
```

| Setting | Rule |
| --- | --- |
| Name | Lowercase letters, digits and underscores, starting with a letter, up to 56 characters (`customers`) |
| Primary key | One column; the first one by default. Each row needs a different value |
| Rows | Up to 200,000 |
| CSV | Comma-separated, values in double quotes when they contain a comma or a line break, `""` for a quote. Every row has as many fields as the header. Blank lines and a UTF-8 byte order mark are ignored |

### Column types

Column types are inferred from the file:

| Type | When |
| --- | --- |
| `INT32` | Every value of the column is a 32-bit integer. The value is a number in the logs it enriches |
| `STRING` | Anything else. The primary key is always `STRING` |

In the example, `seats` is `INT32` and the other columns are `STRING`.

### The Reference Tables page

[Reference Tables](/logs/reference-tables) lists the tables with their
primary key, columns, number of rows and last update.

- **New Reference Table** asks for the name, the primary key (a column of
  the header you pasted), a description and the CSV, typed in or chosen as a
  file.
- Click a table's name to see its rows, with each column's type, and search
  them (**Search rows** matches any column, case-insensitively).
- **Replace CSV** uploads a new version of the file. The name and the
  primary key stay; the rows are replaced entirely.
- The trash icon deletes the table.

Tables are stored in PostgreSQL and kept in memory. A change applies to the
next logs received and the next searches.

## Enrich logs with the lookup processor

Add a **Lookup Processor** to a pipeline and choose the table under
**Reference Table**. The processor reads the source attribute, finds the
row whose primary key equals it, and writes the whole row, as an object, to
the target attribute. When no row matches, the log is left unchanged.

```json
{
  "type": "lookup-processor",
  "name": "Customer details",
  "is_enabled": true,
  "source": "usr.id",
  "target": "customer",
  "lookup_enrichment_table": "customers"
}
```

A log with `"usr": {"id": "c-1001"}` gets:

```json
{
  "customer": {
    "customer_id": "c-1001",
    "name": "Acme, Inc.",
    "tier": "gold",
    "seats": 120,
    "account_manager": "claire"
  }
}
```

You can then search `@customer.tier:gold`, create a [facet](log-explorer.md#facets)
on `@customer.account_manager` or group [log-based metrics](generate-metrics.md)
by it. The lookup happens at intake: logs stored before the table changed
keep the values they were given.

## Filter searches

In a search, `reference_table()` stands for the keys of a table:

| Query | Keeps the logs whose attribute is |
| --- | --- |
| `@usr.id:reference_table(customers)` | Any primary key of `customers` |
| `@usr.id:reference_table(customers, tier:gold)` | The key of a row whose `tier` is `gold` |
| `@usr.id:reference_table(customers, account_manager:cl*)` | The key of a row whose `account_manager` starts with `cl` |
| `@usr.id:reference_table(customers, name:"Acme, Inc.")` | The key of a row whose `name` is `Acme, Inc.` (quotes for spaces and commas) |
| `-@usr.id:reference_table(customers)` | Not a key of the table |

The term combines with the rest of the query like any other:

```text
service:checkout status:error @usr.id:reference_table(customers, tier:gold)
```

The table's matching keys are written into the query before it runs, as
`@usr.id:("c-1001" OR "c-1003")`. Only the first 2,000 matching keys are
used: on a larger table, narrow the term with a condition. A table or
column that does not exist is an error.

`reference_table()` works in the searches of the [Log Explorer](log-explorer.md)
and the [Trace Explorer](trace-explorer.md), their analytics, facets and
patterns, and the log and span search APIs. Monitor queries and pipeline
filters do not expand it.

## API

The endpoints are Datadog's, with the `DD-API-KEY` header. A table can be
named by its id or by its name in the paths.

| Method and path | Does |
| --- | --- |
| `GET /api/v2/reference-tables/tables` | Lists the tables, by name, with `row_count` and `schema` (`primary_keys`, `fields` with their `type`) |
| `POST /api/v2/reference-tables/tables` | Creates a table |
| `GET /api/v2/reference-tables/tables/{id}` | Reads a table's description and schema |
| `PATCH /api/v2/reference-tables/tables/{id}` | Replaces the rows (a new CSV) or the description |
| `DELETE /api/v2/reference-tables/tables/{id}` | Deletes a table |
| `GET /api/v2/reference-tables/tables/{id}/rows` | Rows: `filter[row_id]=c-1001,c-1003` by key, or the first ones; `search` keeps rows containing the text; `limit` defaults to 100, at most 1,000. `meta.total` is the table's row count |

`POST` takes either a plain body with the CSV inline:

```sh
curl -X POST http://localhost:8080/api/v2/reference-tables/tables \
  -H "DD-API-KEY: $ROBINE_DOG_API_KEY" -H "Content-Type: application/json" \
  -d "$(jq -n --rawfile csv customers.csv \
        '{table_name: "customers", description: "Customer accounts",
          primary_key: "customer_id", tags: ["team:sales"], csv: $csv}')"
```

or Datadog's JSON:API shape, with the file's content in
`data.attributes.file_metadata.content` and the key in
`data.attributes.schema.primary_keys`. `PATCH` takes `csv` (or
`file_metadata.content`) and `description`, in either shape:

```sh
curl -X PATCH http://localhost:8080/api/v2/reference-tables/tables/customers \
  -H "DD-API-KEY: $ROBINE_DOG_API_KEY" -H "Content-Type: application/json" \
  -d "$(jq -n --rawfile csv customers.csv '{csv: $csv}')"
```

## Differences from Datadog

- Files are uploaded with the request. Tables are not synced from cloud
  storage, and `source` is always `LOCAL_FILE`.
- A table has one primary key column, and two column types, `STRING` and
  `INT32`.
- A table holds at most 200,000 rows.
