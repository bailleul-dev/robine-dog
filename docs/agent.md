---
title: Datadog Agent
section: Send Data
order: 10
app: /organization-settings/agent
description: Run the unmodified Datadog Agent against Robine Dog to collect host, container and Kubernetes metrics, logs and traces, with every intake URL pointed at Robine Dog.
---

# Datadog Agent

Robine Dog accepts what the Datadog Agent sends to Datadog. You install the
official Agent (`datadog/agent:7`, the Helm chart or the host package) and
override its intake URLs so that every payload goes to Robine Dog instead.
Your applications keep talking to the Agent as usual: traces on port 8126,
DogStatsD on 8125, log files and container output collected by the Agent.

The [Agent](/organization-settings/agent) page of the interface shows the
three configurations below with your instance's address already filled in.

## Why `DD_SITE=robine-dog.invalid`

The Agent builds the URL of every intake it doesn't have an override for
from its site (`datadoghq.com` by default). The configurations below set
`DD_SITE=robine-dog.invalid`, a name that never resolves (`.invalid` is
reserved), so any endpoint that is not redirected to Robine Dog fails
instead of sending your data, and your key, to Datadog. Remote
configuration is disabled for the same reason.

## Install

Create an API key first, in [API Keys](/organization-settings/api-keys), or
use `ROBINE_DOG_API_KEY`. See [API and application keys](api-keys.md).

> **Note:** the Agent must reach Robine Dog at the address you give it.
> From a container, `localhost` is the container itself: use a host name or
> address the container can resolve.

### Docker

```sh
docker run -d --name dd-agent \
  -e DD_API_KEY=$ROBINE_DOG_API_KEY \
  -e DD_SITE=robine-dog.invalid \
  -e DD_DD_URL=http://localhost:8080 \
  -e DD_APM_DD_URL=http://localhost:8080 \
  -e DD_LOGS_ENABLED=true \
  -e DD_LOGS_CONFIG_USE_HTTP=true \
  -e DD_LOGS_CONFIG_LOGS_DD_URL=localhost:8080 \
  -e DD_LOGS_CONFIG_LOGS_NO_SSL=true \
  -e DD_PROCESS_CONFIG_PROCESS_DD_URL=http://localhost:8080 \
  -e DD_PROCESS_CONFIG_PROCESS_COLLECTION_ENABLED=true \
  -e DD_REMOTE_CONFIGURATION_ENABLED=false \
  -v /var/run/docker.sock:/var/run/docker.sock:ro \
  -v /proc/:/host/proc/:ro -v /sys/fs/cgroup/:/host/sys/fs/cgroup:ro \
  -p 8126:8126 -p 8125:8125/udp datadog/agent:7
```

`DD_LOGS_CONFIG_LOGS_NO_SSL=true` is for a plain-HTTP address; leave it out
when Robine Dog is served over HTTPS.

### Kubernetes (Helm)

Deploy the Agent as a DaemonSet with Datadog's Helm chart and these values:

```yaml
# helm repo add datadog https://helm.datadoghq.com
# helm install datadog-agent datadog/datadog -f values.yaml
datadog:
  apiKey: <API key>
  site: robine-dog.invalid
  dd_url: http://localhost:8080
  logs:
    enabled: true
    containerCollectAll: true
  apm:
    portEnabled: true
  processAgent:
    processCollection: true
  remoteConfiguration:
    enabled: false
  env:
    - name: DD_APM_DD_URL
      value: "http://localhost:8080"
    - name: DD_LOGS_CONFIG_USE_HTTP
      value: "true"
    - name: DD_LOGS_CONFIG_LOGS_DD_URL
      value: "localhost:8080"
    - name: DD_LOGS_CONFIG_LOGS_NO_SSL
      value: "true"
    - name: DD_PROCESS_CONFIG_PROCESS_DD_URL
      value: "http://localhost:8080"
    - name: DD_PROCESS_CONFIG_PROCESS_COLLECTION_ENABLED
      value: "true"
    - name: DD_REMOTE_CONFIGURATION_ENABLED
      value: "false"
clusterAgent:
  enabled: true
```

The Agent then collects every container's logs, the kubelet and
kube-state metrics, and receives traces on each node's port 8126.

### Host (`datadog.yaml`)

```yaml
# /etc/datadog-agent/datadog.yaml
api_key: <API key>
site: robine-dog.invalid
dd_url: http://localhost:8080
apm_config:
  apm_dd_url: http://localhost:8080
logs_enabled: true
logs_config:
  use_http: true
  logs_dd_url: localhost:8080
  logs_no_ssl: true
process_config:
  process_dd_url: http://localhost:8080
remote_configuration:
  enabled: false
```

