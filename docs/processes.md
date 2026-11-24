---
title: Live Processes
section: Infrastructure
order: 20
app: /processes
description: The processes running on each host right now, with their CPU, memory, threads and command line, from the Datadog Agent's process collection.
---

# Live Processes

Host metrics tell you a machine is busy; Live Processes tells you which
program is. The Datadog Agent's process agent takes a snapshot of every
process on the host at a regular interval and sends it to Robine Dog, which shows
the latest snapshot of each host, the busiest processes first.

## Setup

Turn on process collection in the Agent and point the process agent at
Robine Dog's port 8080:

```sh
docker run -d --name dd-agent \
  -e DD_API_KEY=$ROBINE_DOG_API_KEY \
  -e DD_SITE=robine-dog.invalid \
  -e DD_DD_URL=http://robine-dog:8080 \
  -e DD_PROCESS_CONFIG_PROCESS_COLLECTION_ENABLED=true \
  -e DD_PROCESS_CONFIG_PROCESS_DD_URL=http://robine-dog:8080 \
  -v /var/run/docker.sock:/var/run/docker.sock:ro \
  -v /proc/:/host/proc/:ro -v /sys/fs/cgroup/:/host/sys/fs/cgroup:ro \
  datadog/agent:7
```

Or in `datadog.yaml`:

```yaml
process_config:
  process_collection:
    enabled: true
  process_dd_url: http://robine-dog:8080
```

The process agent posts to `POST /api/v1/collector` with the
`DD-API-KEY` header. Robine Dog reads the process agent's binary format: a
16-byte header, then protobuf, raw or zstd-compressed. A large snapshot sent
in several groups is put back together. The same endpoint also receives the
system-probe's network connections; see [Network monitoring](network.md).
Other process agent messages (container and real-time stats) are accepted
and ignored.

The [Agent setup page](/organization-settings/agent) gives these settings
filled in with your Robine Dog's address.

## The Live Processes page

[Live Processes](/processes) refreshes every 10 seconds. It lists the
processes of the hosts that sent a snapshot in the last two minutes, sorted
by CPU then resident memory, 1,000 at most, with the total count.

| Column | Meaning |
| --- | --- |
| Host | The host of the process |
| PID | Process id |
| User | The user running it |
| Command | The full command line (or the executable when there are no arguments); hover for all of it |
| CPU | Total CPU percentage |
| RSS | Resident memory |
| Threads | Number of threads |
| FDs | Open file descriptors |

| Control | Effect |
| --- | --- |
| Host selector | One host, or all hosts |
| Filter by command, user or PID | Keeps processes whose command line or user contains the text, or whose PID is the number |

## Retention

Snapshots are kept **in memory only**: each host's latest snapshot replaces
the previous one, a host that stops sending disappears after two minutes, and
the page is empty after a restart until the next snapshots arrive. There is
no process history.

## Differences from Datadog

- No history: you cannot look at the processes of an earlier time.
- No process-based metrics, process monitors or grouping by tags; for
  monitoring a process over time, use the Agent's `process` check, whose
  metrics land in the [Metrics Explorer](metrics.md).
- Command-line arguments are shown as sent; the Agent's own scrubbing of
  sensitive arguments is the only one applied.
