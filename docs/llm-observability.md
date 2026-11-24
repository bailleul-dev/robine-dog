---
title: LLM Observability
section: LLM Observability
order: 10
app: /llm/overview
description: Traces of your LLM applications from ddtrace's LLM Observability SDK, with prompts, answers, tokens, estimated cost per model and evaluations, plus ml_obs.* metrics.
---

# LLM Observability

LLM Observability follows the requests of your LLM applications from end
to end: the workflow, the documents retrieved, every model call with its
prompt and answer, the tools called. Robine Dog receives the spans of
ddtrace's LLM Observability SDK (the automatic OpenAI, Anthropic, LangChain…
integrations and your own annotated functions), and shows:

- [Overview](/llm/overview): traces, errors, tokens, estimated cost, latency, models and evaluations of an application.
- [Traces](/llm/traces): each trace with its spans, prompts and answers.
- [Experiments](/llm/experiments) and [Agent Observability](/llm/agents), described in [Experiments and agents](llm-experiments.md).

## Set up the SDK

Enable LLM Observability in ddtrace and name your application with
`DD_LLMOBS_ML_APP`. Robine Dog accepts the spans through its trace port,
as a Datadog Agent would, or agentless on its HTTP port.

Through the trace port (the Agent's EVP proxy, which Robine Dog serves on
8126):

```sh
DD_LLMOBS_ENABLED=1 \
DD_LLMOBS_ML_APP=support-bot \
DD_SERVICE=support-bot DD_ENV=prod \
DD_TRACE_AGENT_URL=http://localhost:8126 \
DD_API_KEY=$ROBINE_DOG_API_KEY \
ddtrace-run python app.py
```

Agentless, straight to the HTTP port, with ddtrace's
`DD_LLMOBS_OVERRIDE_ORIGIN` in place of a Datadog site:

```sh
DD_LLMOBS_ENABLED=1 \
DD_LLMOBS_ML_APP=support-bot \
DD_LLMOBS_AGENTLESS_ENABLED=1 \
DD_LLMOBS_OVERRIDE_ORIGIN=http://localhost:8080 \
DD_API_KEY=$ROBINE_DOG_API_KEY \
ddtrace-run python app.py
```

| Endpoint | Receives |
| --- | --- |
| `POST /api/v2/llmobs`, `POST /api/intake/llm-obs/v1/trace/spans` | Spans (JSON, gzipped or not) |
| `POST /api/intake/llm-obs/v1/eval-metric`, `/v2/eval-metric` | Evaluations |
| `/evp_proxy/v2/…`, `/evp_proxy/v4/…` on port 8126 | The same, through the EVP proxy |

The HTTP endpoints need `DD-API-KEY`; the trace port takes what it is
given unless `ROBINE_DOG_TRACE_REQUIRE_KEY` is set (see
[Configuration](configuration.md)).

## Spans

Each span has a **kind**, set by the SDK's integrations or by the
decorator you use (`@workflow`, `@agent`, `@tool`, `@task`, `@retrieval`,
`@embedding`, `@llm`):

| Kind | Typically |
| --- | --- |
| `workflow` | A request handled end to end (the default when a span has no kind) |
| `agent` | An agent's run; see [Agent Observability](llm-experiments.md#agent-observability) |
| `llm` | One call to a model, with its messages, model name, provider and tokens |
| `embedding` | A call to an embedding model |
| `retrieval` | A search that returns documents |
| `tool` | A function an agent or a model calls |
| `task` | Any other step |

Robine Dog keeps, for every span: its trace, parent, name, kind, start and
duration, error status, `ml_app`, model name and provider, input, output
and total tokens, estimated cost, session (`session_id`), `service` and
`env` tags, its other tags, and its input and output (messages as
`role: content`, a value, or documents). A span received twice (same trace
and span id) is stored once.

The `ml_app` is the span's `ml_app:` tag, or the payload's `ml_app`, or
`unknown`.

## Estimated cost

For `llm` and `embedding` spans with a model name and token counts,
Robine Dog estimates the cost in USD from a built-in price list, per
million tokens. The model name is matched by prefix, case-insensitively,
after the last `/` (so `anthropic/claude-sonnet-4-20250514` matches
`claude-sonnet`); the most specific prefix wins.

| Model prefix | Input | Output |
| --- | --- | --- |
| `gpt-4o-mini` / `gpt-4o` | 0.15 / 2.50 | 0.60 / 10.00 |
| `gpt-4.1-nano` / `gpt-4.1-mini` / `gpt-4.1` | 0.10 / 0.40 / 2.00 | 0.40 / 1.60 / 8.00 |
| `gpt-4-turbo` / `gpt-4` / `gpt-3.5-turbo` | 10.00 / 30.00 / 0.50 | 30.00 / 60.00 / 1.50 |
| `o1-mini`, `o3-mini`, `o4-mini` | 1.10 | 4.40 |
| `o1` / `o3` | 15.00 / 2.00 | 60.00 / 8.00 |
| `claude-3-haiku` / `claude-3-5-haiku` / `claude-haiku` | 0.25 / 0.80 / 1.00 | 1.25 / 4.00 / 5.00 |
| `claude-3-opus`, `claude-opus` | 15.00 | 75.00 |
| `claude-3-5-sonnet`, `claude-3-7-sonnet`, `claude-sonnet` | 3.00 | 15.00 |
| `gemini-1.5-flash` / `gemini-1.5-pro` | 0.075 / 1.25 | 0.30 / 5.00 |
| `gemini-2.0-flash` / `gemini-2.5-flash` / `gemini-2.5-pro` | 0.10 / 0.30 / 1.25 | 0.40 / 2.50 / 10.00 |
| `mistral-large` / `mistral-small` | 2.00 / 0.20 | 6.00 / 0.60 |
| `text-embedding-3-small` / `text-embedding-3-large` | 0.02 / 0.13 | 0 |