## What the Agent forwards

Every endpoint below is on Robine Dog's main port (8080) and checks the
Agent's API key.

| Agent payload | Endpoint | What Robine Dog does with it |
| --- | --- | --- |
| Traces | `POST /api/v0.2/traces` | Decodes the protobuf `AgentPayload` (the classic and the string-table formats, zstd or gzip), puts the Agent's env, host name, application version and sampling priority back on the spans, then ingests them like [traces from a tracer](tracing.md) |
| Metrics | `POST /api/v2/series` (protobuf), `POST /api/v1/series` | Stores the points as sent; see [DogStatsD and the series API](dogstatsd.md) |
| Distributions | `POST /api/beta/sketches` | Turns each DDSketch bin into a weighted point, kept as a distribution |
| Logs | `POST /api/v2/logs` | The [logs intake](log-collection.md) |
| Host metadata | `POST /intake/` | Keeps the host tags, host aliases, Agent version and OS (see [host tags](tagging.md#host-tags-and-aliases)); Agent events in the payload become [events](events.md) |
| Check runs | `POST /api/v1/check_run` | Stored as service checks (`datadog.agent.up`, `*.can_connect`…), for [service check monitors](monitors.md) |
| Inventories | `POST /api/v1/metadata` | Agent version, install method, configuration and checks, shown in [Fleet Automation](fleet.md) |
| Live processes | `POST /api/v1/collector` | Each host's latest process snapshot, shown in [Processes](processes.md) |
| Network connections | `POST /api/v1/collector`, `POST /api/v1/connections` | Flows between hosts, shown in [Cloud Network Monitoring](network.md) |
| Network devices | `POST /api/v2/ndm` | SNMP devices and interfaces, in [Network Devices](network.md) |
| Database Monitoring | `POST /api/v2/dbmmetrics`, `POST /api/v2/databasequery` | Query metrics, samples, plans and sessions, in [Database Monitoring](database-monitoring.md) |

Compressed payloads (gzip, deflate, zstd) are accepted, up to 64 MiB per request.

### Acknowledged and ignored

Any other request under `/api/` with a valid key (for example APM stats on
`/api/v0.2/stats`) is answered `202 Accepted` so the Agent stays healthy,
and dropped. Robine Dog logs the first request
to each such endpoint:

```text
INFO Agent endpoint acknowledged but not used endpoint="POST /api/v0.2/stats"
```

Trace metrics are computed from the spans themselves, which is why the
Agent's APM stats are not needed.

## Optional features

| Feature | Agent configuration | Read |
| --- | --- | --- |
| Live processes | `DD_PROCESS_CONFIG_PROCESS_COLLECTION_ENABLED=true` and `DD_PROCESS_CONFIG_PROCESS_DD_URL` (in the configurations above) | [Processes](processes.md) |
| Cloud Network Monitoring | `network_config.enabled: true` (system-probe) | [Network](network.md) |
| Network Devices | The `snmp` integration | [Network](network.md) |
| Database Monitoring | `dbm: true` on the `postgres` or `mysql` check, and `database_monitoring.metrics.dd_url`, `database_monitoring.samples.dd_url`, `database_monitoring.activity.dd_url` pointing here | [Database Monitoring](database-monitoring.md) |
| Workload Protection and Cloud Security | `runtime_security_config` and `compliance_config` with `endpoints.logs_dd_url: localhost:8080` | [Workload Protection](workload-protection.md) |

To leave the process agent off, set
`DD_PROCESS_CONFIG_PROCESS_COLLECTION_ENABLED=false` and drop
`DD_PROCESS_CONFIG_PROCESS_DD_URL`.

## Check that it works

- [Infrastructure](/infrastructure) lists the host once its first
  `system.*` metrics arrive.
- [Metrics Summary](/metrics/summary) shows `system.*` metrics, and
  `container.*` and `kubernetes.*` ones on Kubernetes.
- Robine Dog's own log, at `info` level, shows a `host metadata` line with
  the host's tags and aliases each time they change.
- `GET /api/v1/validate` with the Agent's key answers `{"valid": true}`.

## Differences from Datadog

- Payloads sent to the configured URLs but not listed above, such as APM
  stats, are accepted and dropped.
- Products the Agent sends to other hosts of its site (derived from
  `DD_SITE`) receive nothing, since `robine-dog.invalid` does not resolve.
- Live processes are kept in memory: only each host's latest snapshot, lost
  on restart.
