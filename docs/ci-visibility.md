---
title: CI Visibility and Test Optimization
section: Software Delivery
order: 10
app: /ci/pipelines
description: Pipelines and jobs from GitHub Actions, GitLab or the pipeline API, and test results from the ddtrace test integrations or JUnit reports, with flaky and slow tests and ci.* metrics for monitors.
---

# CI Visibility and Test Optimization

CI Visibility shows how your pipelines behave: which ones fail, how long
they take, which jobs slow them down. Test Optimization does the same for
tests: failures, flaky tests and the slowest tests, per service and branch.

| Data | Comes from | Page |
| --- | --- | --- |
| Pipelines, stages and jobs | GitHub Actions and GitLab webhooks, or the pipeline API | [CI Visibility](/ci/pipelines) |
| Test runs | The ddtrace test integrations, or JUnit XML reports uploaded with `datadog-ci` | [Test Optimization](/ci/tests) |

Pipelines also feed the lead time of [DORA Metrics](cd-visibility.md#dora-metrics).

## Send pipelines

### GitHub Actions

In your repository (or organization), go to **Settings > Webhooks > Add
webhook**:

| Setting | Value |
| --- | --- |
| Payload URL | `http://localhost:8080/api/v2/webhook?dd-api-key=<your API key>` |
| Content type | `application/json` (the default form encoding is rejected) |
| Secret | Leave empty: it is not checked |
| Events | *Workflow runs* and *Workflow jobs* (add *Deployment statuses* for [CD Visibility](cd-visibility.md)) |

A workflow run becomes a pipeline, each of its jobs a job. Only finished
runs and jobs (`status: completed`) are stored.

### GitLab

In your project (or group), go to **Settings > Webhooks > Add new
webhook**: URL `http://localhost:8080/api/v2/webhook?dd-api-key=<your API
key>`, triggers **Pipeline events** and **Job events** (and **Deployment
events** for CD Visibility). The secret token is not checked. Pipeline
hooks become pipelines and job hooks become jobs, with their stage, once
they are no longer running.

> **Warning:** the API key travels in the webhook URL. Create a dedicated
> key in [API Keys](/organization-settings/api-keys) so you can revoke it
> alone (see [API and application keys](api-keys.md)).

### Other CI providers: the pipeline API

Post each finished pipeline, stage or job to `POST /api/v2/ci/pipeline`, as
Datadog's pipeline API takes it:

```sh
curl -X POST http://localhost:8080/api/v2/ci/pipeline \
  -H "DD-API-KEY: $ROBINE_DOG_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"data": {"type": "cipipeline_resource_request", "attributes": {
        "provider_name": "jenkins",
        "resource": {
          "level": "pipeline", "unique_id": "shop-build-812", "name": "shop-build",
          "status": "success", "url": "https://ci.example.com/job/shop-build/812",
          "start": "2026-11-24T10:00:00Z", "end": "2026-11-24T10:06:30Z",
          "git": {"repository_url": "https://github.com/acme/shop", "branch": "main", "sha": "9f2c1e7"}
        }}}}'
```

| Field | Meaning |
| --- | --- |
| `provider_name` | Any name; `custom` if absent |
| `level` | `pipeline`, `stage` or `job` |
| `unique_id` (or `id`) | Unique per provider and level; posting the same one again updates its status, times and name |
| `pipeline_unique_id` | For a stage or job: the `unique_id` of its pipeline |
| `name`, `pipeline_name`, `stage_name` | Names; `pipeline_name` defaults to `name` |
| `status` | `success`, `error` (also `failed`, `timed_out`…), `canceled`, `skipped`, `blocked` |
| `start`, `end` | RFC 3339 times, required |
| `url`, `git.branch`, `git.sha`, `git.repository_url` | Optional |

## Send test results

### ddtrace test integrations

The test integrations of the Datadog tracers report each test, suite,
module and session. Point them at Robine Dog in one of two ways.

Through the trace port, as an Agent would receive them (Robine Dog
advertises the Agent's EVP proxy on port 8126):

```sh
DD_SERVICE=shop-api DD_ENV=ci \
DD_TRACE_AGENT_URL=http://localhost:8126 \
pytest --ddtrace
```

Or agentless, straight to the HTTP port:

```sh
DD_SERVICE=shop-web DD_ENV=ci \
DD_CIVISIBILITY_AGENTLESS_ENABLED=true \
DD_CIVISIBILITY_AGENTLESS_URL=http://localhost:8080 \
DD_API_KEY=$ROBINE_DOG_API_KEY \
NODE_OPTIONS="-r dd-trace/ci/init" npm test
```

Both send `citestcycle` payloads (MessagePack or JSON, gzipped or not),
which Robine Dog also accepts at `POST /api/v2/citestcycle`. The test's
`git.branch`, `git.commit.sha` and `git.repository_url` come from the
tracer's CI detection.

### JUnit XML reports

Any test framework that writes JUnit XML works with `datadog-ci junit
upload`:

```sh
DD_API_KEY=$ROBINE_DOG_API_KEY DD_SITE=robine.example.com DD_ENV=ci \
  npx @datadog/datadog-ci junit upload --service shop-api --skip-git-metadata-upload ./reports
```

`datadog-ci` sends reports to `https://cireport-intake.<DD_SITE>/api/v2/cireport`.
Robine Dog answers every path whatever the host name, so give it a DNS
name for `cireport-intake.<your domain>` and an HTTPS certificate valid for
it. Each `<testcase>` becomes a test: `classname` (or the enclosing
`<testsuite>`'s name) is its suite, `<failure>` or `<error>` makes it
fail (with the message), `<skipped>` skips it, and `time` is its duration.

Robine Dog takes the test service from the `service` field of the upload's
`event` part, and the `env`, `git.branch`, `git.commit.sha` and
`git.repository_url` from that event or its `tags` (the `--tags` you
pass). Current `datadog-ci` releases send the service and the git tags
they detect in the event's `metadata`, which is not read: their tests are
filed under the service `unknown`. To get branches and flaky-test
detection, pass the git tags yourself:

```sh
npx @datadog/datadog-ci junit upload --service shop-api --skip-git-metadata-upload \
  --tags git.branch:"$GITHUB_REF_NAME" --tags git.commit.sha:"$GITHUB_SHA" ./reports
```

> **Note:** a JUnit report has no start times: its tests are dated when
> the report is received.

## The CI Visibility page

For the selected time range (the last day by default):

- **Pipelines**: one row per pipeline and provider, with executions, success rate, median (P50) and P95 duration and the last status.
- **Executions**: the latest 300 pipeline executions, with status, branch, commit and duration; the name links to the execution in your CI. Click a row to list its stages and jobs with their status and duration.

Filter by pipeline, status and branch.

## The Test Optimization page

For the selected time range (the last day by default), filtered by test
service, status (`pass`, `fail`, `skip`) or text in the test name, suite or
error:

| Part | Shows |
| --- | --- |
| Services | Tests run, failed, skipped and the average duration, per test service |
| Flaky tests | Tests that both passed and failed on the same commit: number of commits where it happened, last seen, latest error (up to 100) |
| Slowest tests | The 20 tests with the highest average duration, with their run count and maximum |
| Test runs | The latest 300 test runs with status, branch, duration and error |

A test needs a commit SHA to be detected as flaky.

## Metrics

Every pipeline, job and test also produces metrics, so
[monitors](monitors.md) and [dashboards](dashboards.md) can use them:

| Metric | Type | Tags |
| --- | --- | --- |
| `ci.pipeline.count`, `ci.pipeline.duration` | count, distribution (seconds) | `ci_level`, `ci_status`, `ci_provider_name`, `ci_pipeline_name`, `git_branch` |
| `ci.job.count`, `ci.job.duration` | count, distribution (seconds) | same, for jobs and stages |
| `ci.test.count`, `ci.test.duration` | count, distribution (seconds) | `test_level` (`test`, `suite`, `module`, `session`), `test_service`, `test_status`, `git_branch` |

Pipelines and jobs are counted when they end, tests when they start. For
example, to alert when `main` fails:

```text
sum(last_1h):sum:ci.pipeline.count{ci_status:error,git_branch:main}.as_count() > 0
```

## Retention

Test runs and pipeline executions are kept as long as spans and logs
(`ROBINE_DOG_RETENTION_DAYS`, 15 days by default; see
[Configuration](configuration.md)). The `ci.*` metrics follow the metrics
retention.

## Differences from Datadog

- Only finished pipelines and jobs are stored; running executions do not appear.
- There is no test or pipeline explorer with the search syntax, no test impact analysis, auto test retries, early flake detection, quarantine or quality gates. The other endpoints the tracers and `datadog-ci` call (library settings, git metadata) are acknowledged with `202 Accepted` and nothing is stored.
- Flakiness is computed from the stored runs (pass and fail on one commit), not from retries.
- Webhook secrets and signatures are not verified: the API key in the URL authenticates the webhook.
