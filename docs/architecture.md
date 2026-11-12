# Architecture

What the service is, how its parts fit together, and where things live in the code. The step-by-step story of how each part was built is in [development/](development/README.md).

## What it is

A small HTTP service for real-time public transport departures in Turin. It reads data from the OpenTripPlanner GraphQL API behind muoversiatorino.it (unofficial) and adds what that API lacks or makes awkward:

- **stop search by name** (the upstream name search is broken) and by position, on a local copy of all ~19,400 stops;
- **departures** with real-time times, delays and line filters, cached so the upstream is called as little as possible;
- **GTT's own predictions** for urban runs, read from GTT's open data and merged into the departures, and **GTT's service alerts**, linked to the stops they concern (Phase 3);
- **favorite stops** with labels, line filters and a manual order;
- **rules** for notifications: a summary of the departures at a fixed time, or the tracking of a run, on chosen days, skipping public holidays (Phase 3; summaries are sent by the scheduler, trackings come with step 6);
- **Web Push**: the server signs and encrypts notifications and hands them to the browsers' push services; a Notifiche screen turns them on for the phone and sends a test (Phase 3);
- **a scheduler** that sends each summary rule at its time, then the new alerts of its stop, and keeps a log of what was sent (Phase 3, step 5);
- **resilience**: retries, stale data when the upstream fails, a status endpoint, optional token;
- **a web app** for the phone, served by the same binary, installable on the phone as "InTime" (Phase 2).

It is a single Rust binary, with the web app embedded in it and an SQLite database, built as a Docker image for the Orange Pi 5 (ARM64).

## How it works

### Components

```
              HTTP (axum)
                  │
  ┌───────────────┼───────────────────────────┐
  │ api/          │ auth (optional token)      │
  │  stops ──────►│ departures ──► cache ──┐   │
  │  stops ──────►│ stop index (SQLite) ◄─┐│   │
  │  favorites ──►│ favorites (SQLite)    ││   │
  │  rules ──────►│ rules (SQLite)        ││   │
  │  push ───────►│ subscriptions (SQLite)││   │
  │  notifications►│ log of sent (SQLite)  ││   │
  │  status ─────►│ upstream health ◄─────┼┤   │
  └───────────────┴───────────────────────┼┼───┘
                                          ││
         nightly sync task ───────────────┘│
                                           ▼
                                upstream client ──► OpenTripPlanner GraphQL
                                (retry, health)      (muoversiatorino.it)

  departures ──► GTT client ──────────────────────► GTT GTFS Realtime
                 (cache 20 s, retry, health)          (trip_update.aspx, alerts.aspx)
                     ▲
  alerts task ───────┘ every 5 min ──► alerts in memory ◄── departures, /api/alerts
       ▲ (read again after a download)
  static data task ── nightly check ──────────────────► GTT GTFS static (gtt_gtfs.zip)
                       └──► gtt_stops (SQLite)

  scheduler task ── every 15 s ──► rules, rule_runs, notified_alerts (SQLite)
       │                           departures, alerts in memory
       └──► notifier (below)

  notifier ── encrypted, signed (VAPID) POST ─────────► the browser's push service
  (stored subscriptions)                                 (FCM, Apple, Mozilla, Windows)
```

`main.rs` reads the configuration, opens the database (applying migrations), starts the background tasks (stop index sync; with GTT's data on, the alerts and GTT's static data; the scheduler) and serves HTTP until Ctrl+C or SIGTERM, finishing requests in progress.

The router answers `/health` and `/api/*`; every other GET goes to the web app (next section).

### Web app

