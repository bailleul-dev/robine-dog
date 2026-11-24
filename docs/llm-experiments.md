---
title: Experiments and agents
section: LLM Observability
order: 20
app: /llm/experiments
description: Datasets and experiments from ddtrace's LLM Observability SDK, compared by their evaluations, and Agent Observability for the agents your applications run.
---

# Experiments and agents

Two views build on [LLM Observability](llm-observability.md):

- **Experiments** run your application, or a variant of it, over a dataset of inputs and expected outputs, and score each answer with evaluators. Comparing experiments on the same dataset tells you whether a new prompt or model is better. Robine Dog implements the projects, datasets and experiments API that ddtrace's SDK calls, and compares the results in [Experiments](/llm/experiments).
- **Agent Observability** sums up the `agent` spans of your traces: which agents run, how often they fail, how many tool and model calls a run takes. It is in [Agent Observability](/llm/agents).

## Run an experiment

Configure ddtrace as for LLM Observability. The SDK sends its experiment
calls to the same place as its spans: through the trace port (the Agent's
EVP proxy on 8126), or agentless with `DD_LLMOBS_OVERRIDE_ORIGIN`:

```sh
export DD_LLMOBS_ENABLED=1 DD_LLMOBS_ML_APP=support-bot
export DD_LLMOBS_AGENTLESS_ENABLED=1 DD_LLMOBS_OVERRIDE_ORIGIN=http://localhost:8080
export DD_API_KEY=$ROBINE_DOG_API_KEY DD_APP_KEY=$ROBINE_DOG_APP_KEY
python experiment.py
```

```python
from ddtrace.llmobs import LLMObs

LLMObs.enable()

dataset = LLMObs.create_dataset(
    dataset_name="order-questions",
    description="Questions customers ask about their orders",
    records=[
        {"input_data": {"question": "Where is my order 1042?"},
         "expected_output": "It ships today."},
        {"input_data": {"question": "Can I change the address of order 1043?"},
         "expected_output": "Yes, until it ships."},
    ],
)

def answer(input_data, config):
    return my_bot(input_data["question"], model=config["model"])

def exact_match(input_data, output_data, expected_output):
    return output_data == expected_output

experiment = LLMObs.experiment(
    name="gpt-4o-mini-baseline",
    dataset=dataset,
    task=answer,
    evaluators=[exact_match],
    config={"model": "gpt-4o-mini"},
)
experiment.run()
```

Robine Dog checks the API key only; the application key is accepted and
not verified.

### The API

The SDK calls these paths under `/api/unstable/llm-obs/v1/` (also served
under `/api/v2/llm-obs/v1/`, and under `llm-obs/v1/` behind the EVP proxy):

| Method and path | Does |
| --- | --- |
| `POST projects` | Creates a project, or returns the one with that `name` |
| `GET projects?filter[name]=…` | Lists projects |
| `POST {project_id}/datasets` | Creates a dataset (version 0, no records) |
| `GET {project_id}/datasets?filter[name]=…` | Lists a project's datasets |
| `POST {project_id}/datasets/{id}/batch_update` | Inserts, updates and deletes records (`insert_records`, `update_records`, `delete_records`); a change bumps the dataset's version unless `create_new_version` is `false` |
| `GET {project_id}/datasets/{id}/records` | The dataset's records, in one page |
| `POST datasets/delete` | Deletes the datasets of `dataset_ids` |
| `POST experiments` | Creates an experiment (status `running`) with its `name`, `project_id`, `dataset_id`, `dataset_version`, `config`… |
| `GET experiments?filter[project_id]=…&filter[dataset_id]=…&filter[id]=…` | Lists experiments |
| `PATCH experiments/{id}` | Sets its `status` and `error` |
| `POST experiments/{id}/events` | Adds the run's `spans` (one per record) and evaluation `metrics` |
| `GET experiments/{id}/events` | The spans, evaluations and summary |

A record holds an `input`, an `expected_output`, `metadata` and `tags`.

## The Experiments page

[Experiments](/llm/experiments) groups experiments by dataset, newest
first. For each experiment:

| Column | Shows |
| --- | --- |
| Experiment | Its name and `config` |
| Status | `running` until the SDK marks it otherwise |
| Rows | Spans received, one per dataset record run |
| Errors | Rows in error |
| One column per evaluation | The summary over the rows: the mean for scores and booleans (a boolean counts as 1 or 0, so `exact_match` shows the share of matches), the most frequent value for categories |
| Created | When it started |

Experiments of a dataset share their evaluation columns, so you compare
them by reading down a column. Click an experiment to list its rows: the
input, the output (or the error), the expected output and each
evaluation's value.

## Agent Observability

[Agent Observability](/llm/agents) reads the spans of kind `agent`
(ddtrace's `@agent` decorator, `LLMObs.agent()`, or an agent framework
integration) in the time range (the last day by default):

| Part | Shows |
| --- | --- |
| Agents | Per agent name and application: runs, errors, average duration, and per run the average number of tool calls, LLM calls and LLM tokens (counted over the run's whole trace) |
| Tools | Per `tool` span name and application: calls, errors, average duration (top 200) |
| Recent runs | The latest 100 root `agent` spans, with duration and error |

Click an agent (it shows one of its runs) or a recent run to draw the run
as a tree of steps: each span under its parent, in order, with its kind,
name, duration, tokens and model; steps in error are marked, and hovering a
step shows the start of its output. For the full prompts and answers, open
the trace in [Traces](/llm/traces).

## Differences from Datadog

- Experiments are stored as sent; Robine Dog runs no evaluator itself, and custom evaluator configuration (`config/evaluators/custom`) is accepted and ignored.
- Dataset versions are counted, but only the current records are kept: earlier versions cannot be read back.
- Records come back in a single page, and datasets cannot be edited or imported (CSV) from the UI.
- There is no playground or prompt management.
