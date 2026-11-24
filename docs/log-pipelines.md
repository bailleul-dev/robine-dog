---
title: Log Pipelines
section: Logs
order: 20
app: /logs/pipelines
description: Pipelines parse and enrich every log at intake with grok rules, remappers and processors, in Datadog's format, so raw lines become structured, searchable attributes.
---

# Log Pipelines

A log line such as `10.0.3.7 - - [16/Nov/2026:16:31:00 +0000] "GET /api/cart HTTP/1.1" 503 12`
is only text until something extracts its client IP, method, URL and
status code. Pipelines do that at intake: each pipeline has a filter query,
and every log that matches runs through the pipeline's processors, in order.
The attributes they create can then be searched, turned into
[facets](log-explorer.md#facets), counted by [log-based metrics](generate-metrics.md)
and used by [indexes](log-indexes.md).

Pipeline definitions use the JSON of Datadog's Logs Pipelines API, so a
pipeline exported from Datadog can be pasted into Robine Dog and the other
way round.

## How logs are processed

Every log received through an intake goes through the same steps:

1. **Preprocessing.** JSON messages are parsed and the reserved attributes
   (`status`, `service`, `host`, `date`, `dd.trace_id`…) are read, as
   described in [Log collection](log-collection.md).
2. **Pipelines.** The enabled pipelines run in the order of the list. A
   pipeline whose filter does not match the log is skipped; one that
   matches runs its enabled processors in order. Each pipeline sees what
   the previous ones did.
3. **After the pipelines.** The [Sensitive Data Scanner](sensitive-data-scanner.md)
   scans the log, then [log-based metrics](generate-metrics.md),
   [forwarding](log-forwarding.md), [archives](log-archives.md) and finally
   the [indexes](log-indexes.md), which decide whether the log is stored.

This applies to the HTTP and Agent logs intake, OpenTelemetry logs, Sentry
events, and browser logs and errors sent by the RUM SDK.

Pipelines run when a log arrives. Changing a pipeline does not reprocess
logs already stored.

## The Pipelines page

[Pipelines](/logs/pipelines) lists every pipeline, in processing order,
with its filter and its number of processors.

- **New Pipeline** asks for a filter (a [search query](search-syntax.md);
  empty matches every log), a name and a description.
- The switch on each row enables or disables the pipeline.
- The arrows move a pipeline up or down. They are disabled while the
  **Filter pipelines** box is in use.
- **Clone** makes an editable copy named `<name> (clone)`, added at the end
  of the list.
- Click a pipeline to open it: its processors are listed in order, each with
  its own switch, arrows, edit and delete buttons. **Add Processor** opens
  the processor editor; choosing **Nested Pipeline** adds a pipeline inside
  the pipeline, with its own filter.

New pipelines are added at the end of the list.

### Integration pipelines

Robine Dog installs read-only pipelines for common software, marked
**Integration**. They are installed the first time Robine Dog starts and
refreshed on every start, keeping whether you enabled them.

| Pipeline | Filter | What it parses |
| --- | --- | --- |
| Nginx | `source:nginx` | Access logs (common and combined formats) and error logs; status from the status code or the error level; URL and user-agent details |
| PostgreSQL | `source:postgresql` | Lines with the default `log_line_prefix` (`%m [%p] `) or none; statement durations; severities mapped to statuses (`LOG` is `info`, `FATAL` is `critical`) |
| Kubernetes | `source:kubernetes` | kubelet `key=value` messages |
| Traefik | `source:traefik` | JSON access logs (`--accesslog.format=json`): standard HTTP attributes, a `METHOD host/path status` message, trace and span ids |
| CoreDNS | `source:coredns` | Query logs (client, question, response code, duration) and level-only lines |
| Redis | `source:redis` | Server logs, with the severity character mapped to a status |
| Datadog Agent | `source:agent` | Agent logs (`date \| COMPONENT \| LEVEL \| (file) \| message`) |
| Garage | `source:garage` | Colored log lines, and the S3 and admin requests in them |
| Go (logfmt, klog) | *(every log)* | logfmt lines starting with `time=`, `ts=`, `t=`, `level=` or `lvl=`, and klog lines (`I1117 10:13:59.888380 …`); the parsed level, message and time become the log's |

An integration pipeline can only be enabled or disabled: it cannot be
edited or deleted. To change one, clone it, edit the clone and disable the
original. Otherwise both run.

## Processors

Each processor has a `type`, a `name` and `is_enabled`. Sources are tried in
order: the first one present on the log is used. An attribute path uses dots
for nested keys (`http.status_code`).

| Processor | `type` | Fields | What it does |
| --- | --- | --- | --- |
| Grok Parser | `grok-parser` | `source` (default `message`), `grok.match_rules`, `grok.support_rules`, `samples` | Parses the source text with grok rules into attributes. See [Grok parser](#grok-parser) |
| Date Remapper | `date-remapper` | `sources` | Sets the log's timestamp: epoch seconds, milliseconds, microseconds or nanoseconds, or an RFC 3339 string |
| Status Remapper | `status-remapper` | `sources` | Sets the log's status. Syslog numbers (`0` to `7`) and names are normalized: `WARNING` is `warn`, `err` is `error`, `fatal` is `emergency` |
| Service Remapper | `service-remapper` | `sources` | Sets the log's service |
| Message Remapper | `message-remapper` | `sources` | Sets the log's message |
| Remapper | `attribute-remapper` | `sources`, `source_type` (`attribute` or `tag`), `target`, `target_type` (`attribute` or `tag`), `target_format` (`auto`, `string`, `integer`, `double`), `preserve_source`, `override_on_conflict` | Copies an attribute or tag value to another attribute or tag, converting it if asked. The source is removed unless `preserve_source` is true. An existing target is kept unless `override_on_conflict` is true |
| URL Parser | `url-parser` | `sources`, `target`, `normalize_ending_slashes` | Writes `scheme`, `host`, `port`, `path` and `queryString` (an object of the parameters) to the target. Relative URLs get only `path` and `queryString`. `normalize_ending_slashes` drops a trailing `/` from the path |
| User-Agent Parser | `user-agent-parser` | `sources`, `target`, `is_encoded` | Writes `browser` and `os` (family, major, minor, patch) and `device` (family, and a category: Desktop, Mobile, Tablet, Bot or Other) |
| Category Processor | `category-processor` | `target`, `categories` (`filter.query`, `name`) | Sets the target to the name of the first category whose query matches the log |
| Arithmetic Processor | `arithmetic-processor` | `expression`, `target`, `is_replace_missing` | Computes `+`, `-`, `*`, `/`, `%`, parentheses and `abs()`, `ceil()`, `floor()`, `round()` over attributes. A missing attribute skips the processor, or counts as `0` with `is_replace_missing` |
| String Builder Processor | `string-builder-processor` | `template`, `target`, `is_replace_missing` | Fills a template such as `%{http.method} %{http.url}`. A missing attribute skips the processor, or is replaced by an empty string with `is_replace_missing` |
| Lookup Processor | `lookup-processor` | `source`, `target`, `lookup_table` (`"key,value"` lines), `default_lookup`, or `lookup_enrichment_table` | Maps the source value through the list, falling back to `default_lookup`. With `lookup_enrichment_table`, the target gets the whole row of a [reference table](reference-tables.md) |
| Trace Id Remapper | `trace-id-remapper` | `sources` | Sets the log's trace id (decimal, or 32 or 16 hexadecimal characters), which links the log to its trace |
| Span Id Remapper | `span-id-remapper` | `sources` | Sets the log's span id |
| Nested Pipeline | `pipeline` | `filter.query`, `processors` | Runs its processors on the logs matching its filter. Pipelines nest one level only |

The status, service, message, date, trace id and span id remappers leave the
source attribute in place.

## Grok parser

A grok parser holds **parsing rules**, one per line, each written
`rule_name pattern`. The pattern is a regular expression in which
`%{MATCHER:attribute:filter}` extracts a value. Rules are tried in order and
the **first rule that matches the whole text** wins; if none matches, the
log is left as it was. Lines starting with `#` are comments.

**Helper rules** (`support_rules`) are written the same way. They are not
matched on their own but can be used from parsing rules, or from each other,
as `%{rule_name}`. A parsing rule can also reuse another parsing rule by name.

Extracted attributes are merged into the log: `http.method` lands in the
`http` object next to the attributes already there.

### Matchers

| Matcher | Matches |
| --- | --- |
| `date("pattern"[, "timezone"])` | A date in Java/Joda syntax (`yyyy-MM-dd HH:mm:ss.SSS`, `dd/MMM/yyyy:HH:mm:ss Z`), stored as epoch milliseconds. The timezone is an IANA name or an offset; UTC by default. A date without a year gets the current year |
| `regex("pattern")` | A regular expression |
| `notSpace` | Everything up to the next space |
| `word` | Letters, digits and underscores, between word boundaries |
| `data` | Anything, spaces included (lazy) |
| `boolean("true", "false")` | `true` or `false` (case-insensitive), or the two values given |
| `number`, `numberStr` | A decimal number, as a number or as a string |
| `numberExt`, `numberExtStr` | A number with an optional exponent (`1.5e3`) |
| `integer`, `integerStr` | An integer, as a number or as a string |
| `integerExt`, `integerExtStr` | An integer with an optional exponent |
| `doubleQuotedString`, `singleQuotedString`, `quotedString` | A quoted string, stored without its quotes |
| `uuid` | A UUID |
| `mac` | A MAC address |
| `ipv4`, `ipv6`, `ip` | An IP address |
| `hostname`, `ipOrHost` | A host name, or an IP address or host name |
| `port` | A port number, stored as an integer |

In `date` patterns, fractions of seconds take 3, 6 or 9 digits (`SSS`,
`SSSSSS`, `SSSSSSSSS`).

### Filters

A filter after the attribute name transforms the value:
`%{notSpace:http.url:url}`.

| Filter | Result |
| --- | --- |
| `number`, `integer`, `boolean` | The value converted |
| `nullIf("value")` | No attribute when the value equals the argument (`nullIf("-")`) |
| `json` | The value parsed as JSON |
| `lowercase`, `uppercase` | The value in lower or upper case |
| `keyvalue([separator[, allowed[, quotes[, delimiter]]]])` | `key=value` pairs as attributes. The separator defaults to `=`; `allowed` adds characters allowed in unquoted values; quoted values use `"`, `'` or `<>` by default |
| `scale(factor)` | The number multiplied by the factor |
| `url` | The URL's parts, as the URL parser writes them |
| `querystring` | The parameters of a query string, as an object |
| `decodeuricomponent` | The value with `%xx` sequences decoded |
| `useragent` | The browser, OS and device, as the user-agent parser writes them |
| `csv("headers"[, separator[, quote]])` | The fields of a CSV line, named by the comma-separated headers |
| `array([brackets, ]separator[, filter])` | A list, such as `array("[]", ",")` for `[a,b]`; the optional last argument is a filter applied to each item |

With no attribute name, an object produced by a filter is merged at the root
of the log: `%{data::keyvalue}` turns `user=bob id=7` into the attributes
`user` and `id`.

### Example: an nginx line with the request time

The Nginx integration pipeline parses nginx's default formats. Suppose your
`log_format` adds `$request_time` after the response size:

```text
192.168.1.20 - alice [24/Nov/2026:10:15:32 +0000] "POST /api/orders?coupon=FALL HTTP/1.1" 201 512 0.250 "https://shop.example.com/cart" "Mozilla/5.0 (X11; Linux x86_64; rv:133.0) Gecko/20100101 Firefox/133.0"
```

Helper rules:

```text
_client_ip %{ipOrHost:network.client.ip}
_auth %{notSpace:http.auth:nullIf("-")}
_date %{date("dd/MMM/yyyy:HH:mm:ss Z"):date_access}
_request %{word:http.method} %{notSpace:http.url} HTTP/%{notSpace:http.version}
```

Parsing rule:

```text
access.timed %{_client_ip} - %{_auth} \[%{_date}\] "%{_request}" %{integer:http.status_code} %{integer:network.bytes_written} %{number:duration:scale(1000000000)} "%{notSpace:http.referer}" "%{data:http.useragent}"
```

The line becomes these attributes (`duration` is in nanoseconds, as Datadog
expects):

```json
{
  "network": { "client": { "ip": "192.168.1.20" }, "bytes_written": 512 },
  "http": {
    "auth": "alice",
    "method": "POST",
    "url": "/api/orders?coupon=FALL",
    "version": "1.1",
    "status_code": 201,
    "referer": "https://shop.example.com/cart",
    "useragent": "Mozilla/5.0 (X11; Linux x86_64; rv:133.0) Gecko/20100101 Firefox/133.0"
  },
  "date_access": 1795515332000,
  "duration": 250000000
}
```

Follow the grok parser with a date remapper on `date_access`, a category
processor and a status remapper for the status, then the URL and
user-agent parsers, as in the [API example](#api) below.

### Preview

The grok editor takes up to five log samples. As you type, each sample shows
the rule that matched (`match: access.timed`) or **No match**, and the
attributes it produced. Errors in the rules (an unknown matcher or filter, an
invalid regular expression, an unclosed `%{`) appear under the rules.
**Advanced Settings** holds the attribute to parse (**Extract from**) and
the helper rules.

## API

The pipelines are served on Datadog's paths, with the `DD-API-KEY` header.
Bodies and responses are Datadog's JSON.

| Method and path | Does |
| --- | --- |
| `GET /api/v1/logs/config/pipelines` | Lists the pipelines, in processing order |
| `POST /api/v1/logs/config/pipelines` | Creates a pipeline, at the end of the order |
| `GET /api/v1/logs/config/pipelines/{id}` | Reads a pipeline |
| `PUT /api/v1/logs/config/pipelines/{id}` | Replaces a pipeline. On an integration pipeline, only `is_enabled` is taken |
| `DELETE /api/v1/logs/config/pipelines/{id}` | Deletes a pipeline. Integration pipelines cannot be deleted |
| `GET /api/v1/logs/config/pipeline-order` | `{"pipeline_ids": [...]}` |
| `PUT /api/v1/logs/config/pipeline-order` | Sets the order. Pipelines not listed keep their relative order after the listed ones |

A pipeline whose filter or processors do not compile is rejected with the
error. This creates the pipeline of the example above:

```sh
curl -X POST http://localhost:8080/api/v1/logs/config/pipelines \
  -H "DD-API-KEY: $ROBINE_DOG_API_KEY" \
  -H "Content-Type: application/json" \
  -d @- <<'EOF'
{
  "name": "shop-web nginx",
  "is_enabled": true,
  "filter": { "query": "source:nginx service:shop-web" },
  "processors": [
    {
      "type": "grok-parser",
      "name": "Parse access logs with request time",
      "is_enabled": true,
      "source": "message",
      "samples": [],
      "grok": {
        "support_rules": "_client_ip %{ipOrHost:network.client.ip}\n_auth %{notSpace:http.auth:nullIf(\"-\")}\n_date %{date(\"dd/MMM/yyyy:HH:mm:ss Z\"):date_access}\n_request %{word:http.method} %{notSpace:http.url} HTTP/%{notSpace:http.version}",
        "match_rules": "access.timed %{_client_ip} - %{_auth} \\[%{_date}\\] \"%{_request}\" %{integer:http.status_code} %{integer:network.bytes_written} %{number:duration:scale(1000000000)} \"%{notSpace:http.referer}\" \"%{data:http.useragent}\""
      }
    },
    { "type": "date-remapper", "name": "Log date", "is_enabled": true, "sources": ["date_access"] },
    {
      "type": "category-processor", "name": "Status from code", "is_enabled": true,
      "target": "http.status_category",
      "categories": [
        { "filter": { "query": "@http.status_code:[200 TO 399]" }, "name": "info" },
        { "filter": { "query": "@http.status_code:[400 TO 499]" }, "name": "warn" },
        { "filter": { "query": "@http.status_code:[500 TO 599]" }, "name": "error" }
      ]
    },
    { "type": "status-remapper", "name": "Log status", "is_enabled": true, "sources": ["http.status_category"] },
    { "type": "url-parser", "name": "URL", "is_enabled": true, "sources": ["http.url"], "target": "http.url_details" },
    { "type": "user-agent-parser", "name": "User agent", "is_enabled": true, "sources": ["http.useragent"], "target": "http.useragent_details" }
  ]
}
EOF
```

### Terraform

The Datadog Terraform provider's `datadog_logs_custom_pipeline` resource
works against these endpoints. Point the provider at Robine Dog:

```hcl
provider "datadog" {
  api_url = "http://localhost:8080"
  api_key = var.robine_dog_api_key
  app_key = var.robine_dog_app_key
}

resource "datadog_logs_custom_pipeline" "payments" {
  name       = "payments"
  is_enabled = true
  filter {
    query = "service:payments"
  }
  processor {
    status_remapper {
      name       = "Status from level"
      is_enabled = true
      sources    = ["level"]
    }
  }
}
```

## Differences from Datadog

- Grok patterns are Rust regular expressions: no look-around, no
  backreferences and no atomic groups (`(?>…)`). Write `(?:…)` instead.
- The `xml` and `rubyhash` grok filters are not implemented; a rule using
  them is rejected.
- The GeoIP parser, array processor, decoder processor and other processor
  types not listed above are not implemented; a pipeline containing one is
  rejected.
- The integration pipelines are the nine listed above, not Datadog's
  library.
- The user-agent parser uses a compact set of rules covering common
  browsers, tools and bots.
