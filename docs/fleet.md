---
title: Fleet Automation
section: Infrastructure
order: 40
app: /fleet
description: A read-only inventory of every Datadog Agent reporting to Robine Dog, with its version, OS, install method, configuration and running checks.
---

# Fleet Automation

With Agents on many hosts, you need to know which version runs where, how it
was installed and which integrations each one runs. Fleet Automation lists
every Agent that reports to Robine Dog, from the inventory payloads the
Agent sends about itself.

In Robine Dog, Fleet Automation is **read only**: upgrading Agents or
changing their configuration remotely needs Datadog's Remote Configuration,
which Robine Dog does not provide.

## Where the data comes from

| Payload | Endpoint | Gives |
| --- | --- | --- |
| Inventories: `agent_metadata`, `check_metadata`, `host_metadata` | `POST /api/v1/metadata` | Agent version, install method, configuration, the checks it runs |
| Host metadata | `POST /intake/` | Host name, Agent version, OS, host tags (see [host tags](infrastructure.md#host-tags)) |

Both arrive on port 8080 with the `DD-API-KEY` header, from an Agent whose
`dd_url` points at Robine Dog (see [Datadog Agent](agent.md)). For each
host, the latest payload of each kind replaces the previous one; they are
stored in PostgreSQL.

> **Note:** inventories are on by default in the Agent. An Agent started
> with `DD_INVENTORIES_ENABLED=false` sends no inventory, and only appears
> through its host metadata (version and OS, no checks).

The Agent's configuration is shown when its inventory includes it
(`full_configuration`, sent when `inventories_configuration_enabled` is
on); otherwise the panel shows the raw `agent_metadata`.

## The Fleet Automation page

[Fleet Automation](/fleet) starts with a count of Agents per version
(`3 on 7.60.0 · 1 on 7.58.2`), then lists every Agent by host name:

| Column | Meaning |
| --- | --- |
| Host | The Agent's host name |
| Version | From `agent_metadata.agent_version`, else from the host metadata |
| OS | From the host metadata |
| Install method | `agent_metadata.install_method_tool` (`docker`, `helm`, `install_script`…) |
| Checks | The checks in `check_metadata`; the first 8, then a count of the others |
| Last inventory | When the latest inventory arrived, or **host metadata only** |

Click a row to show the Agent's configuration (or its metadata as JSON).

## Differences from Datadog

- Read only: no Agent upgrades, no remote configuration changes and no
  configuration flares.
- Agents are listed by host name; there is no search, no filter by tag and
  no per-integration status (OK, warning, error) beyond the list of checks.
- Agents that stopped reporting stay listed with their last inventory time.