- **Source**: `web/`, a Svelte project with Vite and TypeScript. `npm run build` compiles it to plain files in `web/dist/`: `index.html` and, under `assets/`, scripts and styles with a content hash in their name.
- **Embedding**: `src/web.rs` uses `rust-embed`. Release builds (and so the Docker image) include `web/dist/` in the binary; debug builds read it from disk at every request, so `npm run build` is enough to see a change with `cargo run`. Without `web/dist/` the binary still builds and runs, with the API only, and logs a warning.
- **Paths**: an existing file is served as is; `/api/*` and missing files under `/assets/` are 404; any other path gets `index.html`, and the app shows the screen for that path (`/fermata/39`).
- **Cache**: files under `assets/` are cached by the browser for a year (`immutable`: a new build gives them new names); everything else is `no-cache`, so a new version is picked up at the next load.
- **Screens**: `App.svelte` shows one screen per path (`/`, `/cerca`, `/fermata/39`, `/vicino`, `/preferiti`, `/notifiche`, `/info`) with a bottom bar of five entries (Home, Cerca, Vicino, Preferiti, Notifiche). The app's own router (`web/src/lib/router.ts`) reads the path; a listener on the document turns clicks on the app's links into screen changes without page loads, using the browser history. `/prova-grafica` shows the design base, in development builds only.
- **Data**: `web/src/lib/api.ts` calls the API with types that mirror the Rust structs and turns failures into one `ApiError`; screens abort calls they no longer need. The stop screen reloads departures every 30 seconds only while the page is visible (`poll.ts`) and filters lines on the data it has (`?linee=` in the address). GTT's alerts arrive with the departures; the stop screen shows those of the lines on screen in a banner closed by default (`AlertsBanner.svelte`) and marks the departures of lines with alerts. Names arrive in capitals and are shown as titles (`format.ts`). The near-me screen asks the browser for the position only when it opens, sends it only in that one search and keeps it nowhere (not in the address, not in storage; the server does not log requests).
- **Look**: design tokens are CSS variables in `web/src/app.css`, with a second set for the phone's dark theme; components use only the variables. Shared pieces (line badge, departure row, loaders, icons) are in `web/src/lib/components/`.
- **Installable and offline**: a web manifest and a service worker (`web/src/sw.ts`, built by `vite-plugin-pwa`). The service worker keeps the app's files on the phone and answers every screen with the cached `index.html`, so the app opens without network; it never touches `/api/`. Screens keep their last data in `localStorage` (`lastData.ts`) and show them with a "Sei offline" banner when the server cannot be reached. A new version is downloaded in the background and offered with "Aggiorna" (`update.svelte.ts`). The server sends `sw.js` and the manifest with `no-cache`, like every file outside `assets/`.
- **Notifications**: the Notifiche screen (`/notifiche`, `web/src/lib/push.ts`) shows one state: off, on, blocked, not supported, push disabled on the server, or, on an iPhone not opened from the Home Screen, the warning to install first. It asks the browser for permission only after a tap, subscribes with the server's public key and sends the subscription to the server; every time it opens it sends it again, so the server never keeps a stale one. The service worker shows each push and opens the app on the notification's screen (see Notifications below).
- **Access**: the web app is not behind the token; it holds no data, and its API calls are checked like any other.
- **Development**: `npm run dev` in `web/` serves the app on port 5173 with instant reloads and forwards `/api` to `cargo run` on 8080.

### Departures

1. The handler validates the parameters and turns `39` into `gtt:39`.
2. `DeparturesService` looks in the cache. On a miss it asks the upstream for the next 2 hours (up to 200 departures) in a single GraphQL query. Concurrent requests for the same stop share that call.
3. Each departure's time is `serviceDay + seconds`, converted to Europe/Rome. `serviceDay` is "noon minus 12 hours" of the service date (the GTFS convention), so times stay right after midnight and on daylight saving days.
4. The result is cached for `DEPARTURES_CACHE_SECS`, and also kept for 2 minutes as "last good" data.
5. Filters (lines, window, limit) are applied to the cached data, so changing them does not call the upstream again.
6. If the upstream fails and "last good" data exists, it is returned with `stale: true`.
7. GTT's predictions are merged in at every request (next section); the cached data stays OpenTripPlanner's.

### GTT predictions

GTT publishes real-time predictions for its urban runs as GTFS Realtime (Protocol Buffers over HTTP). `src/gtt/` reads them and `departures.rs` merges them into OpenTripPlanner's departures (Phase 3, decision N):

