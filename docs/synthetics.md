---
title: Synthetic Monitoring
section: Digital Experience
order: 30
app: /synthetics/tests
description: API, multistep and browser tests that Robine Dog runs on a schedule or from CI, with Datadog's test format, assertions, variables, alerting and datadog-ci support.
---

# Synthetic Monitoring

Synthetic tests check your services from the outside, on a schedule, before
your users notice a problem: an endpoint that answers 500, a certificate
about to expire, a DNS record gone wrong, a checkout page that no longer
works. Robine Dog runs the tests itself and keeps the shape of Datadog's
Synthetics API, so existing definitions, Terraform and `datadog-ci` work.

Tests run from one location, `robine-dog`: the Robine Dog server.

## API tests

Create tests on [Synthetic Tests](/synthetics/tests) with **New API Test**,
or through the API. Each API test has a `subtype`:

| Subtype | Request | Assertions (`type`) |
| --- | --- | --- |
| `http` | `method`, `url`, `headers`, `body`, `timeout` | `statusCode`, `responseTime`, `body` (with `validatesJSONPath`), `header` |
| `ssl` | `host`, `port` (443), `servername` (SNI) | `certificate` (`isInMoreThan` days), `tlsVersion`, `property`, `responseTime` |
| `tcp` | `host`, `port` | `connection`, `responseTime` |
| `dns` | `host`, `dnsServer` (the system's resolver), `dnsServerPort` (53) | `recordSome`, `recordEvery` (`property`: record type, `A` by default), `responseTime` |
| `udp` | `host`, `port`, `message` | `receivedMessage`, `responseTime` |
| `websocket` | `url`, `message`, `headers` | `receivedMessage`, `header`, `responseTime` |
| `grpc` | `host`, `port` (443), `service` | `grpcHealthcheckStatus`, `grpcMetadata`, `responseTime` |
| `icmp` | `host`, `numberOfPackets` (4, 1 to 10) | `latency` (`property`: `min`, `avg`, `max`), `packetLossPercentage`, `packetsReceived` |

gRPC tests call the standard health check service. Operators are Datadog's:
`is`, `isNot`, `lessThan`, `lessThanOrEqual`, `moreThan`, `moreThanOrEqual`,
`contains`, `doesNotContain`, `matches`, `doesNotMatch`, `startsWith`,
`isEmpty`. The test fails on the first assertion that does not hold, and the
result says which and why.

```sh
curl -X POST http://localhost:8080/api/v1/synthetics/tests/api \
  -H "DD-API-KEY: $ROBINE_DOG_API_KEY" -H "Content-Type: application/json" \
  -d '{
    "name": "Shop health",
    "type": "api", "subtype": "http",
    "config": {
      "request": {"method": "GET", "url": "https://shop.example.com/health"},
      "assertions": [
        {"type": "statusCode", "operator": "is", "target": 200},
        {"type": "responseTime", "operator": "lessThan", "target": 500}
      ]
    },
    "options": {"tick_every": 60, "min_failure_duration": 300},
    "locations": ["robine-dog"],
    "message": "The shop is down @webhook-ops",
    "tags": ["env:prod", "team:shop"]
  }'
```

> **Note:** ICMP tests need Robine Dog to send pings: give it the `NET_RAW`
> capability, or allow its group in `net.ipv4.ping_group_range`.

### Multistep API tests

With `subtype: multi`, `config.steps` lists requests run in order. Each step
has its own `subtype` (any of the above, or `wait` with `value` seconds),
request and assertions. A step can extract values for the next ones with
`extractedValues`: from the body, a header or the status code
(`http_body`, `http_header`, `http_status`), whole or through a
`json_path` or `regex` parser. With `allowFailure`, the test goes on when the
step fails; `isCritical` then says whether that failure still fails the test.

## Browser tests

A browser test (**New Browser Test**, `type: browser`) loads a start URL in a
headless Chromium, with a fresh profile, then runs its `steps`:

| Kind | Step types |
| --- | --- |
| Navigation | `goToUrl`, `refresh`, `scroll`, `wait` |
| Interaction | `click` (primary, double, right), `hover`, `typeText`, `pressKey`, `selectOption` |
| Assertions | `assertElementPresent`, `assertElementContent`, `assertElementAttribute`, `assertPageContains`, `assertPageDoesNotContain`, `assertCurrentUrl`, `assertFromJavascript` |
| Variables | `extractVariable`, `extractFromJavascript` |

Elements are found by their locator, CSS or XPath. A step waits up to its
`timeout` (20 seconds by default) for its element or condition; a run stops
after 5 minutes. Each step ends with a screenshot, shown with the result.

`options.device_ids` sets the viewport: `chrome.laptop_large` (1440×1100, the
default), `chrome.tablet` (1024×1366) or `chrome.mobile_small` (390×844,
mobile). Two browser runs happen at a time at most.

Robine Dog looks for Chromium on the host; set `ROBINE_DOG_CHROMIUM` to its
path if it is elsewhere, and `ROBINE_DOG_CHROMIUM_ARGS` for extra command-line
flags.

## Variables

Use `{{ NAME }}` in URLs, headers, bodies and browser steps.

- **Local variables** (`config.configVariables`, or `config.variables` for
  browser tests) are generated at each run from a pattern:
  `{{ numeric(6) }}`, `{{ alphabetic(8) }}`, `{{ alphanumeric(8) }}`,
  `{{ uuid }}`, `{{ timestamp(-1h, ms) }}`, `{{ date(1d, YYYY-MM-DD) }}`.
  A local variable of `type: global` gives a global variable another name.
- **Global variables**, on [Global Variables](/synthetics/variables) or
  through `/api/v1/synthetics/variables`, are shared by every test. A
  **secure** one is used in runs but its value is never returned by the API.
- **Extracted variables** come from earlier steps of the same run.

## Scheduling and retries

| Option | Meaning |
| --- | --- |
| `tick_every` | Seconds between runs: at least 30 for API tests and 300 for browser tests; 60 and 300 when unset |
| `retry.count`, `retry.interval` | Retries of a failed run before it counts as failed: 3 at most, `interval` in milliseconds (300 by default, 10,000 at most) |
| `min_failure_duration` | Seconds a test must fail before it alerts; 0 alerts on the first failure |
| `follow_redirects`, `accept_self_signed` | HTTP and SSL behavior |

A test with `status: paused` does not run on schedule. **Run now** on a test
runs it at once.

## Alerting

A failing test alerts by itself, as in Datadog. When it has failed for
`min_failure_duration`, Robine Dog emits an event
`[Triggered] Synthetic test failed: <name>`, and sends the test's `message`
to the `@webhook-…` and e-mail handles it names (see
[Notifications](notifications.md)). When it passes again, a `[Recovered]`
event and notification follow. Each test also appears in
[Monitors](monitors.md) as `[Synthetics] <name>`, of type `synthetics alert`.

## Results, logs and metrics

Each run is stored in PostgreSQL with its steps, and is also written as:

- a log with `source:synthetics`, `@synthetics.check_id`,
  `@synthetics.passed` and `@synthetics.failure`, status `error` when it
  failed;
- metrics tagged with the test's tags, `check_id` and `test_type`:

| Metric | Meaning |
| --- | --- |
| `synthetics.test_runs` | One per run, tagged `status:passed` or `status:failed` |
| `synthetics.<subtype>.response.time` | API tests' duration in milliseconds, such as `synthetics.http.response.time` |
| `synthetics.browser.test_duration` | Browser tests' duration in milliseconds |
| `synthetics.ssl.certificate.days_left` | Days before the certificate expires (SSL tests) |

Results are kept for `ROBINE_DOG_RETENTION_DAYS` days (15 by default);
screenshots only for the latest 50 runs of each test.

## Continuous Testing

Run tests from CI with `datadog-ci`, pointed at Robine Dog:

```sh
DD_API_HOST_OVERRIDE=http://localhost:8080 DD_API_KEY=$ROBINE_DOG_API_KEY \
  DD_APP_KEY=$ROBINE_DOG_APP_KEY \
  datadog-ci synthetics run-tests --public-id abc-def-ghi
```

A trigger (`POST /api/v1/synthetics/tests/trigger/ci`) takes stored tests by
`public_id` or local definitions, and runs a batch: one run per test and per
device. `datadog-ci` polls the batch until every run is done (30 minutes at
most by default). These overrides apply: `startUrl`, `variables`, `headers`,
`body`, `bodyType`, `basicAuth`, `cookies`, `followRedirects`,
`allowInsecureCertificates`, `deviceIds`, `defaultStepTimeout`, `retry` and
`executionRule` (`blocking`, the default, `non_blocking` or `skipped`). CI
runs are tagged `trigger:ci` and do not alert. Recent batches are on
[Continuous Testing](/synthetics/ci).

## API

| Method and path | Does |
| --- | --- |
| `GET`, `POST /api/v1/synthetics/tests` | Lists or creates tests |
| `POST /api/v1/synthetics/tests/api`, `/tests/browser` | Creates an API or browser test |
| `GET`, `PUT /api/v1/synthetics/tests/{type}/{id}` | Reads or updates a test (`type`: `api` or `browser`) |
| `POST /api/v1/synthetics/tests/delete` | Deletes tests (`public_ids`) |
| `GET /api/v1/synthetics/tests/search?text=` | Searches by name, or `tag:`, `type:`, `id:` |
| `PUT /api/v1/synthetics/tests/{id}/status` | Sets `new_status`: `live` or `paused` |
| `GET /api/v1/synthetics/tests/{id}/results` | The test's runs |
| `/api/v1/synthetics/variables` | Global variables |
| `GET /api/v1/synthetics/locations` | The single `robine-dog` location |

## Differences from Datadog

- Tests run from Robine Dog only; `locations` are accepted but every test
  runs once, from `robine-dog`. There are no private locations.
- Browser tests run in Chromium only, with the three devices above; there
  is no test recorder, and no mobile application tests.
