---
title: Session Replay
section: Digital Experience
order: 20
app: /rum/sessions
description: Watch what visitors saw and did on your site, rebuilt from the browser SDK's recordings, view by view, next to the session's RUM events.
---

# Session Replay

A session's events tell you that a visitor clicked **Pay** three times and
left; a replay shows you what the page looked like when they did. The
Datadog browser SDK records the page's DOM and its changes, mouse moves,
clicks, scrolls and input, and sends them to Robine Dog with the other
[RUM](rum.md) data. Robine Dog stores the recordings and plays them back in
the browser.

## Turn on recording

Session Replay needs the [RUM browser SDK](rum.md#set-up-the-browser-sdk),
set up with the `proxy` option. Set `sessionReplaySampleRate` to the
percentage of sessions to record:

```js
import { datadogRum } from "@datadog/browser-rum";

datadogRum.init({
  applicationId: "shop-web",
  clientToken: "<client token>",
  proxy: "http://localhost:8080/rum/proxy",
  service: "shop-web",
  env: "prod",
  sessionSampleRate: 100,
  sessionReplaySampleRate: 20,
  defaultPrivacyLevel: "mask-user-input",
});
```

`sessionReplaySampleRate: 0` turns recording off. `defaultPrivacyLevel`
works as in Datadog: masking happens in the browser, before anything is
sent, so masked text and input never reach Robine Dog.

## How recordings are received and stored

The SDK sends the recording in **segments** to `/api/v2/replay` through the
same proxy (`/rum/proxy?ddforward=/api/v2/replay…`), or directly to
`POST /api/v2/replay`. Each segment is a `multipart/form-data` upload with:

| Part | Content |
| --- | --- |
| `segment` | The records, as zlib-compressed JSON |
| `event` | The segment's metadata: session id, view id, start and end, records count (older SDKs send these as form fields) |

The same client token as RUM is required. Robine Dog checks each segment
(zlib, raw deflate or plain JSON; 32 MiB decompressed at most), stores it
compressed in PostgreSQL, and refuses segments without a session, view or
times with a 400.

Segments are kept for `ROBINE_DOG_RETENTION_DAYS` days (15 by default), like
the RUM events; an hourly job deletes older ones.

### SDK versions

| SDK version | Recording format | Playback |
| --- | --- | --- |
| v5, v6 | rrweb's events | Played as received |
| v7 | Datadog's "change records": full snapshots and changes referring to a string table | Converted to rrweb's events in your browser before playing |

Both formats can be mixed in one Robine Dog: each session is played in its
own format. Media `play` and `pause` records are mapped to rrweb's media
interactions.

## Watch a replay

From [Sessions](/rum/sessions), click **Replay** on a session (shown only
on recorded sessions). The replay page (`/rum/replay/<session id>`)
shows:

- **Views**: **Whole session**, then each recorded view with its start time,
  duration and number of records. Click a view to replay only that view.
- The **player**: play and pause, a timeline to seek, speed control, and
  inactive periods skipped automatically.
- **Session events**: the session's events in the RUM Explorer.

A view without a full snapshot cannot be rebuilt; the page says so instead
of playing it. A session's records are read back up to 64 MiB decompressed.

## Differences from Datadog

- No heatmaps, no developer tools tab (network and console) in the player,
  and no sharing of a replay at a timestamp.
- Replays are not available for mobile SDKs.
- There is no separate retention for replays: they follow
  `ROBINE_DOG_RETENTION_DAYS`.