1. `DeparturesService::get` asks OpenTripPlanner (its cache) and GTT (`GttRealtime::trip_updates`, its own cache of 20 seconds) in parallel. GTT failing never fails a request: the failure is recorded, GTT is left alone for 30 seconds, and the departures use OpenTripPlanner alone.
2. The feed is indexed by run: GTT's trip id (ours without `gtt:`) and its service date. A departure finds its run through `trip_id` and `service_date`, and its stop through `stopSequence`, which matches GTT's `stop_sequence`.
3. The prediction at the stop is used; when the stop has none, the delay of the last earlier stop carries over. It is ignored when the run's update is older than 3 minutes, when only later stops have updates (already passed), or when the earlier stop has an absolute time (no delay to carry). A time more than 60 minutes from the scheduled one counts as a mismatch and OpenTripPlanner is used.
4. A run cancelled by GTT or by OpenTripPlanner is cancelled. Each departure says where its time comes from in `source` (`gtt`, `otp`, `scheduled`).
5. Every outcome is recorded once per run and stop for the day (`comparison.rs`), so `/api/status` can show how often GTT's predictions are used, set aside, and how far they are from OpenTripPlanner's.

The Rust types for the messages are generated from the official `gtfs-realtime.proto` by `tools/gtfs-rt-codegen` and kept in `src/gtt/proto.rs`, so building the service needs no protobuf compiler.

### GTT alerts

GTT publishes its service alerts (detours, limited lines, suspended stops; about 120 at a time, lasting months) in `alerts.aspx`. `src/gtt/alerts.rs` reads them and links them to stops (Phase 3, decision O):

1. A background task reads the feed at startup and every `GTT_ALERTS_POLL_SECS`, and again at once after new static data. The linked alerts replace the previous ones in memory; when a read fails, the previous ones stay.
2. An alert names stops by GTT's `stop_id` and lines by `route_id`. When it lists stops, it is linked **by stop**: `gtt_stops` turns `2843` into `gtt:39`, and stops missing there (temporary ones) are only counted. When it lists none, or without static data, it is linked **by line**, to every stop whose lines (from the stop index) it names. A line alert lists nearly all the stops of its line; a stop alert lists one stop and every line serving it, so linking it by line would show it at hundreds of stops.
3. Line names and modes come from the stop index (`54U` → `gtt:54U` → "54", as in the departures); a line no stop serves keeps its id without the final `U` or `E`.
4. At each request, only alerts whose period (`communication_period`, or the deprecated `active_period` that GTT uses) includes now are returned; at a stop, two alerts with the same title and text are returned once. The departures of a stop carry its alerts; `/api/alerts` filters by stop and line.

### GTT static data

Only `stops.txt` of GTT's GTFS static archive is used (Phase 3, decision Q): it maps GTT's `stop_id` to its `stop_code`, which is our id (`gtt:<code>`, metro stops included); stations are skipped. `src/gtt/static_data.rs` checks the archive at startup when it was not checked for a day, then every night 10 minutes after `STOPS_SYNC_TIME`, with `If-Modified-Since` set to the `Last-Modified` of the last download: GTT answers `304` without a body when nothing changed, so the 20 MB are downloaded only after GTT rebuilds the archive (about weekly). A download replaces `gtt_stops` in one transaction; an archive without stops is refused. Failures are retried after 5 minutes, 15 minutes, then every hour, like the stop sync.

### Rules

A rule is a **summary** (at a time, the next departures from a stop) or the **tracking** of a run (line and scheduled time at a stop), on some days of the week (Phase 3, section 5). Step 3 stores them and computes their calendar; the scheduler (below) acts on summaries; trackings wait for step 6.

