---
title: Code Security
section: Security
order: 30
app: /security/code
description: Software Composition Analysis from the tracers' telemetry. Robine Dog lists the libraries each service loads and checks them once a day against the OSV vulnerability database.
---

# Code Security

Code Security finds known vulnerabilities in the open source libraries your
services run. It needs no scanner in CI: the Datadog tracers already report
the libraries they see loaded, and Robine Dog checks each one against
[OSV](https://osv.dev), the open source vulnerability database. The result
is in [Code Security](/security/code).

## How it works

1. A tracer sends its **telemetry** to the trace agent: at start-up (`app-started`) and when new libraries are loaded (`app-dependencies-loaded`), it lists each library with its version.
2. Robine Dog reads these messages on its trace port (`/telemetry/proxy/api/v2/apmtelemetry` on 8126) and remembers every library per service, env and language.
3. Once a day per library version, Robine Dog asks OSV whether it is affected by a known vulnerability, and fetches the details of each vulnerability the first time it sees it.

## Set up

Point your tracers at Robine Dog's trace port, as for traces (see
[APM tracers](tracing.md)). Telemetry is on by default in the tracers.

```sh
DD_SERVICE=shop-api DD_ENV=prod \
DD_TRACE_AGENT_URL=http://localhost:8126 \
ddtrace-run gunicorn shop.wsgi
```

> **Note:** telemetry is read on the trace port only. If you turned it off
> with `DD_INSTRUMENTATION_TELEMETRY_ENABLED=false`, turn it back on.

Robine Dog must reach `https://api.osv.dev` over HTTPS. The libraries of
these languages are checked:

| Tracer language | OSV ecosystem |
| --- | --- |
| `python` | PyPI |
| `nodejs`, `node`, `javascript` | npm |
| `java`, `jvm` | Maven |
| `ruby` | RubyGems |
| `go`, `golang` | Go |
| `dotnet`, `.net` | NuGet |
| `php` | Packagist |

Libraries of other languages are listed but not checked.

### ROBINE_DOG_OSV_URL

| Value | Effect |
| --- | --- |
| Not set | OSV's public API, `https://api.osv.dev` |
| A URL | An OSV-compatible API at that address (a mirror or a proxy): Robine Dog calls `<url>/v1/querybatch` and `<url>/v1/vulns/<id>` |
| Empty (`ROBINE_DOG_OSV_URL=`) | No lookup at all. Libraries are still recorded, and the page says lookups are off. |

## When checks run

The first check runs a minute after Robine Dog starts. Each pass takes up to
1,000 library versions that were never checked or were last checked more
than 24 hours ago, and queries OSV in batches of 500. While there are
libraries left to check, passes follow each other a minute apart; when
nothing is due, or OSV fails, the next pass is an hour later. A new
library therefore shows its vulnerabilities within minutes of the service
loading it.

## The Code Security page

The header counts the libraries loaded, the services loading them, and the
library versions checked against OSV. The table lists each vulnerable
library per service:

| Column | Shows |
| --- | --- |
| Service | The service and its env |
| Library | Name, version and language |
| Vulnerability | The OSV identifier (linked to osv.dev), its aliases (CVE, GHSA…) and summary |
| Severity | OSV's `database_specific.severity` (such as `HIGH`), or else the first severity score (a CVSS vector) |
| Fixed in | The versions that fix it, from the affected ranges |

When nothing is listed, no known vulnerability affects the libraries seen.

## Differences from Datadog

- Only Software Composition Analysis of loaded libraries. Static analysis (SAST), secrets scanning, infrastructure-as-code scanning and runtime code vulnerabilities (IAST) are not implemented.
- Vulnerabilities come from OSV only; there is no risk scoring, remediation workflow or muting.
- `datadog-ci sbom upload` and repository scans are not read.
- There is no public API for the findings.
