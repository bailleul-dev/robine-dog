---
title: API and application keys
section: Getting Started
order: 30
app: /organization-settings/api-keys
description: The keys that authenticate the intakes, the Datadog-compatible API, the MCP server and the browser and Sentry SDKs, and how to create and revoke them.
---

# API and application keys

Robine Dog authenticates machines with keys, as Datadog does. People sign in
to the interface with Robine ID (see [Getting started](getting-started.md#sign-in));
everything else (Agents, tracers, SDKs, scripts, Terraform, the API clients)
sends a key.

| Key | Where it is set | What it is for |
| --- | --- | --- |
| The configured API key | `ROBINE_DOG_API_KEY` | Every intake and the public API. Also the only key the MCP server accepts |
| Named API keys | [API Keys](/organization-settings/api-keys) or `/api/v2/api_keys` | Every intake and the public API, like the configured key |
| Application keys | [API Keys](/organization-settings/api-keys) or `/api/v2/application_keys` | Recorded when a client sends them; not required |
| The client token | `ROBINE_DOG_CLIENT_TOKEN` | The browser SDK (RUM) and Sentry DSNs: public, safe to ship in a web page or a mobile app |

> **Warning:** when `ROBINE_DOG_API_KEY` is unset, every intake and the
> public API accept any key, or none, and named API keys make no difference.
> The server logs a warning at startup. Set it to a long random value on any
> instance that is reachable from a network.

## Sending a key

Send the API key the way Datadog's clients do:

| Form | Example |
| --- | --- |
| `DD-API-KEY` header | `-H "DD-API-KEY: $ROBINE_DOG_API_KEY"` |
| `dd-api-key` query parameter | `/api/v2/logs?dd-api-key=...` (webhooks, browser intakes) |
| `api_key` query parameter | `/api/v1/series?api_key=...` |

Check a key with the validation endpoint, as the Datadog clients do:

```sh
curl http://localhost:8080/api/v1/validate -H "DD-API-KEY: $ROBINE_DOG_API_KEY"
```

```json
{"valid": true}
```

A missing or unknown key gets `403` with `{"errors": ["Forbidden"]}`.

## Which endpoints need which key

| Endpoint | Accepts |
| --- | --- |
| Logs, series, sketches, events, check runs, Agent payloads, OTLP (`/v1/traces`, `/v1/logs`, `/v1/metrics`), profiles, CI, LLM Observability and the other intakes on port 8080 | The configured API key or a named API key |
| Datadog's public API (`/api/v1/...`, `/api/v2/...`: monitors, dashboards, pipelines, search…) | The configured API key or a named API key; an application key is optional |
| Trace agent API on port 8126 | No key, unless `ROBINE_DOG_TRACE_REQUIRE_KEY` is set (see below) |
| RUM (`/rum/proxy`, `/api/v2/rum`, `/api/v2/replay`) and Sentry (`/sentry/...`, `/api/<project>/envelope/`) | The client token or the configured API key |
| MCP server (`/v1/mcp`) | The configured API key only |
| The interface and its API (`/api/ui/...`) | A signed-in session; keys are not accepted |

> **Note:** named API keys are not accepted by the RUM and Sentry intakes or
> by the MCP server. Use the client token for the SDKs and
> `ROBINE_DOG_API_KEY` for MCP clients.

### The trace port

Port 8126 plays the role of a local Datadog Agent, which tracers reach
without a key. Keep it on a trusted network, as you would an Agent's port.
To require a key there, set:

```sh
ROBINE_DOG_TRACE_REQUIRE_KEY=true
```

Robine Dog then checks the `DD-API-KEY` header of every trace payload
(`1` also works). Query parameters are not read on this port. The ddtrace
libraries do not send a key to an agent, so with this setting your
applications report through a Datadog Agent (which sends its key to port
8080) or through a proxy that adds the header.

### The client token

The client token authenticates code that runs on devices you don't control,
where any key can be read. Set it in the environment:

```sh
ROBINE_DOG_CLIENT_TOKEN=pub0123456789abcdef
```

The RUM browser SDK sends it as `clientToken`, and a Sentry DSN carries it
as its public key: `http://<client token>@localhost:8080/sentry/my-api/1`.
When neither `ROBINE_DOG_CLIENT_TOKEN` nor `ROBINE_DOG_API_KEY` is set, these
intakes accept any key. With a client token set, Robine Dog's own interface
also reports its sessions to its RUM intake; see
[Configuration](configuration.md).

## Manage keys in the interface

Open [API Keys](/organization-settings/api-keys) (**Integrations > APIs**).
The page has two tables, **API keys** and **Application keys**, with each
key's name, last four characters, creation time and last use ("never" until
it is used).

- **Create**: click **New API key** or **New application key**, enter a
  name and click **Create**. The key is shown once; copy it before closing
  the dialog. Robine Dog stores only its SHA-256 hash and its last four
  characters.
- **Revoke**: click the trash icon on the key's row. The key stops working
  at once and the row is deleted.

API keys are 32 hexadecimal characters; application keys are 40.
`ROBINE_DOG_API_KEY` is not listed: it is configuration, and stays valid
whatever you do on this page.

Last use is tracked in memory as requests arrive and saved every minute.

## Application keys

Datadog requires an application key, in the `DD-APPLICATION-KEY` header,
for most of its API. Robine Dog does not: a valid API key is enough. When a
request to the public API carries a `DD-APPLICATION-KEY` that you created,
its last use is recorded, so you can see which tools still use which key.
Create one for clients that refuse to start without an application key, such
as the Terraform provider; see [API and MCP server](api.md).

## API

The key endpoints follow Datadog's `api_keys` and `application_keys` APIs.
They are part of the public API, so they need an API key themselves.

| Method and path | Does |
| --- | --- |
| `GET /api/v2/api_keys` | List API keys (never the keys themselves) |
| `POST /api/v2/api_keys` | Create one; the response holds `key`, once |
| `GET /api/v2/api_keys/{id}` | Get one |
| `PATCH /api/v2/api_keys/{id}` | Rename it |
| `DELETE /api/v2/api_keys/{id}` | Revoke it |
| `GET`, `POST /api/v2/application_keys` | List, create application keys (also `/api/v2/current_user/application_keys`) |
| `GET`, `PATCH`, `DELETE /api/v2/application_keys/{id}` | Get, rename, revoke one |

```sh
curl -X POST http://localhost:8080/api/v2/api_keys \
  -H "DD-API-KEY: $ROBINE_DOG_API_KEY" -H 'Content-Type: application/json' \
  -d '{"data": {"type": "api_keys", "attributes": {"name": "k8s-agent"}}}'
```

```json
{
  "data": {
    "type": "api_keys",
    "id": "q8ZPj3vXr0uC1yW2mD5aLQ",
    "attributes": {
      "name": "k8s-agent",
      "key": "3f9a6c0d8e7b4a21c5d0e9f8a7b6c5d4",
      "last4": "c5d4",
      "created_at": "2026-11-24T09:12:03.512Z",
      "modified_at": "2026-11-24T09:12:03.512Z",
      "last_used_at": null,
      "scopes": null
    }
  }
}
```

`name` is required. `scopes` is stored and returned as sent.

## Differences from Datadog

- There are no organizations, users or roles: a key belongs to the instance.
- Application keys are never required and their `scopes` are not enforced.
- Revoking a key deletes it; there is no disabled state.
- The configured `ROBINE_DOG_API_KEY` is a key outside the list, which you
  rotate by changing the environment and restarting.