- **Storage** (`src/rules/mod.rs`): one row per rule in `rules`. The fields of each kind are a JSON `config`, read into typed structs (`SummaryConfig`, `TrackConfig`) and checked on the way in; the API returns them as `kind` plus `config`, as stored (decision AG). Missing numbers get their defaults when a rule is created, and the defaults are stored.
- **Checks** (`src/api/rules.rs`): the stop must be in the index and every line must serve it; the stored name of a line is the index's.
- **Calendar** (`src/rules/schedule.rs`): `Schedule::on(date)` says what a rule does on a day: due at a time, not one of its days, a holiday it skips, paused, disabled. `Schedule::next(now)` is the next occurrence. Days are calendar days in Europe/Rome (decision AI); a local time skipped when daylight saving starts moves to the next valid minute, and one repeated when it ends is the first one.
- **Holidays** (`src/rules/holidays.rs`): Italian national holidays (fixed dates, Easter and Easter Monday computed, 4 October from 2026) and 24 June, by their Italian names (decision K).
- **Pause**: `paused_until` is the last day skipped, included; "salta oggi" pauses until today. A pause without a date turns the rule off (decision AH).
- The API adds `today` and `next` to every rule, computed at each request from the clock (decision AJ); `today.run` is the status of today's row in `rule_runs`, or `null`.

### Notifications

Web Push delivers a notification to the phone through the push service of its browser (Phase 3, section 7). Step 4 builds the channel and a test; the scheduler (next section) uses it.

1. **Identity** (`src/notify/vapid.rs`): the server has a P-256 key pair, `VAPID_PRIVATE_KEY`; the public key is derived from it and served at `GET /api/push/key`, which the browser needs to subscribe. `intime vapid-keys` prints a new pair. Without a key (or without `VAPID_SUBJECT`) there is no Web Push: `LogNotifier` only writes notifications to the log, and the key and subscription endpoints answer `push_disabled`.
2. **Subscriptions** (`src/push.rs`, table `push_subscriptions`): one row per device, keyed by its endpoint (the push service URL), with the browser's keys and a label (`iPhone`, `Mac`, ...) taken from the `User-Agent`. Subscribing again updates the row (decision AP). Only `https` endpoints on the default port of a known push service are accepted (decision AM); the keys are checked when they arrive.
3. **Delivery** (`src/notify/webpush.rs`): behind the `Notifier` trait, which `AppState` holds as `Arc<dyn Notifier>`. For each subscription the payload (`{title, body, url, tag?}`, at most 3000 bytes) is encrypted with `aes128gcm` (RFC 8291) and sent with `TTL: 900`, `Urgency: high` and an `Authorization: vapid t=<jwt>, k=<key>` header (RFC 8292), the JWT being signed by the server and valid for one hour. Redirects are not followed.
4. **Outcome**: a 2xx is `sent`; 404 or 410 means the subscription expired and it is deleted (`removed`); anything else is `failed`, logged, and the subscription kept. One device failing never stops the others.
5. **On the phone** (`web/src/sw.ts`): the service worker shows the notification (`push`) and, when it is tapped, focuses the app and navigates to its `url` or opens it (`notificationclick`). A payload it cannot read still shows a generic notification.

6. **Log** (`src/scheduler/store.rs`, table `notifications`): every notification the scheduler sends, and every `POST /api/rules/{id}/test`, is recorded with its `kind` (`summary`, `alert`), title, body, url, channel and the counts of the `SendReport`; `rule_id` becomes null when the rule is deleted. The test of `POST /api/notifications/test` is not recorded (decision AL). `GET /api/notifications` returns it, newest first. Rows older than 90 days are deleted.

### Scheduler

A background task (`src/scheduler/mod.rs`) wakes up every `SCHEDULER_TICK_SECS` (15) and calls `Scheduler::tick(now)`; the time comes from the `Clock`, so tests call `tick` with chosen times (Phase 3, section 6 and [plan/steps/phase-3-step-5.md](plan/steps/phase-3-step-5.md)).