A model that matches no prefix has no cost. The list is part of Robine
Dog and cannot be changed at run time.

## The Overview page

For the selected time range (the last hour by default) and application
(the selector lists every `ml_app` with its number of traces):

| Part | Shows |
| --- | --- |
| Tiles | Traces (root spans), share of traces in error, LLM calls, input and output tokens, estimated cost, p50 and p95 trace latency |
| Charts | Traces and errors, tokens, estimated cost over time |
| Models | Per model and provider (`llm` and `embedding` spans): calls, errors, input and output tokens, p95 latency, estimated cost |
| Evaluations | Per evaluation label: type, count, average score, most frequent value, and how many were *flagged* (a categorical value of `false`, `negative`, `fail`, `failed`, `bad`, `toxic` or `yes`) |

## The Traces page

[Traces](/llm/traces) lists the latest 200 traces (root spans) of the
range, with their input, output, duration, tokens, estimated cost and
evaluations. Tokens and cost are summed over the whole trace.

- Search text in span names, inputs and outputs.
- Pick a kind in **Spans** to list spans of that kind (all `llm` calls, for example) instead of traces.
- Check **Errors only** to keep spans in error.

Click a row to open the trace: its spans as a tree, and for the selected
span its duration, model, tokens, estimated cost, session, tags, error,
input and output messages (or retrieved documents with their score), and
metadata.

## Evaluations

Attach a judgement to a span, from your code with ddtrace's
`LLMObs.submit_evaluation()` or with the API:

```sh
curl -X POST http://localhost:8080/api/intake/llm-obs/v2/eval-metric \
  -H "DD-API-KEY: $ROBINE_DOG_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"data": {"type": "evaluation_metric", "attributes": {"metrics": [
        {"join_on": {"span": {"span_id": "5122183711230961283", "trace_id": "6930cbb2000000004f8c6a2f8e1d3b77"}},
         "ml_app": "support-bot", "label": "faithfulness", "metric_type": "score",
         "score_value": 0.82, "timestamp_ms": 1764000000000, "tags": ["env:prod"]}]}}}'
```

| Field | Meaning |
| --- | --- |
| `join_on.span.span_id`, `join_on.span.trace_id` | The span evaluated (v1: `span_id`, `trace_id` at the top) |
| `label` | The evaluation's name, required |
| `metric_type` | `score`, `categorical`, `boolean`… shown as given |
| `score_value`, `categorical_value`, `boolean_value` | The value; a boolean is stored as the category `true` or `false` |
| `ml_app` | Or an `ml_app:` tag; else `unknown` |
| `timestamp_ms` | Defaults to the time received |

## Metrics

Every span and evaluation also produces metrics, for
[dashboards](dashboards.md) and [monitors](monitors.md):

| Metric | Type | Counts |
| --- | --- | --- |
| `ml_obs.span` | count | Spans |
| `ml_obs.span.duration` | distribution (seconds) | Span durations |
| `ml_obs.span.error` | count | Spans in error |
| `ml_obs.trace`, `ml_obs.trace.duration` | count, distribution (seconds) | Root spans |
| `ml_obs.span.llm.input.tokens`, `.output.tokens`, `.total.tokens` | count | Tokens |
| `ml_obs.span.llm.estimated_cost` | count | Estimated cost, in USD |
| `ml_obs.evaluation` | count | Evaluations, tagged `ml_app`, `label` and `value` (categorical) |
| `ml_obs.evaluation.score` | distribution | Score values |

Span metrics are tagged `ml_app`, `span_kind`, `error` (`0` or `1`) and,
when known, `model_name`, `model_provider`, `service` and `env`. For
example, the daily spend per model:

```text
sum:ml_obs.span.llm.estimated_cost{ml_app:support-bot} by {model_name}.as_count().rollup(sum, 86400)
```

## Retention

Spans and evaluations are kept `ROBINE_DOG_RETENTION_DAYS` days (15 by
default), like logs and spans. The `ml_obs.*` metrics follow the metrics
retention.

## Differences from Datadog

- Costs are estimated from Robine Dog's own price list, in USD; there is no per-organization pricing.
- Managed evaluations (hallucination, toxicity, prompt injection…), sensitive data scanning of prompts, clusters and the session view are not implemented: evaluations are those you submit.
- Search is a substring match on names, inputs and outputs, not the [search syntax](search-syntax.md).
