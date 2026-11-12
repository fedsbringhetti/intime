# Phase 3 — GTT data, rules, trip tracking and notifications: detailed plan

Status: **approved** on 2026-11-02, after Phase 2 (`v0.2.0`). Decisions G–M confirmed on 2026-10-30; GTT open data (N–Q) and libraries, navigation and mockups (U–Y) added on 2026-11-02. Mockups approved on 2026-11-04 (step 0), with decisions Z–AC; decisions O and Q changed and AD–AE added with step 2 (alerts), the same day. Decision K changed and AF–AJ added with step 3 (rules), on 2026-11-05. Decisions AK–AR added with step 4 (Web Push), the same day.

Until 2026-10-30 this was Phase 2, with Telegram as the notification channel. The phases were swapped so that notifications can use Web Push from the web app built in Phase 2.

Builds on Phase 1 (stop index, departures with cache, favorites, upstream health) and Phase 2 (web app). Developed and tested locally; authorization, deployment and the checks on real phones are in [Phase 4](phase-4.md).

## 1. Goal

Receive the departures I need on my phone, automatically, as notifications from the web app, using GTT's own data where it is better:
- real-time predictions for urban runs from GTT's feed, with OpenTripPlanner as fallback;
- GTT service alerts (detours, limited routes, cancelled runs) next to the departures and notified for the lines of a rule;
- a summary of the next departures at a fixed time on chosen days;
- tracking of a run I want to catch, started on the fly from the departures list or by a recurring rule, with reminders and alerts when it is late or cancelled, plus a live page;
- an alert when the upstream stops working;
- a history of the delays of tracked runs.

Checked on the Mac in Chrome, where `localhost` allows Web Push (there is no Xcode, so no iOS Simulator); real phones in Phase 4.

### Definition of done

- [ ] Every departure says where its time comes from (`gtt`, `otp`, `scheduled`); urban runs use GTT's prediction when it is fresh; with GTT's feed unreachable, departures still work from OpenTripPlanner.
- [ ] The stop page in the web app shows the active alerts of the lines serving it; a new alert on a line of an active rule is notified once.
- [ ] The live page of a tracked urban run shows whether a vehicle is assigned and how many stops away it is.
- [ ] A Mon–Fri 7:40 summary rule sends a notification within 30 seconds of 7:40, never on Saturdays, Sundays or public holidays, and never twice on the same day, even after a restart.
- [ ] Tapping a departure in the web app and choosing "avvisami" tracks that run today: reminders 10 and 5 minutes before the expected departure, an alert when the delay reaches 3 minutes, a single alert if it is cancelled or cannot be found.
- [ ] A recurring tracking rule ("line 4 at 8:05 from stop X, Mon–Fri") does the same every matching day.
- [ ] The live page of a tracked run shows the expected time and the countdown, updated while it is open.
- [ ] Rules can be created, edited, paused, skipped for today, resumed, tested and deleted from the web app.
- [ ] Notifications arrive in Chrome on the Mac (on real phones: Phase 4).
- [ ] If the upstream is down for 5 minutes there is one alert, and one notification when it recovers.
- [ ] Scheduling is correct across daylight saving changes (covered by tests with a fake clock).
- [ ] Idle memory stays under 60 MB in the container.

## 2. Decisions