- **Occurrences**: for each summary rule, `Schedule::on` gives the occurrence of today and of yesterday (a 23:58 summary seen at 00:01 still counts). Tracking rules are ignored until step 6.
- **Once per rule and day**: the row `(rule_id, date)` of `rule_runs` (calendar day in Europe/Rome) is inserted as `running` before sending, then set to `sent` or `failed`. After a crash a row left `running` becomes `failed` at startup and is never sent again: a missing summary is better than a duplicate. Changing the time after the rule ran does not send it again that day.
- **Missed runs** (decision M): an occurrence found due is sent if it is at most `MISSED_RUN_GRACE_MINUTES` (5) old; otherwise it is recorded as `skipped`, also for a rule created or moved to a time already past.
- **Upstream failure**: stale departures (up to 2 minutes) are used. On an error the run is `pending` and retried at each tick until the grace ends; then it is `failed` and one notification, "Riepilogo non disponibile", is sent. A `pending` run found after the grace (even if the upstream is back, or after a restart) never sends the summary: it becomes `failed`, with the notice only if it is at most twice the grace late, silently beyond that.
- **The message** (`summary_notification`, shared with `POST /api/rules/{id}/test`): title `🚌 <rule> — <stop> (<code>)`, stop and headsign names title-cased like the web app (`PORTA NUOVA` → `Porta Nuova`, `src/display_name.rs`), one line per departure (`4 → Falchera 07:43 (+2) tra 3'`, `(orario)` when only scheduled, `cancellata`), or "Nessuna partenza nei prossimi N minuti."; `tag: rule-<id>` so today's replaces yesterday's; `url` the stop page. Lines are cut so the payload stays under 3000 bytes.
- **New alerts**: at each tick, when the alerts were read again, the ids of the active alerts are stored in `notified_alerts`. The first read is a baseline: stored as handled, never sent. Right after a rule's summary, the alerts of its stop (and of its lines, or naming no line) first seen after the rule was created and not yet handled are sent: one notification each up to two, a single list above that; each is marked handled right after its own send, so a failure midway never resends the earlier ones. They never go out at night: they wait for an occurrence.
- **Upkeep**: at the first tick of each day, `notifications` and `rule_runs` older than 90 days are deleted.

