---
title: Code Coverage
section: Software Delivery
order: 30
app: /ci/coverage
description: Coverage reports uploaded with datadog-ci (lcov, Cobertura, JaCoCo), shown as the latest line coverage per repository, branch and flags, with its trend.
---

# Code Coverage

Code Coverage keeps the line coverage of your test suites over time. Your
CI uploads its coverage reports with `datadog-ci coverage upload`, and
[Code Coverage](/ci/coverage) shows the latest coverage of each
repository and branch, with how it moved across the last uploads.

## Upload reports

Run `datadog-ci` after your tests, in the CI job:

```sh
DD_API_KEY=$ROBINE_DOG_API_KEY DD_SITE=robine.example.com \
  npx @datadog/datadog-ci coverage upload --format lcov --flags type:unit-tests \
  --skip-git-metadata-upload coverage/lcov.info
```

`datadog-ci` sends reports to `https://ci-intake.<DD_SITE>/api/v2/cicovreprt`.
Robine Dog answers every path whatever the host name, so give it a DNS
name for `ci-intake.<your domain>` and an HTTPS certificate valid for it.
The endpoint is `POST /api/v2/cicovreprt`, authenticated by `DD-API-KEY`.

| Option | Effect in Robine Dog |
| --- | --- |
| Report paths | Files or directories; every report of one upload is summed into one figure |
| `--format` | The report format (see below); also detected from the content |
| `--flags` | Kept with the report and shown on the page; reports with different flags are tracked apart, so unit and integration coverage do not mix |
| `--skip-git-metadata-upload` | Skips sending git history, which Robine Dog does not store |

The repository, branch and commit come from the git information
`datadog-ci` detects in the CI job (`git.repository_url`, `git.branch`,
`git.commit.sha`).

## Supported formats

| Format | Coverage read |
| --- | --- |
| lcov | Sum of the `LH:` (lines hit) over the sum of the `LF:` (lines found) of every file |
| Cobertura XML | `lines-covered` over `lines-valid` of the root `<coverage>` element |
| JaCoCo XML | `covered` over `missed + covered` of the report's own `LINE` counter (the last one in the file) |

Reports may be gzipped, as `datadog-ci` sends them. An upload in which no
line is found in any report is rejected with `400` and the message `no
lines found in the coverage reports`.

## The Code Coverage page

One row per repository, branch and flags, from the latest report:

| Column | Shows |
| --- | --- |
| Repository | The repository URL, without the scheme |
| Branch, Flags | As uploaded |
| Coverage | The percentage of lines covered, with covered and total lines |
| Trend | The coverage of the last 10 uploads of that repository, branch and flags, oldest first (`71 → 72 → 74%`) |
| Commit | The commit of the latest report |
| Uploaded | When the latest report arrived |

The page reads the latest 2,000 reports.

## Differences from Datadog

- Coverage is a single line-coverage figure per upload. File-level coverage, patch (diff) coverage, pull request comments, code owners and coverage gates are not implemented; the PR diff, file fixes and configuration `datadog-ci` may send are ignored.
- Formats other than lcov, Cobertura and JaCoCo are not read.
- Coverage does not come from the test integrations' per-test code coverage.
