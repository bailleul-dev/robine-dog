# robine-dog

A personal, self-hosted log, trace and metrics explorer to practise Datadog's
Logs, APM and Metrics workflows on real data: the same search syntax, facets,
explorers, trace views, service pages and metric queries, fed by the **real
Datadog clients** (ddtrace libraries, DogStatsD, the logs and series HTTP
intakes). Sign-in is through Robine ID.

Rust (axum) + React (Vite, TanStack Query). One binary serves the UI, the API
and every intake. Telemetry (logs, spans, metric points) is stored in
[GreptimeDB](https://greptime.com) standalone, used only as a SQL store (table
TTLs, full-text and inverted indexes, JSON columns); the application's own
state (facets, metric metadata) is in PostgreSQL.

## What works

| Datadog | robine-dog |
| --- | --- |
| Log Explorer: search bar, facets, volume histogram (drag to zoom), log side panel, attribute actions (filter / exclude / create facet) | `/logs` |
| Live Tail | `/logs/livetail` |
| Trace Explorer: span search, facets, Service Entry / Root / All spans, slowest | `/apm/traces` |
| Trace view: flame graph, waterfall, span list, span tags/metrics, errors, correlated logs | side panel on a span |
| Service Catalog and service page: requests, errors, p50/p95/p99/max, latency distribution, resources, downstream services | `/apm/services` |
| Log ↔ trace correlation (`dd.trace_id` / `dd.span_id`, 64-bit or 128-bit) | "View Trace", span "Logs" tab |
| Search syntax: `AND`/`OR`/`NOT`/`-`, `( )`, `key:value`, `@attr:value`, `key:val*`, `key:*`, `@n:>=500`, `@n:[1 TO 5]`, `@duration:>1.5s`, `key:(a OR b)`, `"phrases"` | everywhere |
| Logs HTTP intake `POST /api/v2/logs` (JSON, gzip/deflate, `DD-API-KEY`), status/host/service/date remapping, JSON messages parsed | port 8080 |
| Trace agent API `/v0.3`, `/v0.4`, `/v0.5/traces` (msgpack/JSON), `/info` | port 8126 |
| Metrics Explorer: `avg:metric{tag:a,!tag:b} by {host}.as_count()`, `.as_rate()`, `.rollup(max, 60)`, p50…p99 on distributions, several queries, lines/bars | `/metrics/explorer` |
| Metrics Summary: type, unit, description (editable), tags and hosts per metric | `/metrics/summary` |
| DogStatsD (`c`, `g`, `s`, `h`/`ms`, `d`, sample rates, `#tags`, `\|T` timestamps), aggregated into 10-second buckets | UDP 8125 |
| Series intake `POST /api/v1/series`, `/api/v2/series` (JSON), `/api/v1/validate` | port 8080 |
| Analytics ("Visualize as" Timeseries / Top List / Table): count, unique count, avg/sum/min/max/median/pcNN of a measure, by up to two facets (top N per parent, like Datadog) | Log and Trace Explorers |
| Log Patterns: the latest 10,000 matching logs clustered by message shape (Drain), wildcards highlighted, click to search | Log Explorer |
| Aggregate API `/api/v2/logs/analytics/aggregate`, `/api/v2/spans/analytics/aggregate` (Datadog's request and bucket format) | port 8080 |
| Dashboards: Datadog's dashboard JSON (import / export), 12-column grid, timeseries (lines, bars), query value with conditional formatting, top list, table, note (Markdown) and group widgets, template variables (`$service`), widget editor with live preview | `/dashboard/lists` |
| Queries and formulas: metrics, logs and spans queries on one time grid, `a / b * 100`, `per_second`, `cumsum`, `diff`, `derivative`, `abs`, `log10`, `clamp_min/max`, `default_zero`, `timeshift`, `hour_before`… `week_before`, `top(a, 10, 'mean', 'desc')`; scalar queries reduce first, as Datadog | `/api/v2/query/timeseries`, `/api/v2/query/scalar` |
| Metric functions, in formulas and around Metrics Explorer queries (`anomalies(avg:x{*}, 'basic', 2)`): `ewma_3`…`ewma_20`, `median_3`…`median_9`, `moving_rollup`, `fill(null\|zero\|linear\|last, limit)` and `.fill()`, `integral`, `monotonic_diff`, `count_nonzero`, `count_not_null`, `exclude_null`, `cutoff_min/max`, `trend_line`, `robust_trend`, `piecewise_constant`; algorithms `anomalies(q, 'basic'\|'agile'\|'robust', bounds, seasonality=…)` (band of expected values, points outside it in red), `forecast(q, 'linear'\|'seasonal', deviations)` (the graph runs a quarter of its range into the future) and `outliers(q, 'dbscan'\|'mad'\|'scaledbscan'\|'scaledmad', tolerance, pct)` (other series greyed) | `/api/v2/query/timeseries` |
| Dashboards API `/api/v1/dashboard` (list, CRUD) | port 8080 |
| More dashboard widgets: heatmap, distribution (of a measure or of a metric's groups), pie chart (`sunburst`), treemap, scatter plot, change, geomap (world map by ISO country), host map, list (logs, spans, RUM, events, Error Tracking issues), event stream and timeline, SLO, monitor summary (`manage_status`), alert graph and value, check status, service summary (`trace_service`), RUM funnel, split graph, free text, image and iframe | `/dashboard/lists` |
| Dashboard features: free layout (`layout_type: free`, widgets moved and resized on a grid), dashboard lists (`/api/v1/dashboard/lists/manual`, `/api/v2/dashboard/lists/manual/{id}/dashboards`), favorites, saved views of the template variables (`template_variable_presets`), read-only public links (`/api/v1/dashboard/public`, page `/p/{token}` without sign-in: only the stored widgets' queries run, with the variables the link allows, over 30 days at most) | `/dashboard/lists` |
| Host Map: hosts as tiles colored by a metric, grouped by tags | `/infrastructure/map` |
| Saved views of the Log and Trace explorers (query, facets, visualization, time) | `/api/ui/views` |
| Generate Metrics: log- and span-based counts and distributions, tagged by attributes, computed at ingestion | `/api/v2/logs/config/metrics`, `/api/v2/apm/config/metrics` |
| OpenTelemetry: OTLP/HTTP traces, logs and metrics (protobuf and JSON, gzip), with Datadog's OTLP mapping | `/v1/traces`, `/v1/logs`, `/v1/metrics` |
| Service Map: services laid out by call depth, edges from real parent / child spans across services (calls, error rate, latency), health colors, neighbors on hover, details panel | `/apm/map` |
| Resource page: a resource's requests, errors, latency and its distribution, status codes, span summary (spans under it: per trace, average, p95, share of its time), slowest requests and latest errors | `/apm/services/<name>/resource` |
| Retention filters and Ingestion Control: every span feeds trace and span-based metrics, then ordered filters keep traces matching a query at a rate (decided on the trace id, like the tracers); spans, traces and volume per service, sampling decisions (`_dd.p.dm`, priority), tracer versions; `datadog.estimated_usage.apm.*` metrics | `/api/v2/apm/config/retention-filters`, `/apm/retention-filters`, `/apm/ingestion-control` |
| Software Catalog: service definitions (schema v2, v2.1, v2.2, JSON or YAML: team, contacts, links, tier, lifecycle, languages) shown on service pages and edited there; Teams with members, links and the services they own | `/api/v2/services/definitions`, `/api/v2/team`, `/teams` |
| Scorecards: Production Readiness, Observability Best Practices and Ownership & Documentation rules evaluated from definitions, monitors, SLOs, spans and logs; custom rules whose outcomes are posted through the API; scores per service and scorecard | `/api/v2/scorecard/rules`, `/api/v2/scorecard/outcomes`, `/apm/scorecards` |
| Continuous Profiler: pprof uploads of the Datadog profilers (gzip or zstd, v4 and older formats), merged over a period into a flame graph per service and version (CPU, wall time, allocations, heap…), top functions, time per endpoint, links from spans and service pages, raw pprof downloads; kept `ROBINE_DOG_PROFILE_RETENTION_DAYS` (8) | `/api/v2/profile`, `/profiling/v1/input`, `/profiling` |
| LLM Observability: spans of the ddtrace LLMObs SDK (agentless or through the Agent's EVP proxy), traces with their workflow, retrieval, LLM and tool spans, prompts and answers, tokens and estimated cost per model; evaluations (score, categorical); overview by model; `ml_obs.*` metrics | `/api/v2/llmobs`, `/api/intake/llm-obs/v2/eval-metric`, `/llm/overview`, `/llm/traces` |
| CI Visibility and Test Optimization: test events of ddtrace's test integrations (agentless or EVP proxy), JUnit reports from `datadog-ci junit upload`, pipelines and jobs from GitHub Actions and GitLab webhooks or the pipeline API; success rate and duration per pipeline, flaky tests (pass and fail on one commit), slowest tests; `ci.pipeline.*`, `ci.job.*` and `ci.test.*` metrics for monitors | `/api/v2/citestcycle`, `/api/v2/cireport`, `/api/v2/webhook`, `/api/v2/ci/pipeline`, `/ci/pipelines`, `/ci/tests` |
| Cloud SIEM: detection rules on logs (threshold on a count by group, or a new value of an attribute against a learning period), evaluated every minute, open security signals with severity; triage (under review, archived with a reason, assignee); default rules for SSH brute force, bursts of 401/403 and new admin users | `/api/v2/security_monitoring/rules`, `/api/v2/security_monitoring/signals`, `/security/signals` |
| Audit Trail: every change through the UI or the API (method, path, outcome) with its author, kept 90 days | `/api/v2/audit/events`, `/audit` |
| Case Management: cases with status, priority, assignee and project keys (`SHOP-12`), a timeline of comments and changes, opened by hand, through the API or from a security signal | `/api/v2/cases`, `/cases` |
| On-Call: rotation schedules, escalation policies per team; `@oncall-<team>` in a monitor message pages the team when it alerts (e-mail to whoever is on call, step after step until acknowledged in the UI) and resolves the page when it recovers; manual pages | `/api/v2/on-call/schedules`, `/api/v2/on-call/escalation-policies`, `/api/v2/on-call/pages`, `/on-call` |
| Live Processes: the process agent's snapshots (`DD_PROCESS_CONFIG_PROCESS_COLLECTION_ENABLED=true`, `DD_PROCESS_CONFIG_PROCESS_DD_URL`), per host, with CPU, memory, threads and command line; kept in memory | `/api/v1/collector`, `/processes` |
| Data Streams Monitoring: the tracers' pathway stats (`DD_DATA_STREAMS_ENABLED=true`): throughput, edge and end-to-end latency per service and queue, Kafka consumer lag | `/v0.1/pipeline_stats` (trace port), `/data-streams` |
| Database Monitoring: the Agent's postgres and mysql checks with `dbm: true` (`database_monitoring.{metrics,samples,activity}.dd_url`): top normalized queries, latency over time, samples with explain plans, active sessions | `/api/v2/dbmmetrics`, `/api/v2/databasequery`, `/databases` |
| Workflow Automation: workflows of steps (HTTP request to any URL or webhook, Kubernetes restart, scale or pod deletion, case, downtime, monitor mute, on-call page, approval in the UI, pause), with `{{ trigger.* }}` / `{{ steps.<name>.* }}` templates and `if` conditions; started by a monitor (`@workflow-<name>`), new security signals, a cron schedule, by hand or through the API; every run with its steps, inputs and outputs. Kubernetes steps need silo01's `robine_dog_workflow_namespaces` | `/api/v2/workflows`, `/api/v2/workflows/{id}/instances`, `/workflows` |
| App Builder: forms whose buttons run workflows with what was typed (`{{ inputs.values.<field> }}`) | `/api/v2/app-builder/apps`, `/apps` |
| Monitor Templates (CPU, disk, memory, Agent down, pod restarts and CrashLoopBackOff, APM errors and latency, error logs, RUM errors, certificate expiry) and Monitor Quality (monitors that notify nobody, are muted without an end or for over 30 days, alert for over 7 days, have no data, or watch cloud metrics without an evaluation delay) | `/monitors/templates`, `/monitors/quality` |
| Dashboard Reports: a dashboard printed to PDF by Chromium (through a public link made for the report) and e-mailed on a cron schedule, or by hand; PDF preview | `/dashboard/reports` |
| Sheets: saved tables over logs, spans or RUM (groups, measures, computed columns like `[errors] / [count] * 100`, pivot on a second group) | `/sheets` |
| Watchdog Insights: above the logs, traces and RUM explorers' results (and on their own page), the facet values over-represented in the errors of the search against everything (`version:3.2.0 is in 63% of the errors, against 4% of everything`), one click to filter | `/api/ui/insights`, `/watchdog/insights` |
| Watchdog Root Cause Analysis (a story's downstream dependencies on the service map with their own anomalies at the same time, the deepest one first; the callers impacted) and Impact Analysis (RUM sessions, users and views whose requests went through the anomalous service, by trace id; RUM resources now keep their `_dd.trace_id`) | `/watchdog` |
| Error Tracking Custom Grouping (an `error.fingerprint` tag or log attribute set by the application is the issue, whatever the message and stack), Issue Teams (each issue's team from the Software Catalog, a Team facet) and Exception Replay (the tracers' `_dd.debug.error.*.snapshot_id` link an error to its snapshots, received on the Agent's `/debugger/v1/input` and kept as `source:dd_debugger` logs with the local variables) | `/error-tracking` |
| CD Visibility: deployments from the DORA API (`POST /api/v2/dora/deployment`), `datadog-ci deployment mark` (`/api/intake/ci/custom_spans`, `cd.*` tags) and GitHub `deployment_status` / GitLab Deployment Hook webhooks (`/api/v2/webhook`), by service and env | `/ci/deployments` |
| DORA Metrics: deployment frequency, lead time (from the commit's first CI pipeline), change failure rate (failed, rolled back, or followed by an incident before the next deployment) and time to restore, from `POST /api/v2/dora/incident` and robine-dog's incidents; overall and per service | `/ci/dora` |
| Code Coverage: `datadog-ci coverage upload` (`/api/v2/cicovreprt`; lcov, Cobertura, JaCoCo), the latest coverage per repository, branch and flags with its trend | `/ci/coverage` |
| App & API Protection: the attacks the tracers' WAF records (`DD_APPSEC_ENABLED=true`, `_dd.appsec.json` on spans) become `source:appsec` logs (rule, type, attacker IP, route), kept whatever the trace retention; top attack types, attackers, services and rules; an out-of-the-box detection rule for one address attacking repeatedly; the API endpoint inventory from `http.route` with requests, errors and attacks | `/security/appsec` |
| Code Security (SCA): the libraries each service loads, from the tracers' telemetry (`app-dependencies-loaded` on the trace port's `/telemetry/proxy`), checked once a day against OSV (osv.dev; `ROBINE_DOG_OSV_URL=` empty turns it off), with the version that fixes each vulnerability | `/security/code` |
| Workload Protection and Cloud Security (Kubernetes and hosts): the security-agent's runtime events and compliance findings sent to robine-dog's logs intake (`runtime_security_config` / `compliance_config` `endpoints.logs_dd_url`), by rule, host, process, framework and resource, usable in detection rules | `/security/workload`, `/security/cloud` |
| Resource Catalog: hosts, containers, pods, deployments, nodes and network devices seen in the last hour, with their tags, in one filterable list | `/resources` |
| Cloud Network Monitoring: the system-probe's connections (process-agent `CollectorConnections`, on `/api/v1/collector` or `/api/v1/connections`) summed per minute and flow: bytes each way, retransmits, RTT, remote addresses resolved to hosts; kept 3 days | `/network` |
| Network Devices: the Agent's NDM metadata (`/api/v2/ndm`, SNMP integration): devices with vendor, model, OS, location, reachability, and their interfaces' admin and oper status | `/devices` |
| Fleet Automation, read only: every Agent with its version, OS, install method, configuration and running checks, from the inventory payloads (`/api/v1/metadata`) and host metadata | `/fleet` |
| Data Jobs Monitoring: Spark applications, jobs and stages from the Java tracer's spans (runs, failures, durations), each run linked to its trace | `/data-jobs` |
| LLM Experiments: the ddtrace SDK's projects, datasets (versioned records) and experiments API (`/api/{unstable,v2}/llm-obs/v1/...`, direct or through the Agent's EVP proxy); experiments of a dataset compared by their evaluations, and each row's input, output, expected output and scores | `/llm/experiments` |
| LLM Agent Observability: agents with their runs, errors, tool and LLM calls and tokens per run, tools by calls and errors, and each run drawn as its tree of steps | `/llm/agents` |
| Metrics without Limits: per metric, the tag keys kept (or dropped, `exclude_tags_mode`) when points are written, the rest aggregated away (counts and rates summed, gauges averaged), with the series ingested and stored this hour (`/api/v2/metrics/{name}/tags`, `/api/v2/metrics/{name}/volumes`) | `/metrics/tags` |
| Log Forwarding: custom destinations (`/api/v2/logs/config/custom-destinations`) receive the logs matching their query after the pipelines, in batches: any HTTP endpoint (JSON array, basic auth or a header), Splunk HEC or Elasticsearch (`_bulk`, dated index); sent, dropped and the last error per destination; secrets never returned | `/logs/forwarding` |
| API keys and application keys: named keys (`/api/v2/api_keys`, `/api/v2/application_keys`), stored hashed and shown once, with their last use, revocable; API keys are accepted wherever `ROBINE_DOG_API_KEY` is, which stays valid | `/organization-settings/api-keys` |
| Agent and OpenTelemetry setup pages: Docker, Helm and `datadog.yaml` for the Agent, SDK variables and a Collector configuration, filled in with this robine-dog's address; Help: this README in the app, searchable | `/organization-settings/agent`, `/organization-settings/opentelemetry`, `/help` |
| Error Tracking: error spans (`error.type`, `error.message`, `error.stack`) and error logs (`error.kind`, `error.stack`) grouped into issues by service, type, masked message and top stack frame; volume, first / last seen, stack trace with app frames highlighted, breakdown by version, env, host and resource, links to the trace or log; For Review / Reviewed / Ignored / Resolved, with regressions | `/error-tracking` |
| Sentry SDKs: envelope and store endpoints, the DSN's key being a client token; events become error logs (`error.kind`, `error.message`, `error.stack`, `usr.*`, `http.*`, trace ids) and Error Tracking issues | `/api/<project>/envelope/`, `/sentry/<service>/api/<project>/envelope/` |
| Monitors: metric (`avg(last_5m):avg:system.cpu.user{*} by {host} > 80`), logs (`logs("…").rollup(…).by(…).last("5m") > 10`) and APM (`trace-analytics(…)`) queries, evaluated every minute per group, OK / Warn / Alert / No Data, recovery thresholds, renotify, mute by scope, priority, event history and value graph, Manage / Triggered Monitors pages, an editor with live evaluation | `/monitors` |
| More monitor types: change (`pct_change(avg(last_5m),last_1h):…`), anomaly, outlier and forecast (`avg(last_4h):anomalies(avg:cpu{*} by {host}, 'agile', 2, direction='above', alert_window='last_15m') >= 1`, `max(next_1w):forecast(…) >= 90`), RUM (`rum("…")`), events (`events("…")`), Error Tracking (`error-tracking-traces("…").rollup("count").by("@issue.id")`), service checks (`"http.can_connect".over("env:prod").by("host").last(3).count_by_status()`, consecutive-run thresholds) and composites (`12 && (34 \|\| !56)`, per group, over the stored states); synthetic tests listed as `synthetics alert` monitors; `new_group_delay`, `evaluation_delay`, `escalation_message`; e-mail notifications to the addresses a message names (`@ops@example.com`) | `/monitors/create` |
| Service checks: the Agent's check runs and DogStatsD's `_sc\|name\|status\|…`, stored per check and host | `/api/v1/check_run`, UDP 8125 |
| Events: monitor state changes, Agent events and `POST /api/v1/events`, searched like logs (`source:alert status:error @monitor.id:12`) | `/api/v1/events` |
| Metrics volume (points and series stored per metric) and the integrations reporting, found from log sources and metric prefixes | — |
| Real User Monitoring: Datadog's browser SDK (`proxy: "https://<robine-dog>/rum/proxy"`, the client token as key): views with their web vitals, actions, resources, errors (also filed in Error Tracking), long tasks; browser logs through the same proxy. Sites install the SDK as Datadog documents it (npm or its CDN), with `proxy` added | `/rum/proxy`, `/api/v2/rum` |
| Watchdog: every five minutes each service's last quarter of an hour is compared with its previous day (error logs, failed requests, p95 latency); anomalies open a story and post a `source:watchdog` event | — |
| Notebooks: text, metric graphs and log graphs in cells | — |
| Incidents: severity, state, commander, a timeline of notes and changes; declared by hand or from a Watchdog story; each change of state posts a `source:incident` event | — |
| Status page: public at `/status` once enabled; components take their state from a synthetic test or a monitor, and incidents marked public are listed | — |
| MCP server, as Datadog's: streamable HTTP at `/v1/mcp`, the API key in `DD_API_KEY` (or `Authorization: Bearer`), the tools of Datadog's `core` toolset under their names (`search_datadog_logs`, `get_datadog_metric`, `search_datadog_spans`, `get_datadog_trace`, `search_datadog_monitors`, `search_datadog_incidents`, notebooks…) | `/v1/mcp` |
| Datadog's public API for reading, for its clients and SDKs (`DD-API-KEY`): search of logs, spans, RUM events and events; metric queries, lists and metadata; hosts; incidents; notebooks | `/api/v2/logs/events/search`, `/api/v2/spans/events/search`, `/api/v2/rum/events/search`, `/api/v2/events/search`, `/api/v1/events`, `/api/v1/query`, `/api/v1/metrics`, `/api/v2/metrics`, `/api/v1/search`, `/api/v1/hosts`, `/api/v2/incidents`, `/api/v1/notebooks` |
| Synthetic tests alert by themselves: when one starts failing (at once, or after `min_failure_duration`) and when it recovers, an event is posted and its message is sent to the `@webhook-…` it names | — |
| RUM sessions: what each visitor did (entry page, duration, views, actions, errors), opening on the session's events | — |
| RUM analytics: sessions and page views over time, top pages, sites, browsers, operating systems, device types and referrers. The browser, system and device type are read from the `User-Agent` at intake, as Datadog does (`@browser.name`, `@os.name`, `@device.type`) | `/api/v2/rum/analytics/aggregate` |
| Session Replay: the browser SDK's recordings (`sessionReplaySampleRate`) arrive through the same proxy (`/api/v2/replay`, multipart, compressed segments), are kept with the other data, and play in the session's page, view by view. Both of the SDK's formats play: rrweb's (SDK v5–v6) and the change records of v7, turned into rrweb's events in the browser | `/rum/replay/{session}` |
| RUM frustration signals: rage, dead and error clicks (`@action.frustration.type`) by page and by element, with the share of frustrated sessions and the latest ones; each session's count in the sessions list | `/rum/frustrations` |
| RUM funnels (views or `@facet:value` steps, by session, in order) and retention (users by day or week of first visit, and the share back each day or week after, by `@usr.id` or another attribute) | `/rum/funnels`, `/rum/retention` |
| RUM-based metrics: counts and distributions of sessions, views, actions, errors, resources, long tasks or vitals, tagged by attributes; sessions and views counted once, when they first match or when they end, with their final values | `/api/v2/rum/config/metrics` |
| Deployment tracking: on a service's page, each `version` that served requests with when it was first and last seen, its requests, error rate and latency; what got worse than the version before is in red | — |
| Monitor search with Datadog's syntax (`status:alert type:log tag:"env:prod" muted:true`, words of the name) and its counts per status, type and tag | `/api/v1/monitor/search` |
| Status page uptime: 30 days of daily bars and the uptime of each component backed by a synthetic test | — |
| Downtimes: scheduled silences by monitor, monitor tags and scope | `/api/v1/downtime` |
| Infrastructure: hosts, containers and Kubernetes resources (pods, deployments, stateful sets, daemon sets, nodes, namespaces), from the Agent's `system.*`, `container.*`, `kubernetes.*` and `kubernetes_state.*` metrics | — |
| Synthetic API tests: HTTP requests on a schedule with assertions (status code, response time, body, headers); each run is a `source:synthetics` log and the `synthetics.http.response.time` and `synthetics.test_runs` metrics | `/api/v1/synthetics/tests` |
| Synthetic tests of every protocol: SSL (certificate expiry, TLS version, issuer), TCP, DNS (`recordSome` / `recordEvery`, any server), UDP, WebSocket, gRPC (health check, metadata) and ICMP (latency, packet loss; needs `NET_RAW` or `net.ipv4.ping_group_range`); multistep API tests whose steps pass values on (`extractedValues` by JSONPath, regex, header or status, `allowFailure`, waits); test variables from patterns (`{{ numeric(6) }}`, `{{ uuid }}`, `{{ date(1d, YYYY-MM-DD) }}`) and global variables (`/api/v1/synthetics/variables`, secure ones never shown); retries; every run stored with its steps, browsable per test | `/api/v1/synthetics/tests` |
| Synthetic browser tests: steps (go to URL, click, type, press key, select, hover, scroll, wait, assert element / content / attribute / page text / URL, JavaScript assertions and extractions) run in a headless Chromium over the DevTools protocol, on a device size, with a screenshot of every step; `synthetics.browser.test_duration` | `/api/v1/synthetics/tests/browser` |
| Continuous Testing: `datadog-ci synthetics run-tests` with `DD_API_HOST_OVERRIDE` pointed here runs stored or local test definitions with their overrides (`startUrl`, `variables`, `headers`, `basicAuth`, `cookies`, `executionRule`, `deviceIds`, `retry`…), and reads back batches and results | `/api/v1/synthetics/tests/trigger/ci`, `/api/v1/synthetics/ci/batch/{id}`, `/api/v2/synthetics/tests/poll_results` |
| SLOs: metric-based (good over total events), monitor-based (time out of `Alert`, optionally some groups) and time-slice (1- or 5-minute slices whose value meets a condition); targets and warnings over 7, 30 and 90 days, error budget, burndown and burn-rate graphs on each SLO's page; status corrections, one-off or recurring (`RRULE` daily, weekly, monthly, in a time zone); history, search (`type:monitor team:shop`) and `can_delete`; burn-rate (`burn_rate("id").over("30d").long_window("1h").short_window("5m") > 14.4`) and error-budget (`error_budget("id").over("30d") > 75`) monitors | `/api/v1/slo`, `/api/v1/slo/correction`, `/slo/manage` |
| Notifications: Datadog message templates (`{{#is_alert}}`, `{{value}}`, `{{host.name}}`, `{{#is_match}}`…) to `@webhook-<name>` (Webhooks integration format: `$EVENT_TITLE`… payloads, custom headers, JSON or form) | `/monitors/webhooks` |
| Monitors and webhooks APIs `/api/v1/monitor` (CRUD, partial update, validate, mute/unmute), `/api/v1/integration/webhooks/configuration/webhooks` | port 8080 |
| Log Pipelines: grok parser (Datadog matchers and filters, helper rules, live sample preview), date / status / service / message / trace id / span id remappers, attribute remapper, URL and user-agent parsers, category, arithmetic, string builder and lookup processors, nested pipelines; read-only integration pipelines (Nginx, PostgreSQL, Kubernetes, Traefik, CoreDNS, Redis, Datadog Agent, Go logfmt / klog) | `/logs/pipelines`, applied at intake |
| Logs pipelines API `/api/v1/logs/config/pipelines`, `/pipeline-order` (Datadog's JSON, `DD-API-KEY`) | port 8080 |
| Log Indexes: ordered filters, exclusion filters with a sampling rate, daily quotas (reset time and UTC offset, warning threshold, usage of the day) and per-index retention; excluded logs still feed log-based metrics and archives; `index:<name>` in searches | `/api/v1/logs/config/indexes`, `/index-order`, `/logs/indexes` |
| Log Archives: logs written in Datadog's format (gzipped JSON lines, `dt=YYYYMMDD/hour=HH/`) to an S3-compatible bucket (AWS, MinIO, Garage, R2…) or a local directory, before index exclusions; rehydration into historical views searched with `index:<view>` | `/api/v2/logs/config/archives`, `/archive-order`, `/logs/archives` |
| Sensitive Data Scanner: scanning groups and rules (custom regex or 14 standard patterns: card numbers with Luhn, IBAN, e-mail, SSN, AWS / GitHub / Slack / Stripe keys, JWT, private keys…), on the whole event or some attributes; redact, hash or partially mask, and tag `sensitive_data:<rule>`, at intake | `/api/v2/sensitive-data-scanner/config`, `/logs/sensitive-data-scanner` |
| Reference Tables: CSV tables (typed columns, primary key) enriching logs through the lookup processor (`lookup_enrichment_table`) and filtering searches (`@usr.id:reference_table(customers, tier:gold)`) | `/api/v2/reference-tables/tables`, `/logs/reference-tables` |
| Host tags and host aliases from the Agent's metadata (`env`, `kube_cluster_name`, `DD_TAGS`…), joined to the logs and metrics of that host as Datadog does | `/intake/`, applied at ingestion |
| Datadog Agent: protobuf traces `/api/v0.2/traces` (classic and indexed formats), protobuf series, sketches `/api/beta/sketches`, logs, zstd | port 8080 |
| Trace metrics `trace.<span>.hits`, `.errors` and `trace.<span>` (distribution, seconds), tagged `service`, `resource_name`, `env`, `version`, `http.status_code` | computed at ingestion |


## Run it

Requirements: Rust 1.85+, Node 22+, Docker (for the databases).

```sh
cp .env.example .env    # then fill in the Robine ID client (see below)
make dev                # databases (docker compose), server :8080 + :8126 + udp :8125, UI http://localhost:5173
make demo               # in another terminal: an hour of history, then live traffic
```

`make deps` starts GreptimeDB (`127.0.0.1:14000`) and PostgreSQL
(`127.0.0.1:15432`) alone; `make deps-down` stops them, keeping their volumes.

Without Robine ID, on loopback only: `make dev-noauth`.

`make demo` simulates a small shop (web-store → shop-api → cart-service /
payment-service → Postgres, Redis, Stripe) with ~8% declined payments and ~3%
provider timeouts, plus host and business metrics over DogStatsD, using the
same wire formats as ddtrace, the Agent and DogStatsD.

## Sign-in with Robine ID

robine-dog is a confidential OIDC client: authorization code with PKCE S256
and a nonce, `client_secret_post`, ID token RS256-verified against the JWKS,
then a 12-hour HMAC-signed session cookie. Only the `sub` values listed in
`ROBINE_DOG_OIDC_SUBJECTS` are let in.

The clients `robine-dog` and `robine-dog-dev` are declared in silo01's
`robine_id_applications`; production deployment is described in
[`deploy/README.md`](deploy/README.md). The callback is
`<ROBINE_DOG_BASE_URL>/auth/callback` (`http://localhost:5173/auth/callback` in
development).

## Send it real data

**Traces** — any ddtrace library, robine-dog plays the local agent:

```sh
DD_TRACE_AGENT_URL=http://localhost:8126 DD_SERVICE=my-api DD_ENV=dev \
DD_LOGS_INJECTION=true ddtrace-run python app.py
```

(Node: `DD_TRACE_AGENT_URL=… node --require dd-trace/init app.js`; Java:
`-javaagent:dd-java-agent.jar -Ddd.trace.agent.url=…`; Go: `DD_TRACE_AGENT_URL`.)

**Logs** — the HTTP intake, like Datadog's:

```sh
curl -X POST http://localhost:8080/api/v2/logs \
  -H "DD-API-KEY: $ROBINE_DOG_API_KEY" -H 'Content-Type: application/json' \
  -d '[{"message":"payment failed","service":"pay","ddsource":"python",
        "ddtags":"env:dev,team:pay","status":"error","http":{"status_code":502}}]'
```

From a Datadog Agent: `DD_LOGS_CONFIG_USE_HTTP=true`,
`DD_LOGS_CONFIG_LOGS_DD_URL=<host>:8080`, `DD_LOGS_CONFIG_LOGS_NO_SSL=true`.

With `DD_LOGS_INJECTION=true`, the ddtrace log integrations add
`dd.trace_id`/`dd.span_id` to every log, which links logs and traces both ways.

**Metrics** — any DogStatsD client, robine-dog plays the Agent's DogStatsD
server:

```sh
DD_DOGSTATSD_URL=udp://localhost:8125     # or DD_AGENT_HOST + DD_DOGSTATSD_PORT
echo -n 'shop.orders:1|c|#env:dev,payment_method:card' | nc -u -w0 localhost 8125
```

Or the series API, as the Agent or a script posts it:

```sh
curl -X POST http://localhost:8080/api/v2/series -H "DD-API-KEY: $ROBINE_DOG_API_KEY" \
  -H 'Content-Type: application/json' \
  -d "{\"series\":[{\"metric\":\"queue.depth\",\"type\":3,\"points\":[{\"timestamp\":$(date +%s),\"value\":12}],
        \"tags\":[\"env:dev\"],\"resources\":[{\"type\":\"host\",\"name\":\"worker-1\"}]}]}"
```

**The Datadog Agent** — point every endpoint at robine-dog and give it a
site that doesn't resolve, so whatever is not overridden (process,
inventories, remote config…) can't reach Datadog with your key:

```sh
docker run -d --name dd-agent \
  -e DD_API_KEY=$ROBINE_DOG_API_KEY -e DD_SITE=robine-dog.invalid \
  -e DD_DD_URL=http://robine-dog:8080 \
  -e DD_APM_DD_URL=http://robine-dog:8080 \
  -e DD_LOGS_ENABLED=true -e DD_LOGS_CONFIG_USE_HTTP=true \
  -e DD_LOGS_CONFIG_LOGS_DD_URL=robine-dog:8080 -e DD_LOGS_CONFIG_LOGS_NO_SSL=true \
  -e DD_PROCESS_AGENT_ENABLED=false -e DD_PROCESS_CONFIG_PROCESS_COLLECTION_ENABLED=false \
  -e DD_PROCESS_CONFIG_CONTAINER_COLLECTION_ENABLED=false \
  -e DD_REMOTE_CONFIGURATION_ENABLED=false -e DD_INSTRUMENTATION_TELEMETRY_ENABLED=false \
  -e DD_INVENTORIES_ENABLED=false -e DD_SBOM_ENABLED=false \
  -p 8126:8126 -p 8125:8125/udp datadog/agent:7
```

In Kubernetes, the same Agent as a DaemonSet connects a whole cluster:
silo01's `datadog_agent` role is one (container logs by namespace, kubelet
and kube-state metrics, unified service tagging labels, OTLP), and what
robine-dog runs against in production.

Applications then talk to the Agent as usual (APM on 8126, DogStatsD on 8125);
it forwards traces, series, sketches and logs to robine-dog. Endpoints
robine-dog doesn't use (`/intake/`, check runs, metadata…) are acknowledged.

**OpenTelemetry** — OTLP/HTTP (protobuf or JSON) on the main port, mapped
as Datadog's OTLP ingestion maps it (`service.name`, `deployment.environment.name`,
`service.version`, `host.name`; operation and resource names from the semantic
conventions; exceptions become `error.*`; cumulative counters become deltas):

```sh
OTEL_EXPORTER_OTLP_PROTOCOL=http/protobuf \
OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:8080 \
OTEL_EXPORTER_OTLP_HEADERS="dd-api-key=$ROBINE_DOG_API_KEY" \
OTEL_SERVICE_NAME=my-api OTEL_RESOURCE_ATTRIBUTES=deployment.environment.name=dev \
opentelemetry-instrument python app.py
```

Traces, logs and metrics go to `/v1/traces`, `/v1/logs` and `/v1/metrics`.

**Errors from a Sentry SDK** — keep the SDK, change the DSN; the path names
the service, the key is `ROBINE_DOG_CLIENT_TOKEN` (or the API key):

```python
sentry_sdk.init(dsn="http://<client token>@localhost:8080/sentry/my-api/1",
                environment="dev", release="my-api@1.4.2")
```

Each error is an error log (`source` is the SDK's platform) and an issue in
Error Tracking; `environment` and `release` are `env` and `version`. With a
plain DSN (`…@host/1`) the service is the event's `service` tag, or
`sentry-<project>`. Transactions, sessions and attachments are dropped.

**Log pipelines** — raw lines (nginx, PostgreSQL…) are parsed by the
pipelines of `/logs/pipelines`, in Datadog's grok syntax; Terraform's
`datadog_logs_custom_pipeline` and the API clients work against
`/api/v1/logs/config/pipelines`.

## Configuration

All in the environment (see [`.env.example`](.env.example)): listen addresses
(`ROBINE_DOG_DOGSTATSD_LISTEN=off` disables DogStatsD),
`ROBINE_DOG_GREPTIME_URL` / `_DB` (optional `_USER` / `_PASSWORD`),
`ROBINE_DOG_POSTGRES_URL`, `ROBINE_DOG_CLIENT_TOKEN` (the key of Sentry DSNs),
`ROBINE_DOG_RETENTION_DAYS` (logs and spans,
default 15) and `ROBINE_DOG_METRICS_RETENTION_DAYS` (default 455, i.e. 15
months) applied as table TTLs, `ROBINE_DOG_API_KEY`,
`ROBINE_DOG_TRACE_REQUIRE_KEY`, the OIDC client, `ROBINE_DOG_SESSION_SECRET`,
`ROBINE_DOG_LOG_FORMAT=json` (robine-dog's own logs, one JSON object per line),
`ROBINE_DOG_SMTP_URL` (e-mail notifications, e.g.
`smtps://user:password@smtp.example.com` or `smtp://localhost:1025`) and
`ROBINE_DOG_SMTP_FROM` (default `robine-dog <robine-dog@localhost>`),
`ROBINE_DOG_CHROMIUM` (the browser of browser tests, found in the `PATH`
otherwise; the image has one), `ROBINE_DOG_CHROMIUM_ARGS` (extra flags,
`--no-sandbox` in the image) and `ROBINE_DOG_REPLAY_SAMPLE_RATE` (the share
of robine-dog's own UI sessions recorded by Session Replay, 0 to 100, default
0; with `ROBINE_DOG_CLIENT_TOKEN` set), `ROBINE_DOG_ARCHIVE_FLUSH_SECONDS`
(how often log archives are written, default 300) and the S3 credentials of
archives, `AWS_ACCESS_KEY_ID` and `AWS_SECRET_ACCESS_KEY` (or the instance's
role), and `ROBINE_DOG_PROFILE_RETENTION_DAYS` (how long profiles are kept,
default 8).

Rows are written in batches (every 200 ms or 5,000 rows) and metric buckets
every 10 seconds; on SIGTERM both are flushed before exiting.

## Layout

```text
crates/robine-dog/src
  main.rs            listeners, routes, graceful shutdown
  store/             GreptimeDB (HTTP SQL, schema, batched writers), PostgreSQL (migrations)
  query/             Datadog search syntax: parser → GreptimeDB SQL
  intake/logs.rs     /api/v2/logs (remapping, JSON messages, tags, trace ids)
  intake/traces.rs   trace-agent API (v0.4/v0.5 msgpack), service entry spans, trace metrics
  intake/series.rs   /api/v1/series, /api/v2/series
  intake/sentry.rs   Sentry SDK envelopes and events → error logs
  intake/agent.rs    Datadog Agent protobuf: traces (v0.2), series, sketches
  hosts.rs           host tags and aliases from the Agent's metadata, applied at ingestion
  pipelines/         log pipelines: processors, grok engine, user-agent parser
  monitors/          monitor queries, evaluation loop, message templates, webhooks
  metrics/           DogStatsD, 10 s aggregator, metric query language → SQL
  api/               UI API: logs, spans, facets, APM, metrics, pipelines, analytics, patterns
  auth.rs            Robine ID OIDC + session
crates/robine-dog-demo  demo traffic generator
web/src                React UI (pages/, components/, lib/)
```

`make check` runs rustfmt, clippy, the tests and the TypeScript check.