Known limits are listed in [development/18-scheduler.md](development/18-scheduler.md#pitfalls).

### Stop index

- A background task downloads all stops (one query, about 5 MB) on startup when the index is missing or older than 24 hours, then every night at `STOPS_SYNC_TIME`. On failure it retries after 5 minutes, 15 minutes, then every hour.
- Each sync replaces the whole index in one transaction: a search sees either the old or the new data. An empty upstream response is rejected and the old index kept.
- For each stop the index stores the original name, a normalized name and a normalized search text (name + description), and an FTS5 trigram index on the search text.
- **Normalization**: lowercase, accents removed, punctuation removed, abbreviations expanded (`c.so` → `corso`, `v.` → `via`, `bv.` → `bivio`, `m.to` → `monferrato`, ...; the list comes from the most frequent abbreviations in real names), saint variants unified (`santa`, `sant'`, `s.` → `san`). The same normalization is applied to queries.
- **Matching**: words of 3 or more letters use the trigram index (partial words match); shorter words must start a word; a code or id matches directly.
- **Nearby**: a bounding box on an indexed lat/lon narrows the candidates, then the exact distance (haversine) filters and sorts them.
- The stop code shown is the upstream `code`, or the part of the id after `:` when missing (GTT metro stops).

### Favorites

Stored in SQLite with a 0-based `position` kept contiguous: inserts go at the end, moves shift the others, deletes close the gap, each in a transaction. A favorite stores only the stop id; its details are read from the index at every request, so a stop removed by a sync shows up as `missing` instead of disappearing.

`GET /api/favorites/departures` returns every favorite with its next departures, for the web app's Home screen. The stops are read in parallel (one Tokio task each, through the departures cache, so two favorites of one stop make one upstream call); a stop that fails gives that favorite an `error` and leaves the others alone.

### Robustness

- **Retry**: one retry after 300 ms on connection errors and HTTP 502–504. Not on timeouts (they already took the whole timeout), GraphQL errors or invalid responses (retrying would give the same result). GraphQL errors arrive with HTTP 500, so the body is checked before the status.
- **Upstream health**: every upstream call, including syncs, records its outcome; `/api/status` shows it. A GraphQL validation error usually means the upstream schema changed and the queries need fixing.
- **Token**: compared in constant time; never logged.

### Data

One SQLite file in `DATA_DIR` (`gtt.db`, plus `gtt.db-wal` and `gtt.db-shm` in WAL mode). Migrations in `migrations/` are compiled into the binary and applied at startup. Tables: `stops`, `stops_fts`, `sync_state` (last stop sync and last check of GTT's static data, with its version), `favorites`, `gtt_stops` (GTT's stop ids and ours), `rules` (kind-specific fields as JSON in `config`), `push_subscriptions` (one row per device that enabled notifications: endpoint, the browser's keys, label), `rule_runs` (one row per summary rule and day: status, due time), `notifications` (log of what was sent), `notified_alerts` (alert ids seen, and whether they were handled).

## Code map

| File | Responsibility |
|------|----------------|
| `src/main.rs` | startup, wiring, graceful shutdown, `healthcheck` and `vapid-keys` subcommands |
| `src/config.rs` | environment variables → `Config`, with validation |
| `src/time.rs` | `serviceDay` + seconds → local time; `Clock` trait (real or fixed for tests) |
| `src/db.rs` | SQLite pool and migrations |
| `src/upstream/mod.rs` | GraphQL client: queries, errors, retry |
| `src/upstream/queries.rs`, `types.rs` | query texts and response shapes |
| `src/upstream/health.rs` | outcome of recent upstream calls |
| `src/departures.rs` | departures: conversion, sorting, filtering, cache, stale fallback, merge with GTT's predictions |
| `src/gtt/mod.rs` | GTT's real-time client: reading, cache, retry, pause after a failure, health |
| `src/gtt/trip_updates.rs` | trip updates indexed by run; the prediction for one stop of a run |
| `src/gtt/comparison.rs` | today's comparison between GTT and OpenTripPlanner |
| `src/gtt/alerts.rs` | GTT's alerts: reading in the background, linking to stops and lines, active periods |
| `src/gtt/static_data.rs` | GTT's static data: nightly conditional download, `stops.txt` → `gtt_stops` |
| `src/gtt/proto.rs` | GTFS Realtime types, generated (do not edit) |
| `tools/gtfs-rt-codegen/` | the generator of `src/gtt/proto.rs`, a separate Cargo project |
| `src/stops/mod.rs` | stop index storage, `Feeds`, line sorting |
| `src/stops/normalize.rs` | name normalization and abbreviations |
| `src/stops/search.rs` | search ranking and nearby stops |
| `src/stops/sync.rs` | nightly sync scheduling and retries |
| `src/favorites.rs` | favorites storage and ordering |
| `src/rules/mod.rs` | rules: typed config, storage, days and times in JSON |
| `src/rules/schedule.rs` | a rule's calendar: what it does on a day, its next occurrence, local times across daylight saving |
| `src/rules/holidays.rs` | Italian public holidays, Easter |
| `src/notify/mod.rs` | `Notification` and its payload, `SendReport`, the `Notifier` trait |
| `src/notify/vapid.rs` | the VAPID key: parsing, generation, the signed `Authorization` header |
| `src/notify/webpush.rs` | `WebPushNotifier`: encryption, sending, removing expired subscriptions |
| `src/notify/log.rs` | `LogNotifier`, used without a VAPID key |
| `src/scheduler/mod.rs` | the tick, occurrences, missed runs, summary and alert messages, `spawn` |
| `src/scheduler/store.rs` | `SchedulerStore`: `rule_runs`, the notifications log, `notified_alerts` |
| `src/push.rs` | push subscriptions: storage, checks of a browser's keys, which push services are allowed |
| `src/api/mod.rs` | router, shared `AppState`, token layer |
| `src/api/stops.rs` | search, nearby, details and departures handlers |
| `src/api/favorites.rs` | favorites handlers and input cleaning; favorites with their departures |
| `src/api/alerts.rs` | `/api/alerts` |
| `src/api/push.rs` | VAPID public key, subscribe and unsubscribe |
| `src/api/notifications.rs` | `/api/notifications/test`, `/api/notifications` |
| `src/api/rules.rs` | rules handlers: checks, defaults, pause and resume, today and next, `test` and `runs` |
| `src/api/status.rs`, `health.rs` | `/api/status`, `/health` |
| `src/api/auth.rs` | `API_TOKEN` middleware |
| `src/api/error.rs` | error codes and JSON error responses |
| `src/web.rs` | embedded web app: file lookup, fallback to `index.html`, cache headers |
| `web/src/main.ts`, `App.svelte` | web app entry point; one screen per route, bottom bar |
| `web/src/app.css` | design tokens (light and dark) and base styles |
| `web/src/lib/router.ts`, `navigation.svelte.ts` | paths ↔ screens; current route, history, link clicks |
| `web/src/lib/api.ts` | API types and calls, errors and their Italian messages |
| `web/src/lib/format.ts`, `poll.ts`, `departures.ts` | times, names and matches; reloading while visible; line filters |
| `web/src/lib/favorites.ts` | the favorite behind the star on a stop |
| `web/src/lib/push.ts`, `pushPayload.ts`, `screens/Notifications.svelte` | the Notifiche screen: state, permission and subscription; reading a push's payload (shared with the service worker) |
| `web/src/lib/location.ts` | the phone's position, asked to the browser, with Italian errors |
| `web/src/sw.ts`, `web/src/lib/update.svelte.ts` | service worker; new versions and "Aggiorna" |
| `web/src/lib/lastData.ts` | last favorites and departures, for offline use |
| `web/public/`, `web/pwa-assets.config.ts` | icon source and generated PNG icons |
| `web/src/lib/alerts.ts`, `components/AlertsBanner.svelte` | GTT's alerts on the stop screen: banner closed by default, alerts of the lines on screen |
| `web/src/lib/` | other web app modules: what the info screen says about the status, line types and delays, bottom bar entries |
| `web/src/lib/components/` | shared components: icons, line badge, departure row, loaders, header, bottom bar |
| `web/src/screens/` | one component per screen, plus the development-only design page |
| `web/vite.config.ts` | build, development proxy to the backend, Vitest |

Reading order for a first look: `main.rs` → `api/mod.rs` → `api/stops.rs` → `departures.rs` → `upstream/mod.rs` → `stops/` → `favorites.rs`.

## Known limitations

- **Unofficial upstream**: it can change or disappear. `/api/status` and the logs show failures; a schema change shows up as a GraphQL validation error.
- **No vehicle positions and no service alerts** in the upstream API. Both come from GTT's open data in Phase 3 ([plan/phase-3.md](plan/phase-3.md)); alerts are done (step 2), positions come with trip tracking.
- **GTT's alerts** name some temporary stops missing from GTT's static data (52 on 2026-11-04): an alert listing only such stops is shown by `/api/alerts` but at no stop. An alert naming neither stops nor lines (1 on 2026-11-04) is shown only by `/api/alerts` without filters. Their periods often last a year, so they do not tell when an alert stops mattering.
- **GTT's predictions cover urban runs only**, the next few stops of each run, and arrivals only (used as the departure time). Runs not yet started have no prediction. On 2026-11-04 they were mostly earlier than OpenTripPlanner's, by about 3 minutes (median); `/api/status` keeps counting to judge them.
- **Search**: stops whose name lacks the searched words but whose description has them rank last (e.g. `pza castello` lists towns' "P.ZA CASTELLO" stops before Turin's "CASTELLO").
- **Stale data** covers only 2 minutes of upstream failure; after that, departures return an error.
- **Release builds on the Mac** fail (the linker produces a malformed `sqlx-macros` library); debug builds and Docker builds work.
- **Single user**: no accounts; favorites are shared by whoever has the token.

Phase 2, the web app, is done (`v0.2.0`, [plan/phase-2.md](plan/phase-2.md)). Phase 3 is in progress: GTT open data, rules, trip tracking and Web Push notifications ([plan/phase-3.md](plan/phase-3.md)); GTT's predictions (step 1), alerts (step 2) and rules without sending (step 3), Web Push (step 4: the channel, subscriptions and a test) and the scheduler with summaries (step 5) are done. Then Phase 4, Cloudflare Access and the first deployment ([plan/phase-4.md](plan/phase-4.md)).
