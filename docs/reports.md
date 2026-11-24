---
title: Scheduled reports
section: Dashboards
order: 30
app: /dashboard/reports
description: Print a dashboard to PDF and e-mail it on a cron schedule, or send it by hand. Reports need Chromium and an SMTP relay.
---

# Scheduled reports

A report sends a [dashboard](dashboards.md) as a PDF to a list of e-mail
addresses: every Monday morning, at the end of each day, or whenever you
click **Send now**. Robine Dog prints the dashboard with a headless
Chromium and sends the PDF through your SMTP relay.

## Setup

Reports need two things on the machine that runs Robine Dog.

**Chromium**, to print the dashboard. Robine Dog uses
`ROBINE_DOG_CHROMIUM` when it is set, and otherwise looks for `chromium`,
`chromium-browser`, `headless_shell`, `google-chrome` or
`google-chrome-stable` in the `PATH`. The same browser runs
[browser tests](synthetics.md).

**An SMTP relay**, to send the e-mails.

| Variable | Value |
| --- | --- |
| `ROBINE_DOG_SMTP_URL` | `smtp://user:password@smtp.example.com:587?tls=required` (STARTTLS), `smtps://user:password@smtp.example.com:465` (TLS), or `smtp://relay.internal:25` (no TLS) |
| `ROBINE_DOG_SMTP_FROM` | The sender, `robine-dog <robine-dog@localhost>` by default |
| `ROBINE_DOG_CHROMIUM` | The path to Chromium, when it is not in the `PATH` |
| `ROBINE_DOG_CHROMIUM_ARGS` | Extra arguments for Chromium, separated by spaces, such as `--no-sandbox` in a container where its sandbox cannot start |

```sh
export ROBINE_DOG_SMTP_URL="smtp://reports%40example.com:$SMTP_PASSWORD@smtp.example.com:587?tls=required"
export ROBINE_DOG_SMTP_FROM="Robine Dog <reports@example.com>"
export ROBINE_DOG_CHROMIUM=/usr/bin/chromium
```

The same SMTP settings send the e-mail [notifications](notifications.md) of
monitors. See [Configuration](configuration.md) for every setting.

## Creating a report

On [Reports](/dashboard/reports), click **New Report** and fill in the
form.

| Field | Meaning |
| --- | --- |
| **Name** | The e-mail's subject and the PDF's file name. Defaults to the dashboard's title. |
| **Dashboard** | The dashboard to print |
| **Time span** | The period the dashboard shows, ending when the report is printed: `1h`, `4h`, `1d`, `1w` (the default) or `1mo` |
| **Schedule** | A cron expression in UTC, `0 8 * * 1` (Mondays at 08:00) by default. Leave it empty to send the report only by hand. |
| **Recipients** | E-mail addresses, separated by commas. Each address gets its own e-mail. |
| **Message** | Text added to the e-mail's body |

The list of reports shows each report's time span, its schedule (or
*by hand*), its recipients and when it was last sent, with the result:
*sent to …*, or *not sent (is ROBINE_DOG_SMTP_URL set?)*. Each row has:

- **Preview PDF**: prints the dashboard now and opens the PDF, without
  sending anything.
- **Send now**: prints and sends the report at once, whatever its schedule.
- **Edit**, and the trash icon to delete the report.

## Schedules

The schedule is a standard 5-field cron expression, evaluated in UTC:
minute, hour, day of the month, month, day of the week.

| Field | Values |
| --- | --- |
| Minute | 0–59 |
| Hour | 0–23 |
| Day of the month | 1–31 |
| Month | 1–12 |
| Day of the week | 0–7, where 0 and 7 are Sunday |

Each field takes `*`, a number, a range (`1-5`), a list (`1,15`) and a step
(`*/15`, `0-30/10`). Names such as `MON` or `JAN` are not supported.

| Schedule | Sends |
| --- | --- |
| `0 8 * * 1` | Mondays at 08:00 UTC |
| `0 18 * * 1-5` | Weekdays at 18:00 UTC |
| `30 6 1 * *` | The 1st of each month at 06:30 UTC |
| `0 */6 * * *` | Every 6 hours |

Robine Dog checks the schedules once a minute.

## How a report is printed

To print a dashboard, Robine Dog creates a
[public link](dashboards.md#public-links) to it the first time the report
is printed, and opens that link in Chromium on its own listening address.
The page is drawn 1600 pixels wide; Robine Dog waits 6 seconds after it
loads for the graphs to draw, then prints it on A3 landscape pages with
backgrounds.

The e-mail's subject is the report's name. Its body reads
*`<name>`: the dashboard over the last `<span>`, attached.*, followed by
the message, and the PDF is attached as `<name>.pdf`.

Because the report goes through a public link:

- the dashboard's template variables keep their default values;
- the widgets that stay empty on public dashboards (lists, event streams
  and timelines, SLOs, monitor summaries, alert graphs and values, check
  statuses, service summaries, funnels, split graphs, distributions,
  scatter plots and host maps) are empty in the PDF too;
- the link shows up in the dashboard's **Share** dialog. If you revoke it
  there, the next print creates a new one.

> **Warning:** the report's public link works for anyone who has it, like
> any other public link. It is deleted with the report.

## Differences from Datadog

- Reports are managed in the UI only: there is no public API for them.
- The PDF shows the dashboard with its default template variable values;
  you cannot pick other values for a report.
- A failed send is recorded in the report's last result and in the server
  log; it is not retried before the next scheduled time.
