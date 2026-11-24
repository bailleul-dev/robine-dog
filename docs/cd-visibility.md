---
title: CD Visibility and DORA Metrics
section: Software Delivery
order: 20
app: /ci/deployments
description: Every deployment by service and env, from the DORA API, CD custom spans or GitHub and GitLab webhooks, and the four DORA metrics computed from them.
---

# CD Visibility and DORA Metrics

CD Visibility records your deployments: which version of which service
went to which environment, when, and whether it succeeded.
[DORA Metrics](/ci/dora) turns them into the four key metrics of software
delivery: deployment frequency, lead time for changes, change failure rate
and time to restore service.

## Send deployments

| Source | Endpoint | Recorded as |
| --- | --- | --- |
| The DORA API | `POST /api/v2/dora/deployment` | `dora` |
| GitHub `deployment_status` webhooks | `POST /api/v2/webhook` | `github` |
| GitLab Deployment Hook webhooks | `POST /api/v2/webhook` | `gitlab` |
| CD custom spans tagged `cd.*` | `POST /api/intake/ci/custom_spans` | `datadog-ci` |

All of them need an API key: the `DD-API-KEY` header, or `?dd-api-key=`
in a webhook URL.

### The DORA API

Call it at the end of your deployment job:

```sh
curl -X POST http://localhost:8080/api/v2/dora/deployment \
  -H "DD-API-KEY: $ROBINE_DOG_API_KEY" \
  -H "Content-Type: application/json" \
  -d "{\"data\": {\"attributes\": {
        \"service\": \"shop-api\", \"env\": \"prod\", \"version\": \"1.4.2\", \"team\": \"checkout\",
        \"started_at\": $STARTED_NS, \"finished_at\": $(date +%s%N),
        \"git\": {\"commit_sha\": \"$GITHUB_SHA\", \"repository_url\": \"https://github.com/acme/shop\"}}}}"
```

| Attribute | Meaning |
| --- | --- |
| `service` | Required |
| `finished_at` | Required. Seconds, milliseconds or nanoseconds since the epoch (Datadog sends nanoseconds), or an RFC 3339 time |
| `started_at` | Same formats; defaults to `finished_at` |
| `env` | Defaults to `none` |
| `version`, `team` | Shown on the pages; `team` also on DORA's per-service table |
| `git.commit_sha`, `git.repository_url` | The commit deployed: needed for the lead time |

A deployment sent with this API is a success. The answer is `202` with
the new deployment's `data.id`.

### GitHub and GitLab webhooks

Use the webhook you set up for [CI Visibility](ci-visibility.md#send-pipelines),
and add the deployment events:

- **GitHub**: *Deployment statuses*. A status `success`, or `failure` / `error`, records a deployment; other states are ignored. The service is `service` in the deployment's `payload`, or else the repository name; the env is the deployment's `environment`; the version its `ref`; the link its status's `target_url` or `log_url`.
- **GitLab**: *Deployment events*. A status `success`, or `failed` / `canceled` (recorded as failures), records a deployment of the project (its name is the service) to its `environment`, with the commit from `commit_url`. GitLab gives no start time, so these deployments have no duration.

### CD custom spans

`POST /api/intake/ci/custom_spans` reads a span whose tags include
`cd.env`. The other tags it uses: `cd.service` (or `service`),
`cd.revision` (the version), `cd.is_rollback:true`, `git.commit.sha`,
`git.repository_url`, `team` and `ci.pipeline.url`. A non-zero
`exit_code` or an `error_message` marks it failed.

```sh
curl -X POST http://localhost:8080/api/intake/ci/custom_spans \
  -H "DD-API-KEY: $ROBINE_DOG_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"data": {"attributes": {
        "start_time": "2026-11-24T10:00:00Z", "end_time": "2026-11-24T10:03:12Z",
        "tags": {"cd.env": "prod", "cd.service": "shop-api", "cd.revision": "1.4.2",
                 "git.commit.sha": "9f2c1e7"}}}}'
```

> **Note:** recent `datadog-ci` releases send `datadog-ci deployment mark`
> as tags on the CI job (`POST /api/v2/ci/pipeline/tags`), which Robine Dog
> acknowledges without storing. If marked deployments do not appear, call
> the DORA API from the same step instead.

## The Deployments page

[Deployments](/ci/deployments) lists the deployments that finished in the
time range (the last week by default), newest first: service, env,
version, commit, status, duration, source and finish time. Filter by
service and env; click a row that has a link to open it.

## DORA metrics

[DORA Metrics](/ci/dora) shows the four metrics over the time range (the
last 30 days by default), overall and per service, for the service and env
you filter on.

| Metric | How Robine Dog computes it |
| --- | --- |
| Deployment frequency | Deployments in the range divided by its length in days |
| Lead time for changes | Median time from the start of the **first CI pipeline execution** of the deployed commit (any execution or job in [CI Visibility](ci-visibility.md) with that SHA) to the end of the deployment. Deployments without a commit, or whose commit no pipeline ran, are left out |
| Change failure rate | Failed deployments over all deployments. A deployment failed when its status is a failure, it is a rollback, or a failure on its service started after it finished and before that service's next deployment |
| Time to restore | Median duration of the failures that ended, from their start to their end |

Failures are:

- incidents sent to `POST /api/v2/dora/incident`, and
- Robine Dog's [incidents](incidents.md) created in the range, on the services they list; they end when they are resolved.

```sh
curl -X POST http://localhost:8080/api/v2/dora/incident \
  -H "DD-API-KEY: $ROBINE_DOG_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"data": {"attributes": {
        "services": ["shop-api"], "env": "prod", "name": "Checkout errors after 1.4.2",
        "severity": "SEV-2", "team": "checkout",
        "started_at": "2026-11-24T10:20:00Z", "finished_at": "2026-11-24T11:05:00Z"}}}'
```

`started_at` is required; `services` can also be a single `service`. An
incident sent without `finished_at` counts in the change failure rate but
not in the time to restore. When you filter on an env, DORA incidents of
another env are left out (incidents without an env are kept).

## Differences from Datadog

- Deployments are not inferred from APM version changes; for what each version did in production, see the deployment tracking of the [service pages](services.md).
- DORA metrics are computed on request over the stored deployments; there are no `dora.*` metrics to graph or monitor.
- Deployments sent through the DORA API are always successes; mark a failure with a DORA incident or an incident in Robine Dog.
- Deployment gates, Argo CD and other CD provider integrations are not implemented.
