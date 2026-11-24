---
title: Hosts, containers and Kubernetes
section: Infrastructure
order: 10
app: /infrastructure
description: Every host, container and Kubernetes resource the Datadog Agent reports, with its latest CPU, memory and health, a host map, the Resource Catalog, and the host tags joined to everything a host sends.
---

# Hosts, containers and Kubernetes

The Infrastructure pages list what your Datadog Agents run on: hosts,
containers, and the pods, deployments, stateful sets, daemon sets, nodes and
namespaces of a Kubernetes cluster. Each list is built from the metrics the
Agent already sends (`system.*`, `container.*`, `kubernetes.*`,
`kubernetes_state.*`): there is nothing to set up beyond
[installing the Agent](agent.md) against Robine Dog.

## Where the data comes from

| Page | Built from | Agent feature |
| --- | --- | --- |
| Hosts, Host Map | `system.*` metrics, by `host` | The Agent's core checks |
| Containers | `container.*` metrics, by `kube_namespace`, `pod_name`, `kube_container_name` | Container collection (Docker socket or containerd) |
| Kubernetes | `kubernetes.*` (kubelet) and `kubernetes_state.*` (kube-state-metrics core check) | The Agent in a cluster, with the Cluster Agent |
| Host tags and aliases | The Agent's metadata payload (`POST /intake/`) | Always sent |

A row appears as soon as the metrics it needs carry the identifying tags; a
container metric without `pod_name`, for example, does not make a container
row.

## Host tags

The Agent regularly describes its host in a metadata payload: its
name, its **host tags** (from `DD_TAGS`, `env`, the cloud provider,
`kube_cluster_name`…) and the **aliases** of its name (a Kubernetes node is
also known as `<node>-<cluster>`). Robine Dog stores them and, as Datadog
does, applies them at ingestion to everything that host sends:

- data sent under an alias is filed under the host's real name;
- the host tags are added to its logs, metrics (series and distributions),
  events and Sentry errors, unless the data already has a value for that tag
  key.

So `env:prod` set once in the Agent's `DD_TAGS` filters that host's logs and
metrics without any change to the applications. Tags set by the data itself
win: a log tagged `env:staging` keeps it.

Host tags apply from the moment the payload arrives; data received before is
not retagged. Hosts are kept in PostgreSQL across restarts.

## Hosts

[Hosts](/infrastructure) lists every host that reported `system.*` metrics in
the last 10 minutes, with the latest value of each column. The list
refreshes every 30 seconds.

| Column | Metric | Highlighted when |
| --- | --- | --- |
| CPU | `100 - avg:system.cpu.idle` | Above 90% |
| Load 1 | `avg:system.load.1` | |
| Memory | `(1 - avg:system.mem.pct_usable) * 100` | Above 90% |
| Disk (fullest) | `max:system.disk.in_use` | Above 85% |
| I/O wait | `avg:system.cpu.iowait` | |
| Uptime | `avg:system.uptime` | |

Filter the list by name or any of its values with the text box, and sort by
any column. Click a host to open its panel:

- its tags, Agent version and OS;
- **View logs** (`host:<name>`) and **Open in Metrics Explorer**;
- graphs of the last hour: CPU usage, load, memory used, disk in use per
  device, network received and sent;
- its **host tags** and **aliases** from the Agent's metadata.

## Host Map

[Host Map](/infrastructure/map) draws each host as a tile colored by a
metric, over the last 15 minutes:

| Control | Effect |
| --- | --- |
| Fill by | CPU user %, Load (normalized), Usable memory %, Disk in use or Network in |
| Filter | Tags that hosts must have, such as `env:prod` |
| Group by | One or more tag keys; hosts are boxed by their values |

The same map is available as the host map widget on dashboards.

## Containers

[Containers](/containers) has one row per Kubernetes container
(namespace, pod, container), with its image and tag:

| Column | Metric |
| --- | --- |
| CPU | `container.cpu.usage`, in cores |
| Memory, Memory limit | `container.memory.usage`, `container.memory.limit` |
| Restarts | `kubernetes.containers.restarts` (highlighted above 5) |
| Threads | `container.pid.thread_count` |
| Uptime | `container.uptime` |

The panel graphs CPU, CPU throttled, memory, network and disk writes, and
**View logs** searches `pod_name:<pod> container_name:<container>`.

## Kubernetes

[Kubernetes](/orchestration/overview/pod) has a tab per kind of resource:

| Kind | Identified by | Columns |
| --- | --- | --- |
| Pods | `kube_namespace`, `pod_name` | Phase and node, Ready, Restarts, CPU, Memory, Age; a phase other than Running or Succeeded is highlighted |
| Deployments | `kube_namespace`, `kube_deployment` | Desired, Available, Unavailable (highlighted above 0), Updated, CPU, Memory |
| StatefulSets | `kube_namespace`, `kube_stateful_set` | Desired, Ready, CPU, Memory |
| DaemonSets | `kube_namespace`, `kube_daemon_set` | Desired, Ready, Unavailable |
| Nodes | `node` | Ready, Pods, CPU and memory allocatable, Age |
| Namespaces | `kube_namespace` | Pods, Deployments, Restarts, CPU, Memory, Volumes used |

Replica counts come from `kubernetes_state.*`; CPU and memory from the
kubelet's `kubernetes.cpu.usage.total` and `kubernetes.memory.usage`. Click a
resource for its graphs and logs.

## Resource Catalog

[Resource Catalog](/resources) puts every resource seen in the last hour in
one list, with its tags: hosts (from the Agent's metadata), containers,
pods, deployments and nodes (from their metrics), and
[network devices](network.md#network-devices). Filter it by name or tag;
click a resource to open its page.

## API

| Method and path | Returns |
| --- | --- |
| `GET /api/v1/hosts` | Datadog's host list: name, aliases, tags (`tags_by_source`), Agent version and platform; `filter=` keeps hosts whose description contains the text |
| `GET /api/v1/hosts/totals` | `total_active` and `total_up` |

```sh
curl "http://localhost:8080/api/v1/hosts?filter=env:prod" \
  -H "DD-API-KEY: $ROBINE_DOG_API_KEY"
```

## Differences from Datadog

- Hosts are known from their metrics and metadata only: there is no host
  muting, no cloud integration inventory and no "Serverless" view.
- The Kubernetes pages show metrics, not the cluster's manifests: the
  orchestrator explorer's YAML and events are not available.
- Every host in `GET /api/v1/hosts` is reported as up, with no metrics
  summary.
