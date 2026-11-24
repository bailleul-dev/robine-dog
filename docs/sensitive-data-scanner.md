---
title: Sensitive Data Scanner
section: Logs
order: 70
app: /logs/sensitive-data-scanner
description: Scanning rules find card numbers, e-mail addresses, tokens and your own patterns in logs at intake, then redact, hash or mask them and tag the log, before anything is stored.
---

# Sensitive Data Scanner

Applications log things they should not: a card number in an error message,
a bearer token in a request dump, a customer's e-mail address. The Sensitive
Data Scanner looks for them in every log as it arrives and replaces what it
finds before the log is stored, archived, forwarded or counted. It also tags
the log, so you can find the services that leak data and fix them.

The configuration and its API are Datadog's: scanning **groups** select
logs with a query, and each group holds **rules** that say what to look for
and what to do with it.

## Where it runs

The scanner runs at intake, on each log, after the [pipelines](log-pipelines.md)
and before everything else:

1. Pipelines parse and enrich the log.
2. The Sensitive Data Scanner scans and redacts it.
3. [Log-based metrics](generate-metrics.md) count it, [Log Forwarding](log-forwarding.md)
   and [Archives](log-archives.md) receive it, and [indexes](log-indexes.md)
   decide whether it is stored.

Because it runs after the pipelines, group filters and rule scopes can use
the attributes the pipelines extracted. Because it runs before storage, the
original values are never written anywhere. Logs already stored are not
rescanned when you add a rule.

## Groups

A scanning group has a name, a description, a filter and an on/off state.
Its rules apply only to the logs that match its filter (**Scan logs
matching**, a [search query](search-syntax.md); `*` for every log).

Groups are scanned in order, and the rules of a group in order. A rule sees
the log as the previous rules left it: once a card number is masked, a later
rule does not find it.

## Rules