| # | Question | Decision |
|---|----------|----------|
| G | Notification channel | **Web Push** from the web app (no Telegram, no third-party app). Sending goes through a `Notifier` interface; a `LogNotifier` is used in development and tests |
| H | Rule types | **Summary** and **tracking**. Tracking can start **on the fly** (from the departures list, for today) or from a **recurring rule**. Events: reminders N minutes before the expected departure (chosen by the user, default 10 and 5), delay over a threshold, cancellation, run not found; plus a live page. **No "leave now"**: walking time is not computed (future idea in `PLAN.md`) |
| I | Managing rules | From the web app, through the API |
| J | Language of user-facing text | Italian (notifications and web app); code, identifiers, logs and docs in English |
| K | Public holidays | Rules can skip Italian national holidays plus 24 June (San Giovanni, Turin), enabled by default per rule. National holidays include Easter Sunday and Monday (computed) and, from 2026, 4 October (San Francesco, law 151/2025; added on 2026-11-05). Personal dates are a future idea in `PLAN.md` |
| L | Delay history | Yes, for tracked runs only, stored **per run** (service date, trip, stop), not per rule or per user, so two trackings of the same run share one record and future users would share the data. To be evaluated after some weeks of use |
| M | Missed schedule (server off or restarting at due time) | Summaries: sent if at most 5 minutes late, otherwise skipped and logged. Tracking: resumes from its saved state; an alert that is no longer useful (the bus has left) is dropped, never sent late |
| N | Real-time source (`PLAN.md` decision 10) | For **urban** runs, GTT's trip updates; otherwise OpenTripPlanner's real time; otherwise the scheduled time. **One source per departure**, never mixed. OpenTripPlanner still decides which runs serve the stop, their line, destination, scheduled time and stop sequence. Merge rules in section 4 |
| O | Service alerts | From GTT's `alerts.aspx`, read every 5 minutes. Linked **by stop** when the alert lists stops, **by line** only when it lists none (changed on 2026-11-04, step 2: a line alert lists nearly every stop of its line, while a "stop suspended" alert lists one stop and every line serving it, so linking by line would show it at hundreds of stops). A new alert on a line of an active rule is notified once |
| P | Vehicle positions | From GTT's `vehicle_position.aspx`, read only while a tracking is active. Shown on the live page only: no new notification for "no vehicle assigned" (can be added later). The vehicle map is a future idea in `PLAN.md` |
| Q | GTFS static (`PLAN.md` decision 11) | `stops.txt` only, to link alerts to stops (step 2). Checked **every night** with a conditional request (`If-Modified-Since`): GTT answers `304` with no body when the zip has not changed, so the 20 MB are downloaded only after GTT rebuilds it (changed from weekly on 2026-11-04, step 2). `routes.txt` was dropped on 2026-11-04 with decision Z; line names in alerts come from the stop index (decision AD) |
| U | Decoding GTFS Realtime | **Types generated once from the official `.proto` and kept in the repository** (`src/gtt/proto.rs`, by `tools/gtfs-rt-codegen` with `prost-build` and `protox`, a protobuf compiler in pure Rust); the service depends on `prost` only, with no build script and no protobuf compiler in Docker or CI. Changed on 2026-11-04: the `gtfs-realtime` crate (0.2) first chosen generates its types at build time and needs `protoc`. Rejected: a `build.rs` with `prost-build` and `protox` (about thirty more crates at every clean build); installing `protoc` on the Mac, in Docker and in CI |
| V | Web Push library | **`web-push-native`** (0.5): pure Rust (`aes128gcm` encryption; the VAPID signature is ours since step 4, decision AK), builds an `http::Request` that we send with our `reqwest` and rustls. Rejected: `web-push` 0.11, last released in February 2025, on `hyper` 0.14 and bringing OpenSSL, which the project does not use |
| W | Reading the GTFS static archive | **`zip`** (deflate only) and **`csv`**, reading just `stops.txt`. Rejected: `gtfs-structures`, which loads the whole feed (116 MB of stop times) |
| X | Where notifications live in the web app | A **fifth entry in the bottom bar, "Notifiche"** (bell icon): "Attiva notifiche" and the test notification, trackings in progress, rules (list and editor), the log of sent notifications |
| Y | New screens | **Mocked up on the design canvas and approved before any code** (step 0), as in Phase 2: alerts banner on the stop page, "Avvisami" on a departure, the live tracking page, the Notifiche screen, the rule editor, the GTT row on the info screen. Approved on 2026-11-04: [design canvas](https://claude.ai/artifact/GEpFZXDq4dVt5SzF1p4NZP), rows "Fase 3" |
| Z | Line colors | **Keep the colors by vehicle type** from Phase 2. GTT's `routes.txt` has one color pair per category, not per line (5 pairs for 218 lines: urban buses `FF9900`/`0000FF`, extra-urban buses (four-digit lines) `0080FF`/`FFFFFF`, trams `CC9900`/`0000FF`, metro `FF0000`/`FFFFFF`, a few others `33CC00`/`000000`), and all but the last are below 4.5:1 (3.3–4.0:1). Rejected: GTT's colors as they are; GTT's colors adjusted for contrast |
| AA | Alerts banner | **Closed by default**: one line with the number of alerts and their titles, opening on tap. Alerts last for months, so the full text every time would push the departures down |
| AB | When GTT predictions are merged | **At each request**, not in the cache: OpenTripPlanner's departures and GTT's trip updates stay in two separate caches, so a source that fails does not spoil the other's data |
| AC | Comparison with OpenTripPlanner | Counted **per day** (Europe/Rome, reset at midnight), **once per run and stop**, so reloading a page does not count twice; `/api/status` shows today's mismatches and the median difference |
| AD | Line names in alerts | From the **stop index** (OpenTripPlanner's names and modes, the same as the departures: `17BU` is "17/"), falling back to the route id without its final `U` or `E` for lines no stop serves (decided 2026-11-04, step 2). Rejected: `routes.txt`, which has the same names but also misses the temporary lines (`S45U`, `B1U`) |
| AE | Alerts in the favorites' departures | **Not added** for now: no screen shows them (decided 2026-11-04, step 2). Only the departures of a stop and `/api/alerts` carry alerts |
| AF | When `rule_runs` is created | **With the scheduler (step 5)**, not with `rules` (decided 2026-11-05, step 3): no code would use it before, and the scheduler decides the shape of its state; a migration cannot change once merged |
| AG | Rules in the API | **`kind` plus a `config` object** with the kind's fields, as in the database; unknown fields are refused. Defaults are **stored** in `config` when a rule is created, so changing a default later does not change existing rules. Lines are checked against the lines of the stop in the index and stored with the index's spelling; a tracking rule has exactly one. `kind` cannot change; `PATCH` replaces `config` whole (decided 2026-11-05, step 3). Rejected: all fields at one level, which cannot refuse unknown fields |
| AH | Pause | `paused_until` is the **last day skipped, included**: "salta oggi" is `until` today, and "in pausa fino al 16 novembre" skips the 16th too (changed on 2026-11-05, step 3; the plan said "until tomorrow"). A pause without `until` turns the rule off (`enabled: false`); `resume` turns it on and clears `paused_until`. `until` is between today and a year from today |
| AI | Days of a rule | **Calendar days in Europe/Rome** of the rule's time, not GTT's service date: "Saturday 00:30" runs in the night between Friday and Saturday (decided 2026-11-05, step 3) |
| AJ | Today's state of a rule | **Computed** at each request from the calendar (`today`: due at a time, not scheduled, holiday, paused, disabled; `next`: the next occurrence), until the scheduler adds what happened (step 5). One-off trackings (`date` set) are refused by `/api/rules`: they are created by `/api/trackings` (step 6), while the calendar already handles them (decided 2026-11-05, step 3) |
| AK | Signing VAPID | **Our own ES256 JWT with `p256`**; `web-push-native` without its `vapid` feature, only for the `aes128gcm` encryption (decided 2026-11-05, step 4). 37 new crates instead of 83, and no `rsa` 0.9 (advisory RUSTSEC-2023-0071, no fix) pulled in by `jwt-simple` |
| AL | Log of sent notifications | The `notifications` table and `GET /api/notifications` come **with the scheduler (step 5)**, which decides `kind` and `rule_id`; the test notification is not recorded (decided 2026-11-05, step 4, as decision AF) |
| AM | Where the server sends pushes | Only to **`https` endpoints of known push services** on their default port: `fcm.googleapis.com`, `push.apple.com`, `push.services.mozilla.com`, `notify.windows.com` and their subdomains; redirects are not followed. Otherwise anyone able to call the API could make the server post to internal addresses (decided 2026-11-05, step 4) |
| AN | Delivery options | `TTL` 15 minutes (a late transport notification is useless), `Urgency: high`, JWT valid 1 hour (decided 2026-11-05, step 4) |
| AO | `Notifier` shape | A trait usable as `Arc<dyn Notifier>` (boxed futures); `send` delivers to every device and reports `sent`, `removed`, `failed`; one device failing never stops the others; 404 and 410 delete the subscription (decided 2026-11-05, step 4) |
| AP | Keeping subscriptions alive | When the Notifiche screen opens with permission granted, the web app **sends its subscription again** (upsert by endpoint); a subscription made with another server key is replaced (decided 2026-11-05, step 4) |
| AQ | States without a mockup | Notifications blocked, push disabled on the server, browser not supported: **text variants of the approved card**, no new mockup. On an iPhone outside the Home Screen, the mockup's warning, linking to `/info` (decided 2026-11-05, step 4) |
| AR | Device label | From the `User-Agent` on the server: iPhone, iPad, Android, Mac, Windows, otherwise none. An iPad in desktop mode says "Macintosh" and is labelled Mac (decided 2026-11-05, step 4) |

## 3. What we know about the upstream

### OpenTripPlanner

Verified on 2026-10-29.

- Every departure from `stop(...).stoptimesWithoutPatterns` carries a `trip.gtfsId` (e.g. `gtt:28440933U`).
- `trip(id).stoptimes` returns **only the static timetable** (`serviceDay: 0`, `realtime: false`).
- `trip(id).stoptimesForDate` returns **real-time** times for every stop of the run, with `serviceDay`.
- Real-time data propagates along the whole run: a run that is late at its first stop is already shown as late at later stops. Before the vehicle starts its run there is usually only the scheduled time.
- There are **no vehicle positions** (`vehiclePositions` does not exist in the schema) and **no alerts** (`alerts` returns an empty list).
- On 2026-11-02 at 10:18 it had no real-time data at all (0 of 15 departures at three stops) while GTT's feed was live.

### GTT open data

Verified on 2026-11-02; the full list of facts is in the Context of `PLAN.md`. What matters for this phase:

- Trip and route ids are OpenTripPlanner's without the `gtt:` prefix; stop sequences and scheduled times match. Stop ids do not: GTT's `stop_id` 2843 is our `gtt:39`, mapped through `stops.txt`.
- The stop sequence of a departure is OpenTripPlanner's `stopSequence` (this server's schema has no `stopPosition`): at stop 39 on 2026-11-04 it matched GTT's `stop_sequence` and scheduled time for 6 runs out of 6.
- Trip updates: urban network only; each run has predictions for its next ~6 stops, by `stop_sequence` (no `stop_id`), as an absolute `time` or as a `delay`; `schedule_relationship = CANCELED` for cancelled runs.
- Vehicle positions: urban network only; position, heading, run, line, vehicle number and timestamp; no speed, no current stop. About a quarter of the vehicles have no run; some are assigned to their next run up to 15 minutes before it starts, when OpenTripPlanner has only the scheduled time.
- Alerts: about 120 at a time, urban and suburban; mostly detours; `informed_entity` by route (`54U`) or by GTT `stop_id`, never both in one entity; `active_period` is often very long (median about a year), so it does not tell when an alert stops mattering. On 2026-11-04 (119 alerts): 104 listed routes and stops, 11 stops only, 3 routes only, 1 neither (agencies only). Line alerts list nearly all the stops of their lines (median 99%; fewer for a detour in one direction); 14 "stop suspended" or "stop moved" alerts list one stop and every line serving it. 92 stop references were missing from `stops.txt` (temporary stops); 9 routes had no stop in OpenTripPlanner (`94U`, `S45U`, ...).
- GTFS static: every one of our 7,051 GTT stops matches a `stop_code` in `stops.txt`; its 2 extra rows are stations (`location_type` 1). The zip answers `If-Modified-Since` and `If-None-Match` with `304`.
- Header timestamps are the time of the request. Entities update every ~14 s (vehicles) and ~38 s (trip updates).

Consequence: tracking polls the stop departures (already cached since Phase 1, now merged with GTT's predictions) and identifies the run by its `trip.gtfsId`. `trip(id).stoptimesForDate` is the fallback when the run has not yet entered the stop's departure window.

## 4. GTT real-time client

### Reading the feeds

| Feed | When | Cache |
|------|------|-------|
| `trip_update.aspx` | on demand, when departures are computed | 20 s; concurrent requests share one call |
| `alerts.aspx` | every 5 minutes, in the background | until the next read |
| `vehicle_position.aspx` | every 15 s, only while at least one tracking is active | until the next read |
| `gtt_gtfs.zip` (`stops.txt`, from step 2) | every night, and at startup when missing or not checked for a day; a conditional request, downloaded only when changed | stored in SQLite |

- Messages are decoded with the generated types in `src/gtt/proto.rs` (decision U); the GTFS static archive is read with `zip` and `csv` (decision W).
- Freshness comes from each entity's timestamp, never from the header: a prediction older than 3 minutes or a position older than 2 minutes is ignored.
- Identical updates are recognized by a hash of the entities, not of the whole response, which changes at every request.
- Same robustness as the OpenTripPlanner client: timeout, one retry on connection errors and 502–504, invalid messages rejected. Outcomes go to a separate entry in `/api/status`.

### Merge rules

For each departure from OpenTripPlanner on a GTT urban run:

1. Look up the trip update by trip id (without `gtt:`) and service date (`start_date`), at the departure's `stopSequence`.
2. Take the update for the departure's `stop_sequence`; if the stop has none, take the last update **before** it and apply its delay (the GTFS Realtime rule: a delay carries over to the following stops). Updates only after the stop mean it has already been passed.
3. Use it only if the result is plausible: the stop sequence exists in the run and the predicted time is within 60 minutes of the scheduled time. Otherwise fall back to OpenTripPlanner and count a **mismatch**.
4. A run cancelled by GTT is cancelled; if GTT says nothing and OpenTripPlanner says cancelled, OpenTripPlanner wins.
5. The departure gets `source: "gtt"`; otherwise `"otp"` (OpenTripPlanner real time) or `"scheduled"`.
6. When both GTT and OpenTripPlanner have a prediction, the difference is logged at debug level and counted in `/api/status`, to judge decision N after some weeks.

Mismatches show up in `/api/status`; a sudden rise usually means the two sources are on different timetable versions (GTT rebuilds its GTFS about weekly, OpenTripPlanner on its own schedule).

### Alerts

- Kept in memory, replaced at each read; when a read fails the last alerts stay. The ids of alerts already notified are stored in SQLite (step 5) so a restart does not notify them again.
- Linked to stops through `stops.txt` (GTT `stop_id` → `stop_code` → `gtt:<code>`) when the alert lists stops; otherwise, or when the static data is missing, to the stops served by its lines (`route_id` `54U` → `gtt:54U`, through the stop index). Stops missing from `stops.txt` (temporary ones) are counted in `/api/status`, not shown. An alert with neither stops nor lines is shown only by `/api/alerts` without filters (decision O).
- Only alerts whose `active_period` includes the current time are shown. Two alerts with the same title and text at one stop are shown once.
- The text is GTT's (Italian), with its link when it is `http` or `https`. Line names come from the stop index (decision AD).

### Vehicle positions in tracking

- **Vehicle assigned**: a vehicle whose run is the tracked one.
- **Stops away**: the nearest stop of the run (coordinates from `trip(id).stoptimesForDate`) compared with the tracked stop's sequence.
- Shown on the live page only (decision P).

## 5. Rule model

### Common fields

| Field | Example | Notes |
|-------|---------|-------|
| `name` | `"Casa → lavoro"` | |
| `kind` | `summary` / `track` | |
| `enabled` | `true` | |
| `stop_id` | `gtt:39` | gtfsId |
| `days` | `["mon","tue","wed","thu","fri"]` | days of the week; empty for a one-off tracking |
| `date` | `2026-11-03` | only for a one-off tracking started on the fly |
| `skip_holidays` | `true` | decision K |
| `paused_until` | `2026-11-03` | optional; the last day skipped, included; used by "pause" and "skip today" (decision AH) |

A tracking started on the fly is stored as a one-off `track` rule with `date` set and no `days`; it is removed automatically when it ends. Everything else (scheduler, state, history) is shared with recurring rules.

### Summary rule

| Field | Example | Notes |
|-------|---------|-------|
| `time` | `07:40` | Europe/Rome |
| `lines` | `["4", "15"]` | optional filter |
| `window_minutes` | `30` | departures shown in the next N minutes |
| `max_departures` | `6` | |

### Tracking rule

| Field | Example | Notes |
|-------|---------|-------|
| `line` | `"4"` | a single line |
| `scheduled_time` | `08:05` | scheduled departure at the stop |
| `trip_id` | `gtt:28440933U` | one-off tracking only: the run tapped in the departures list |
| `tolerance_minutes` | `2` | recurring rules: to match the run in the timetable |
| `watch_from_minutes` | `30` | tracking starts N minutes before `scheduled_time` |
| `alert_before_minutes` | `[10, 5]` | reminders relative to the **expected** departure |
| `delay_alert_minutes` | `3` | delay threshold for an alert |

A recurring rule identifies the run each day from line, stop and scheduled time, never from a stored `trip_id`, because trip ids change between days and timetables.

## 6. Scheduler

- A single background task, ticking every 15 seconds.
- Time comes from the `Clock` trait, so tests use a fake clock (daylight saving, midnight, holidays).
- On each tick it computes today's occurrences of every active rule in `Europe/Rome`: local times that do not exist (spring forward) move to the next valid minute; ambiguous ones (fall back) use the first occurrence.
- Each occurrence has a row in `rule_runs` (rule, date, status), so after a restart nothing already sent is sent again.
- Upstream calls go through the departures cache, so a summary and a tracking on the same stop share the same calls.

### Summary
At `time`, fetch the departures, apply the filters, send a notification listing them. Tapping it opens the departures page of that stop in the web app, which is live.

### Tracking: state machine

```
waiting ──(watch_from)──▶ searching ──(run found)──▶ tracking ──(departed)──▶ done
                              │                          │
                              └──(not found at T-10)──▶ not_found
                                                         └──(cancelled)──▶ cancelled
```

- **searching**: recurring rules look for the departure with the same line and a scheduled time within `tolerance_minutes`; one-off trackings already know the `trip_id`. Once found, the `trip_id` is stored for the rest of the day.
- **tracking**: polls every 30 seconds. Events, each notified at most once:
  - each threshold in `alert_before_minutes`, relative to the expected departure;
  - delay reaching `delay_alert_minutes`; a new alert only if the delay grows by 3 more minutes;
  - cancellation (by GTT or by OpenTripPlanner, merge rule 4), with the next departure of the same line.
- **done**: the expected departure has passed; the final delay is written to the delay history (decision L).
- **not_found**: 10 minutes before the scheduled time the run is still missing; one alert with the closest departures of that line, then stop.
- State (phase, `trip_id`, events already notified, last expected time) is saved in `rule_runs`, so tracking survives a restart.
- Notifications of the same tracking share a Web Push `tag`, so a newer one replaces the older one instead of piling up (to verify on iPhone).

### Live page
`GET /api/trackings/{id}` returns the current state of a tracking (phase, scheduled and expected time, delay, source of the time; for urban runs, whether a vehicle is assigned and how many stops away it is). The web app polls it while the page is open and shows a countdown; no notification is needed for routine updates.

## 7. Notifications (Web Push)

### `Notifier` interface

```rust illustrative
trait Notifier {
    async fn send(&self, notification: &Notification) -> Result<(), NotifyError>;
}
```

Implementations: `WebPushNotifier` and `LogNotifier` (development and tests).

### How Web Push works here

1. The server has a **VAPID key pair**, generated once (`intime vapid-keys` subcommand); the private key lives in `.env`.
2. The web app, after the user taps "attiva notifiche", asks the browser for a push subscription with the server's public key and sends it to `POST /api/push/subscriptions`. On iPhone this works only when the web app was added to the Home Screen (iOS 16.4+), and the permission prompt must follow a tap.
3. To notify, the server encrypts the payload and posts it to the subscription's endpoint (Apple's or Google's push service). This is an outbound call: it does not go through Cloudflare Access.
4. The service worker in the web app shows the notification; tapping it opens the right page (departures of a stop, live page of a tracking).
5. Subscriptions that the push service reports as gone (HTTP 404 or 410) are deleted.

Library: `web-push-native` (decision V). It encrypts the payload and returns an `http::Request`; the VAPID header is signed by our code (decision AK), and the request is sent with a `reqwest` client of its own (rustls, 10-second timeout, no redirects: decision AM).

### Message examples (Italian, decision J)

```text
🚌 Casa → lavoro — Porta Nuova (39)
4  → Falchera   07:43 (+2)  tra 3'
15 → Sassi      07:47       tra 7'
```

```text
⏰ Il 4 delle 08:05 passa tra 5 minuti (08:08, +3)
```

```text
⚠️ Il 4 delle 08:05 è cancellato. Il prossimo: 08:17
```

```text
🚧 Linea 54 limitata in entrambe le direzioni
Oggi dalle 8:00 alle 14:00 circa. Capolinea provvisorio alla fermata 1471.
```

Added with the mockups (2026-11-04): 🐢 for a delay ("Il 67 delle 08:05 è in ritardo di 4 minuti"), ❓ for a run not found, 📡 for upstream health alerts.

## 8. Upstream health alerts

Built on `UpstreamHealth` from Phase 1:

- if there was no traffic in the last 5 minutes, a light probe query is sent;
- **down**: failures for at least 5 minutes in a row; one notification, then silence;
- **recovered**: one notification when calls succeed again;
- **schema changed**: a GraphQL validation error alerts immediately, because the code needs fixing;
- **stop sync failed**: two nightly syncs in a row failed.
- **GTT feeds**: the same down and recovered alerts for GTT's real-time feeds, tracked separately; while they are down, departures use OpenTripPlanner (decision N), so the alert is informative only. A weekly GTFS download that fails twice in a row alerts like a stop sync.

## 9. Schema

```sql illustrative
CREATE TABLE rules (
  id            INTEGER PRIMARY KEY,
  name          TEXT NOT NULL,
  kind          TEXT NOT NULL,        -- "summary" | "track"
  enabled       INTEGER NOT NULL DEFAULT 1,
  stop_id       TEXT NOT NULL,
  days          TEXT NOT NULL,        -- JSON: ["mon", ...]; empty for one-off trackings
  date          TEXT,                 -- one-off trackings only
  skip_holidays INTEGER NOT NULL DEFAULT 1,
  paused_until  TEXT,                 -- last day skipped, included (decision AH)
  config        TEXT NOT NULL,        -- JSON with the kind-specific fields (section 5)
  created_at    TEXT NOT NULL,
  updated_at    TEXT NOT NULL
);

-- Created with the scheduler, step 5 (decision AF).
CREATE TABLE rule_runs (
  rule_id    INTEGER NOT NULL REFERENCES rules(id) ON DELETE CASCADE,
  date       TEXT NOT NULL,           -- service date, Europe/Rome
  status     TEXT NOT NULL,           -- waiting, searching, tracking, done, not_found, cancelled, skipped, failed
  state      TEXT,                    -- JSON: trip_id, events notified, last expected time
  updated_at TEXT NOT NULL,
  PRIMARY KEY (rule_id, date)
);

CREATE TABLE push_subscriptions (
  id         INTEGER PRIMARY KEY,
  endpoint   TEXT NOT NULL UNIQUE,
  p256dh     TEXT NOT NULL,
  auth       TEXT NOT NULL,
  label      TEXT,                    -- e.g. "iPhone", from the user agent
  created_at TEXT NOT NULL
);

CREATE TABLE notifications (
  id       INTEGER PRIMARY KEY,
  rule_id  INTEGER,                   -- null for system alerts
  kind     TEXT NOT NULL,             -- summary, reminder, delay, cancelled, not_found, system
  channel  TEXT NOT NULL,             -- "webpush", "log"
  text     TEXT NOT NULL,
  sent_at  TEXT NOT NULL
);

-- One row per tracked run: shared by every rule (and, later, every user) tracking it.
CREATE TABLE observed_delays (
  service_date   TEXT NOT NULL,
  trip_id        TEXT NOT NULL,
  stop_id        TEXT NOT NULL,
  line           TEXT NOT NULL,
  scheduled      TEXT NOT NULL,
  final_expected TEXT NOT NULL,
  delay_seconds  INTEGER NOT NULL,
  source         TEXT NOT NULL,        -- where the final time came from: gtt, otp, scheduled
  PRIMARY KEY (service_date, trip_id, stop_id)
);

-- From GTT's stops.txt (decision Q, step 2), replaced at each download.
CREATE TABLE gtt_stops (
  gtt_stop_id TEXT PRIMARY KEY,         -- GTT stop_id, e.g. "2843"
  stop_id     TEXT NOT NULL              -- our gtfsId, e.g. "gtt:39" (from stop_code)
);

-- The zip's Last-Modified, sent back as If-Modified-Since; the row "gtt_static" records the
-- last check (at) and the number of stops (count).
ALTER TABLE sync_state ADD COLUMN version TEXT;

-- Alerts already notified, so a restart does not notify them again.
CREATE TABLE notified_alerts (
  alert_id    TEXT PRIMARY KEY,
  notified_at TEXT NOT NULL
);
```

`config` is JSON because the two rule kinds have different fields; it is validated with typed Rust structs on read and write.

## 10. API

| Method | Path | Description |
|--------|------|-------------|
| GET, POST | `/api/rules` | list rules with today's state; create a rule |
| GET, PATCH, DELETE | `/api/rules/{id}` | details, edit, delete |
| POST | `/api/rules/{id}/pause` | pause until a date, included ("skip today" = until today); without `until`, turn the rule off (decision AH) |
| POST | `/api/rules/{id}/resume` | resume: turn on, clear the pause |
| POST | `/api/rules/{id}/test` | run the rule now and send the result |
| GET | `/api/rules/{id}/runs` | recent runs |
| POST | `/api/trackings` | track a run on the fly: `{"stop": "39", "trip_id": "gtt:..."}` |
| GET, DELETE | `/api/trackings/{id}` | live state of a tracking; stop it |
| GET | `/api/push/key` | VAPID public key for the web app |
| POST, DELETE | `/api/push/subscriptions` | register or remove this device |
| POST | `/api/notifications/test` | send a test notification to every device |
| GET | `/api/notifications` | sent notifications log |
| GET | `/api/delays?stop=&line=&time=` | delay statistics of a run (average, worst, number of days) |
| GET | `/api/alerts?stop=&line=` | active GTT alerts, filtered by stop (through its lines) or line |

Changes to existing endpoints:

- **Departures** (`/api/stops/{stop}/departures`, `/api/favorites/departures`): each departure gets `source` (`gtt`, `otp`, `scheduled`). `realtime` stays, true when `source` is not `scheduled`. The departures of a stop also get `alerts`, the active alerts linked to the stop (decisions O and AE).
- **`/api/status`**: a `gtt` entry with the outcome of each feed, the age of the last data, today's number of mismatches and median difference from OpenTripPlanner (decision AC), the alerts read and the version of the static data.

Validation errors use the Phase 1 error format.

## 11. Screens

Mocked up and approved before the code (decision Y), on the same design canvas as Phase 2, with the same tokens and components.

| Screen | Path | Content |
|--------|------|---------|
| Stop (changes) | `/fermata/{stop}` | banner with the active GTT alerts of the stop (lines, title, text, link), closed by default (decision AA); a small warning icon on the departures of a line with alerts; tapping a departure opens a sheet with "Avvisami" for that run |
| Live tracking | `/corsa/{id}` | line, destination, stop; countdown to the expected departure, expected and scheduled time, delay, source of the time; for urban runs, "mezzo assegnato" and "a N fermate"; the events already notified; the run's past delays (decision L); "Smetti di seguire" |
| Notifiche (new tab) | `/notifiche` | "Attiva notifiche" (permission asked after the tap) and "Invia una prova"; trackings in progress, each opening its live page; rules with today's state, enabled toggle, pause and "salta oggi"; the log of sent notifications |
| Rule editor | `/notifiche/regole/nuova`, `/notifiche/regole/{id}` | kind (riepilogo or segui una corsa), name, stop (search as in Cerca), days, time, lines, reminders, delay threshold, "salta i festivi"; "Prova adesso", delete |
| Info (changes) | `/info` | rows for GTT's predictions (working, last data), OpenTripPlanner's real time, today's comparison between them (step 1); the date of GTT's static data and of its last check (step 2) |

The bottom bar gets a fifth entry, "Notifiche" (decision X).

## 12. Configuration

| Variable | Default | Purpose |
|----------|---------|---------|
| `VAPID_PRIVATE_KEY` | empty | Web Push key (secret, in `.env`); when empty, `LogNotifier` is used |
| `VAPID_SUBJECT` | empty | contact for push services, e.g. `mailto:...` |
| `SCHEDULER_TICK_SECS` | `15` | scheduler tick |
| `TRACK_POLL_SECS` | `30` | polling interval during tracking |
| `MISSED_RUN_GRACE_MINUTES` | `5` | decision M |
| `UPSTREAM_DOWN_ALERT_MINUTES` | `5` | delay before the "down" alert |
| `GTT_REALTIME_URL` | `https://percorsieorari.gtt.to.it/das_gtfsrt` | base URL of the three real-time feeds; empty disables GTT data (OpenTripPlanner only) |
| `GTT_GTFS_URL` | `https://www.gtt.to.it/open_data/gtt_gtfs.zip` | GTFS static archive, checked every night 10 minutes after `STOPS_SYNC_TIME`; empty disables it (alerts are then linked by line only) |
| `GTT_TRIP_UPDATES_CACHE_SECS` | `20` | cache of the trip updates |
| `GTT_ALERTS_POLL_SECS` | `300` | alerts reading interval |
| `GTT_VEHICLES_POLL_SECS` | `15` | vehicle positions reading interval during tracking |

## 13. Tests

- **Unit, with fake clock**: rule occurrences (weekdays, holidays, Easter Monday, pause, skip, one-off dates), daylight saving on both days, midnight.
- **Tracking state machine**: sequences of departures from fixtures (on time, getting later, cancelled, never appearing, restart in the middle) checked against the expected events.
- **Web Push**: a fake push service (`wiremock`): payload sent to every subscription, 404/410 removes it.
- **Idempotency**: restart between two ticks; no duplicate notification.
- **Health alerts**: failure sequences, single alert, recovery.
- **GTT feeds**: real messages recorded on 2026-11-02 as fixtures (protobuf files) for decoding; a fake server (`wiremock`) for timeouts, errors and invalid messages.
- **Merge rules**: prediction at the stop, delay carried over from an earlier stop, stop already passed, stale prediction, implausible prediction (mismatch), cancellation from either source, non-urban run, GTT unreachable.
- **Alerts**: linking by route and by stop, filtering on a stop's lines, a single notification across restarts.
- **By hand, in Chrome on the Mac**: notifications, the Notifiche screen, the live page. Real phones in Phase 4.

## 14. Development order

Each step in its own branch with a PR to `main` and its chapter in `docs/development/` (from chapter 14). When all steps are complete, `v0.3.0` is released.

0. **Mockups** of the new screens (section 11) on the design canvas, approved before step 1. No code. Done on 2026-11-04.
1. **GTT real-time predictions**: real-time client for trip updates, merge rules, `source` in departures, `/api/status` and the GTT rows on the info screen. Check: departures of an urban stop show `source: "gtt"`, and fall back to OpenTripPlanner with the feed disabled.
2. **Alerts**: GTFS static download (`stops.txt` → `gtt_stops`), reading, linking, `alerts` in the departures of a stop, `/api/alerts`, banner on the stop page of the web app, the GTT static data row on the info screen. Decisions O and Q changed, AD and AE added on 2026-11-04.
3. **Rules**: the `rules` table, typed config, CRUD API with pause and resume, holiday calendar, occurrence calculation with fake clock. Decision K changed and AF–AJ added on 2026-11-05.
4. **Web Push**: VAPID keys, subscriptions, `WebPushNotifier`, `LogNotifier`, push handling in the service worker, the Notifiche tab with "Attiva notifiche" and the test notification. Check: a test notification arrives in Chrome on the Mac. Decisions AK–AR added on 2026-11-05.
5. **Scheduler and summaries**: tick, the `rule_runs` table (decision AF), summary rules, `/api/rules/{id}/test`, notification of new alerts on the lines of a rule.
6. **Tracking**: state machine, "Avvisami" on a departure, recurring tracking rules, vehicle positions, the live page.
7. **Rules in the web app**: list and editor in the Notifiche tab: create, edit, pause, skip today, test, delete.
8. **Health alerts**: down, recovered, schema changed, sync failed, GTT feeds.
9. **Delay history**: `observed_delays` and `/api/delays`, shown on the live page and next to tracking rules.

## 15. Out of scope for this phase

- Telegram, ntfy and other channels (the `Notifier` interface allows them later)
- Walking time and "leave now" (future idea)
- Personal dates to skip (future idea)
- Vehicle map (future idea); positions are used only in tracking
- Full GTFS import and departures computed locally (future idea)
- Authorization, deployment and checks on real phones (Phase 4)
- Multiple users
- Rules spanning several stops or interchanges

## 16. Risks

| Risk | Mitigation |
|------|------------|
| Real-time data is missing for a run (`source: "scheduled"`) | Alerts use the scheduled time and say so |
| GTT's predictions turn out worse than OpenTripPlanner's | The difference is measured from step 1 (`/api/status`); if it is large, OpenTripPlanner goes back first (`PLAN.md` decision 10), a one-line change in the merge order |
| GTT and OpenTripPlanner on different timetable versions for a few hours | Plausibility check per departure, fallback to OpenTripPlanner, mismatch counter in `/api/status` |
| GTT's feeds change or disappear (endpoints are not documented as stable) | `GTT_REALTIME_URL` can be emptied: everything works on OpenTripPlanner, as in Phase 2 |
| The run cannot be matched (timetable change, wrong scheduled time) | `not_found` alert with the closest departures of that line, so the rule can be fixed |
| iPhone Web Push limits (Home Screen only, permission after a tap, `tag` behavior) | Designed in from Phase 2 (installable PWA); verified on a real iPhone in Phase 4 |
| A push subscription expires | Removed on 404/410; the web app re-subscribes when opened |
| Too many upstream calls during tracking | Shared departures cache; polling only inside the watch window; GTT's trip updates cover the whole network in one call |
| `web-push-native` is a young library | A fake push service in the tests, and a real notification in Chrome at step 4, before the scheduler and tracking are built on it |
| The GTFS Realtime specification changes | Additions only so far: replace the `.proto` in `tools/gtfs-rt-codegen` and generate the types again |
