---
title: Network monitoring
section: Infrastructure
order: 30
app: /network
description: Cloud Network Monitoring shows the traffic between hosts from the system-probe's connections; Network Devices lists the routers and switches the Agent's SNMP integration discovers.
---

# Network monitoring

Two views cover the network:

- **Cloud Network Monitoring** shows who talks to whom: the TCP and UDP
  flows between your hosts and the outside, with the bytes sent each way,
  retransmits and round-trip time. The data comes from the Agent's
  system-probe, which watches connections in the kernel.
- **Network Devices** lists the routers, switches and other SNMP devices
  the Agent monitors, with their interfaces and whether they are reachable.

## Cloud Network Monitoring

### Setup

Turn on network monitoring in the Agent (it runs the system-probe) and point
the process agent at Robine Dog, since connections travel with it:

```yaml
# system-probe.yaml
network_config:
  enabled: true
```

```yaml
# datadog.yaml
process_config:
  process_dd_url: http://robine-dog:8080
```

The process agent sends the connections (`CollectorConnections` messages)
to `POST /api/v1/collector` or `POST /api/v1/connections` on port 8080,
with the `DD-API-KEY` header. See [Live Processes](processes.md) for the
format and [Datadog Agent](agent.md) for the rest of the Agent's
configuration.

### How flows are built

Each connections message is summed per minute into **flows**, one per:

| Key | Meaning |
| --- | --- |
| Host | The host the system-probe runs on |
| Local and remote IP | The two ends |
| Port | The server's port: the local port for an incoming connection, the remote port otherwise |
| Protocol | `tcp` or `udp` |
| Direction | `incoming`, `outgoing` or `local` |

For each flow, Robine Dog keeps the bytes sent and received, the
retransmits, the sum of round-trip times and the number of connections.
Every local address a host reports (other than `127.*`) is remembered as
that host's, so a remote address that belongs to another monitored host is
shown by that host's name.

### The Cloud Network page

[Cloud Network](/network) opens on the last hour and refreshes every
minute. It lists the 1,000 flows with the most traffic in the range, local
traffic excluded, oriented from client to server:

| Column | Meaning |
| --- | --- |
| Source | The client: the host or address that opened the connection |
| Destination | The server: a host name when its address is known, else the IP |
| Port | Protocol and server port, such as `tcp/5432` |
| Sent, Received | Bytes, from the reporting host's point of view |
| Retransmits | TCP retransmits |
| RTT | Average round-trip time, in milliseconds |
| Connections | The most connections seen in one minute |

Use **Filter by host, address or port** to narrow the list.

### Retention

Flows are stored in PostgreSQL and kept **3 days**; an hourly job deletes
older ones.

## Network Devices

### Setup

Configure the Agent's `snmp` integration, by autodiscovery over a subnet or
with instances, as Datadog documents it:

```yaml
# conf.d/snmp.d/conf.yaml
init_config:
instances:
  - ip_address: 10.0.0.1
    community_string: public
    tags:
      - site:paris
```

The Agent sends device and interface metadata to `POST /api/v2/ndm` on
port 8080 (JSON, optionally compressed, with `DD-API-KEY`); point its
network devices metadata endpoint at Robine Dog as you did for its other
endpoints. The polled values are regular `snmp.*` metrics, sent with the
Agent's other metrics and available in the [Metrics Explorer](metrics.md).

### The Network Devices page

[Network Devices](/devices) lists every device the Agent described:

| Column | Meaning |
| --- | --- |
| Device | The device name (or its id) |
| IP | Its management address |
| Vendor / model | From its SNMP profile |
| OS | OS name and version |
| Location | The SNMP location |
| Interfaces | How many interfaces it has |
| Status | **reachable** or **unreachable**, as the Agent last saw it |
| Last seen | When its metadata last arrived |

Click a device to list its interfaces: name, MAC address, alias or
description, and their admin and operational status.

Devices and interfaces are stored in PostgreSQL, the latest metadata of
each replacing the previous one. They also appear in the
[Resource Catalog](infrastructure.md#resource-catalog).

## Differences from Datadog

- Cloud Network Monitoring shows a table of flows: there is no network map,
  no grouping by service, container or tag, and no DNS monitoring.
- Remote addresses are resolved to hosts only when another monitored host
  reported that address; there is no cloud or domain resolution.
- Network Devices shows inventory and status; there is no device topology,
  NetFlow, or SNMP trap collection.