| Setting | API attribute | Meaning |
| --- | --- | --- |
| Name | `name` | Also the default tag: `sensitive_data:<name>`, lower case with non-alphanumerics as `_` |
| Pattern | `relationships.standard_pattern` or `pattern` | One of the standard patterns below, or a custom regular expression |
| Scan only these attributes | `namespaces` | Attribute paths to scan. Empty: the whole event |
| Except | `excluded_namespaces` | Attribute paths not to scan; `message` skips the message |
| Action | `text_replacement` | What to do with each match. See [Actions](#actions) |
| Tags added to matching logs | `tags` | Tags added when the rule finds something |
| Enabled | `is_enabled` | Disabled rules are skipped |

### Scope

With an empty scope, a rule scans the log's message and every attribute,
nested objects and arrays included. A path in `namespaces` or
`excluded_namespaces` covers that attribute and everything under it:
`usr` covers `usr.email` and `usr.name`. When `namespaces` lists attributes,
only those are scanned and the message is not.

Strings and numbers are scanned. A number that gets redacted becomes a
string.

### Actions

| Action | `text_replacement.type` | Result for `4111 1111 1111 1111` |
| --- | --- | --- |
| Tag only | `none` | Unchanged; the log is tagged |
| Replace with a string | `replacement_string` (with `replacement_string`) | `[redacted]` |
| Hash | `hash` | The first 16 hexadecimal characters of the match's SHA-256, the same for the same value |
| Mask the first characters | `partial_replacement_from_beginning` (with `number_of_chars`) | `************11 1111` with 12 characters |
| Mask the last characters | `partial_replacement_from_end` (with `number_of_chars`) | `4111 11************` with 12 characters |

Masks count the characters of the match as found, spaces and dashes
included. They use `*`, or the `replacement_string` when it is a single
character.

### Tags

When a rule finds something, the log gets the rule's tags. If none of them
is a `sensitive_data:` tag, `sensitive_data:<rule name>` is added. Rules on
a standard pattern also add `sensitive_data_category:<category>`. Search
for them in the [Log Explorer](log-explorer.md):

```text
sensitive_data:* service:checkout
```

## Standard patterns

| Pattern | `id` | Category | Finds |
| --- | --- | --- | --- |
| Credit card number | `credit_card` | `credit_card` | 13 to 19 digits, with spaces or dashes, that pass the Luhn checksum |
| Email address | `email_address` | `pii` | E-mail addresses |
| IBAN | `iban` | `financial` | International bank account numbers that pass the ISO 13616 checksum |
| US Social Security Number | `us_ssn` | `pii` | `123-45-6789` |
| International phone number | `phone_number` | `pii` | Phone numbers with a `+` country code |
| IPv4 address | `ipv4_address` | `network` | IPv4 addresses |
| AWS Access Key ID | `aws_access_key_id` | `credentials` | `AKIA…` and `ASIA…` key IDs |
| JSON Web Token | `json_web_token` | `credentials` | `eyJ….eyJ….…` |
| Bearer token | `bearer_token` | `credentials` | `Bearer <token>` |
| GitHub token | `github_token` | `credentials` | `ghp_`, `gho_`, `ghu_`, `ghs_`, `ghr_` tokens |
| Slack token | `slack_token` | `credentials` | `xoxb-`, `xoxp-` and other Slack tokens |
| Stripe secret key | `stripe_secret_key` | `credentials` | `sk_live_` and `rk_live_` keys |
| Private key | `private_key` | `credentials` | PEM private key blocks |
| Password in URL | `password_in_url` | `credentials` | `scheme://user:password@` |

A card number or IBAN that fails its checksum is not a match.

## The Sensitive Data Scanner page

[Sensitive Data Scanner](/logs/sensitive-data-scanner) shows each group as a
card with its filter and a table of its rules: pattern, scope, action and
tags.

- **New Scanning Group** creates a group; **Edit** and the trash icon
  change or delete it; the arrows reorder the groups.
- **Add rule** opens the rule editor. **Pattern** lists the standard
  patterns, with their description and regular expression, and **Custom
  regex**.
- **Try it** in the rule editor runs the rule, with its action, on a sample
  text and shows the number of matches and the result, before you save.

## API

The endpoints are Datadog's, in JSON:API format, with the `DD-API-KEY`
header.

| Method and path | Does |
| --- | --- |
| `GET /api/v2/sensitive-data-scanner/config` | The groups (`data.relationships.groups`) and, in `included`, the groups and rules |
| `PATCH /api/v2/sensitive-data-scanner/config` | Reorders the groups (`data.relationships.groups.data`) |
| `POST /api/v2/sensitive-data-scanner/config/groups` | Creates a group |
| `PATCH /api/v2/sensitive-data-scanner/config/groups/{id}` | Updates a group; `data.relationships.rules.data` reorders its rules |
| `DELETE /api/v2/sensitive-data-scanner/config/groups/{id}` | Deletes a group and its rules |
| `POST /api/v2/sensitive-data-scanner/config/rules` | Creates a rule in the group of `data.relationships.group` |
| `PATCH /api/v2/sensitive-data-scanner/config/rules/{id}` | Updates a rule |
| `DELETE /api/v2/sensitive-data-scanner/config/rules/{id}` | Deletes a rule |
| `GET /api/v2/sensitive-data-scanner/config/standard-patterns` | The standard patterns |

Every change increments `meta.version`. Create a group for the checkout
service, then a rule masking card numbers in it:

```sh
curl -X POST http://localhost:8080/api/v2/sensitive-data-scanner/config/groups \
  -H "DD-API-KEY: $ROBINE_DOG_API_KEY" -H "Content-Type: application/json" \
  -d '{"data": {"type": "sensitive_data_scanner_group", "attributes": {
        "name": "PCI", "filter": {"query": "service:checkout"},
        "is_enabled": true, "product_list": ["logs"]}}}'

curl -X POST http://localhost:8080/api/v2/sensitive-data-scanner/config/rules \
  -H "DD-API-KEY: $ROBINE_DOG_API_KEY" -H "Content-Type: application/json" \
  -d '{"data": {"type": "sensitive_data_scanner_rule",
        "attributes": {"name": "Card numbers", "namespaces": [], "excluded_namespaces": [],
          "tags": [], "is_enabled": true,
          "text_replacement": {"type": "partial_replacement_from_beginning", "number_of_chars": 12}},
        "relationships": {
          "group": {"data": {"type": "sensitive_data_scanner_group", "id": "<group id>"}},
          "standard_pattern": {"data": {"type": "sensitive_data_scanner_standard_pattern", "id": "credit_card"}}}}}'
```

For a custom regular expression, send `"pattern": "ORD-\\d{6}"` in the
attributes instead of the `standard_pattern` relationship.

## Limits

| Limit | Value |
| --- | --- |
| Groups | 20 |
| Rules per group | 50 |
| Length of a custom pattern | 1,000 characters |

## Differences from Datadog

- Only logs are scanned. A group whose `product_list` does not include
  `logs` is not applied; spans, RUM events and events are not scanned.
- The standard patterns are the 14 above, not Datadog's library.
- Custom patterns are Rust regular expressions: no look-around and no
  backreferences.
- Keyword dictionaries and sampling are not implemented; `samplings` is
  always empty. A rule's `priority` is stored but does not change anything.
