# API reference

How to run the service and use every endpoint. For settings see [configuration.md](configuration.md); for the internals see [../architecture.md](../architecture.md).

## Running it

| Where | How |
|-------|-----|
| Locally, from source | `cargo run`, then `http://localhost:8080`. Data goes to `./data`. The web app is there too once built (`npm ci && npm run build` in `web/`) |
| Locally, with Docker | build the image (`scripts/build-image.sh`), `cp .env.example .env`, `docker compose up -d`, then `http://localhost:8100` |
| Home server | not deployed yet, planned in Phase 4; steps in [deploy.md](deploy.md) |

On first start the service downloads all stops (2–3 seconds). Until then the stop endpoints answer `503 index_not_ready`.

Paths outside `/api/*` and `/health` serve the web app. All API responses are JSON; times are ISO 8601 with time zone (`2026-10-29T17:23:53+01:00`).

## Authentication

If `API_TOKEN` is set, every `/api/*` request needs the header `Authorization: Bearer <token>`. `/health` never does. The examples below assume:

```bash
export BASE=http://localhost:8100
export AUTH="Authorization: Bearer <your token>"
```

## Typical flow

1. Find the stop code: by name (`/api/stops?q=`) or near you (`/api/stops/nearby`).
2. Read its departures (`/api/stops/{stop}/departures`), optionally filtered by line.
3. Save it as a favorite to find it again quickly.

## Find a stop by name or code

```bash
curl -s -H "$AUTH" "$BASE/api/stops?q=porta%20nuova&limit=5"
```

| Parameter | Default | Notes |
|-----------|---------|-------|
| `q` | required | name, part of a name, or stop code; 1–100 characters |
| `feeds` | `DEFAULT_FEEDS` (`gtt`) | `all`, or a list such as `gtt,2` (`2` Trenitalia, `3` Arriva, `4` regional buses) |
| `limit` | 20 | 1–100 |

Response: `{"stops": [stop, ...]}`, where each stop is:

```json
{
  "id": "gtt:39", "feed": "gtt", "code": "39", "name": "PORTA NUOVA",
  "description": "PORTA NUOVA - M1 - STAZIONE",
  "lat": 45.06256, "lon": 7.67898, "mode": "BUS",
  "lines": [{"id": "gtt:6U", "name": "6", "mode": "BUS"}, {"id": "gtt:9U", "name": "9", "mode": "TRAM"}, {"id": "gtt:9BU", "name": "9B", "mode": "BUS"}]
}
```

Search tips:
- abbreviations are understood both ways: `c.so vittorio` = `corso vittorio`, `p.za` = `piazza`, `s.anna` = `santa anna` = `sant'anna`;
- partial words work (`nuov`), and the order of words does not matter;
- a code (`39`) or full id (`gtt:39`) comes first;
- results are ordered: exact code, exact name, name starting with the text, name containing it, description only; then GTT first, shorter names first.

## Find stops near a point

```bash
curl -s -H "$AUTH" "$BASE/api/stops/nearby?lat=45.0712&lon=7.6853&radius=200"
```

| Parameter | Default | Notes |
|-----------|---------|-------|
| `lat`, `lon` | required | decimal degrees |
| `radius` | 300 | meters, 1–2000 |
| `feeds`, `limit` | as above | |

Same stop objects, nearest first, each with `"distance_m"`.

## Stop details

```bash
curl -s -H "$AUTH" "$BASE/api/stops/39"
```

`{stop}` is a GTT code (`39`) or a full id (`gtt:39`, `2:S00219`). Returns one stop object, or `404 stop_not_found`.

## Departures

```bash
curl -s -H "$AUTH" "$BASE/api/stops/39/departures?lines=4,15&minutes=30&limit=10"
```

| Parameter | Default | Notes |
|-----------|---------|-------|
| `lines` | all | comma-separated line names, case-insensitive (`9b` = `9B`) |
| `minutes` | 60 | how far ahead, 1–120 |
| `limit` | 20 | 1–100 |

```json
{
  "stop": {"id": "gtt:39", "code": "39", "name": "PORTA NUOVA"},
  "generated_at": "2026-10-29T17:23:23+01:00",
  "fetched_at": "2026-10-29T17:23:20+01:00",
  "stale": false,
  "departures": [{
    "line": "67", "mode": "BUS", "headsign": "MONCALIERI, TESTONA - PIAZZA MARCONI",
    "scheduled": "2026-10-29T17:24:00+01:00", "expected": "2026-10-29T17:23:53+01:00",
    "delay_seconds": -7, "realtime": true, "canceled": false,
    "trip_id": "gtt:28459721U", "source": "otp", "minutes": 0
  }],
  "alerts": [{
    "id": "6aa2a2bc22bb480c377bee29", "title": "Linea 9 deviata in entrambe le direzioni.",
    "description": "Da lunedì 14 settembre 2026 e sino a nuove comunicazioni.\nDirezione piazza Stampalia: ...",
    "url": "http://www.gtt.to.it/cms/percorari/urbano",
    "lines": [{"id": "gtt:9U", "name": "9", "mode": "TRAM"}],
    "start": "2026-09-15T05:00:00+02:00", "end": "2027-09-15T05:00:00+02:00"
  }]
}
```

- `expected` is the time to rely on; it equals `scheduled` when `realtime` is `false`.
- `source` says where `expected` comes from: `gtt` (GTT's own prediction, urban runs only), `otp` (OpenTripPlanner's real time) or `scheduled` (the timetable). GTT's prediction is used when it is less than 3 minutes old and within 60 minutes of the scheduled time; otherwise OpenTripPlanner's. A run cancelled by either source has `canceled: true`.
- `minutes` is rounded down and never negative.
- `stale: true` means the upstream is failing and this is the last good data (at most 2 minutes old, see `fetched_at`).
- `alerts` are GTT's active alerts for the stop, whatever the `lines` filter, in the shape of [GTT alerts](#gtt-alerts); empty when GTT's data is disabled.

## GTT alerts

```bash
curl -s -H "$AUTH" "$BASE/api/alerts?stop=39&line=9"
```

GTT's active service alerts (detours, limited lines, suspended stops), read from GTT every 5 minutes.

| Parameter | Default | Notes |
|-----------|---------|-------|
| `stop` | none | the alerts of a stop: those that list it, and those that list no stop but name a line serving it. A code (`39`) or a full id (`gtt:39`); `stop_not_found` when the stop is not in the index |
| `line` | none | the alerts naming a line (`9`, `68+`), case-insensitive; with `stop`, those of that stop naming the line |

Without parameters, every active alert, including those that name neither a stop nor a line.

```json
{
  "generated_at": "2026-11-04T16:24:10+01:00",
  "read_at": "2026-11-04T16:22:52+01:00",
  "alerts": [{"id": "6aa2a2bc22bb480c377bee29", "title": "Linea 9 deviata in entrambe le direzioni.", "lines": [{"id": "gtt:9U", "name": "9", "mode": "TRAM"}], "...": "..."}]
}
```

- `read_at`: when the alerts were read from GTT; `null` before the first read or when GTT's data is disabled (`alerts` is then empty). When a read fails, the last alerts stay.
- Each alert: `id`, `title` and `description` (GTT's Italian text; the description keeps its line breaks), `url` (GTT's page, `http` or `https` only, else `null`), `lines` (sorted, with name and mode from the stop index; `mode` is `null` for a line no stop serves), and `start` and `end` of the active period that includes now (`null` when open-ended).
- An alert is shown only while one of its periods includes now. At a stop, two alerts with the same title and text are shown once.

## Favorites

```bash
curl -s -H "$AUTH" "$BASE/api/favorites"
```

Lists favorites in order; each has `id`, `stop_id`, `label`, `lines`, `position`, `created_at`, the full `stop`, and `missing` (true if a sync removed the stop).

```bash
curl -s -H "$AUTH" -H "Content-Type: application/json" -X POST "$BASE/api/favorites" -d '{"stop": "39", "label": "Home → station", "lines": ["9B", "52"]}'
```

Adds a favorite at the end (201). The stop must exist. `label` (at most 100 characters) and `lines` (at most 20) are optional.

```bash
curl -s -H "$AUTH" -H "Content-Type: application/json" -X PATCH "$BASE/api/favorites/1" -d '{"position": 0, "label": null}'
```

Changes only the fields given: `label` (`null` clears it), `lines`, `position` (0-based; past the end moves it last).

```bash
curl -s -H "$AUTH" -X DELETE "$BASE/api/favorites/1"
```

Deletes it (204); the following favorites move up.

```bash
curl -s -H "$AUTH" "$BASE/api/favorites/departures?limit=3"
```

Every favorite, in order, with its next departures: what the web app's home screen shows, in one call.

| Parameter | Default | Notes |
|-----------|---------|-------|
| `limit` | 3 | departures per favorite, 1–10 |
| `minutes` | 60 | how far ahead, 1–120 |

```json
{
  "generated_at": "2026-10-29T17:23:53+01:00",
  "favorites": [{
    "id": 1, "stop_id": "gtt:39", "label": "Casa", "lines": ["67"], "position": 0,
    "created_at": "2026-10-29T16:23:53Z", "stop": {"id": "gtt:39", "code": "39", "name": "PORTA NUOVA"},
    "missing": false, "stale": false, "error": null,
    "departures": [{"line": "67", "minutes": 0, "expected": "2026-10-29T17:23:53+01:00"}]
  }]
}
```

- Each favorite is shown as in `GET /api/favorites` (`stop` abridged here) plus `departures` (same fields as the stop departures, filtered by the favorite's `lines`), `stale` and `error`. GTT's alerts are not included: ask `/api/alerts?stop=...` for them.
- When one stop fails, that favorite has `departures: []` and `error: {"code", "message"}` (codes as in [Errors](#errors)); the others are unaffected and the response is still 200.
- Stops are read in parallel and through the same cache as `/api/stops/{stop}/departures`.

## Rules

Rules send notifications on their own: a **summary** of the next departures at a fixed time, or the tracking of one **run** every matching day ([plan](../plan/phase-3.md#5-rule-model)). Summaries are sent by the scheduler (Phase 3, step 5, see [architecture](../architecture.md#scheduler)); tracking comes with step 6, and until then a tracking rule is stored and has its calendar but sends nothing.

```bash
curl -s -H "$AUTH" -H "Content-Type: application/json" -X POST "$BASE/api/rules" -d '{"name": "Casa → lavoro", "kind": "summary", "stop": "39", "days": ["mon", "tue", "wed", "thu", "fri"], "config": {"time": "07:40", "lines": ["9B", "67"]}}'
```

Creates a rule (201). The stop must exist, and every line must serve it (case does not matter: `9b` is stored as `9B`, as the stop index spells it).

| Field | Default | Notes |
|-------|---------|-------|
| `name` | | 1–100 characters |
| `kind` | | `summary` or `track`; cannot change later |
| `stop` | | code (`39`) or id (`gtt:39`) |
| `days` | | `mon`, `tue`, `wed`, `thu`, `fri`, `sat`, `sun`; at least one, stored Monday first |
| `skip_holidays` | `true` | skip Italian national holidays (with Easter Monday and, from 2026, 4 October) and 24 June, San Giovanni |
| `enabled` | `true` | |
| `config` | | the fields of `kind`, below; unknown fields are refused |

`config` of a `summary`:

| Field | Default | Notes |
|-------|---------|-------|
| `time` | | `HH:MM`, Europe/Rome |
| `lines` | `[]` | at most 20; empty means all lines |
| `window_minutes` | 30 | departures in the next N minutes, 5–120 |
| `max_departures` | 6 | 1–20 |

`config` of a `track`:

| Field | Default | Notes |
|-------|---------|-------|
| `line` | | one line serving the stop |
| `scheduled_time` | | `HH:MM`: the run's scheduled departure at the stop |
| `tolerance_minutes` | 2 | how far the run may be from `scheduled_time` in the timetable, 0–10 |
| `watch_from_minutes` | 30 | tracking starts N minutes before, 10–120, at least the longest reminder |
| `alert_before_minutes` | `[10, 5]` | reminders before the expected departure, 1–60 each, at most 4, stored longest first; `[]` for none |
| `delay_alert_minutes` | 3 | alert when the delay reaches N minutes, 1–30 |

Missing numbers get their defaults, which are stored in the rule: a later change of a default does not change existing rules.

```json
{
  "id": 1, "name": "Casa → lavoro", "enabled": true, "stop_id": "gtt:39",
  "days": ["mon", "tue", "wed", "thu", "fri"], "date": null, "skip_holidays": true, "paused_until": null,
  "kind": "summary",
  "config": {"time": "07:40", "lines": ["9B", "67"], "window_minutes": 30, "max_departures": 6},
  "created_at": "2026-11-05T06:00:00Z", "updated_at": "2026-11-05T06:00:00Z",
  "stop": {"id": "gtt:39", "code": "39", "name": "PORTA NUOVA"}, "missing": false,
  "today": {"status": "due", "at": "2026-11-05T07:40:00+01:00", "run": {"status": "sent", "updated_at": "2026-11-05T06:40:02Z"}},
  "next": "2026-11-05T07:40:00+01:00"
}
```

- `stop` is abridged here; `missing` is true when a sync removed the stop, as for favorites. `date` is set only for a run tracked on the fly (step 6), which this endpoint does not create.
- `today` says what the rule does today in Europe/Rome, by `status`: `due` (at `at`, even when that time has passed), `not_scheduled` (not one of its days), `holiday` (with the `holiday` name in Italian), `paused` (with `until`), `disabled`.
- `today.run` is how today's occurrence went, `null` while there is no run yet: `status` is `running` (being sent), `pending` (the upstream failed, retried until the grace ends), `sent`, `skipped` (found too late, or the rule was created or moved to a time already past) or `failed`.
- `next` is the next occurrence from now, `null` when the rule is off. Times that do not exist when daylight saving starts move to the next valid minute (02:30 → 03:00); times that happen twice when it ends are the first one.
- The days are calendar days in Rome: a rule on `sat` at `00:30` runs in the night between Friday and Saturday.

```bash
curl -s -H "$AUTH" "$BASE/api/rules"
```

```bash
curl -s -H "$AUTH" "$BASE/api/rules/1"
```

All rules, oldest first (`{"rules": [...]}`), or one rule, each as above.

```bash
curl -s -H "$AUTH" -H "Content-Type: application/json" -X PATCH "$BASE/api/rules/1" -d '{"days": ["mon", "wed"], "config": {"time": "07:50"}}'
```

Changes only the fields given: `name`, `stop`, `days`, `skip_holidays`, `enabled`, and `config`, which is **replaced whole** (missing numbers take their defaults again). `kind` is accepted only when unchanged. When `stop` or `config` changes, the lines are checked against the stop again.

```bash
curl -s -H "$AUTH" -H "Content-Type: application/json" -X POST "$BASE/api/rules/1/pause" -d '{"until": "2026-11-16"}'
```

Pauses the rule until a date, **that day included** ("in pausa fino al 16 novembre"); "salta oggi" is `until` today. `until` goes from today to one year ahead; the rule is also turned on. Without a body (or without `until`) the rule is turned off (`enabled: false`).

```bash
curl -s -H "$AUTH" -X POST "$BASE/api/rules/1/resume"
```

Turns the rule on and ends its pause.

```bash
curl -s -H "$AUTH" -X DELETE "$BASE/api/rules/1"
```

Deletes it (204). Its log of notifications is kept, without the link to the rule.

```bash
curl -s -H "$AUTH" -X POST "$BASE/api/rules/1/test"
```

Sends the summary of a summary rule now, whatever its days, pause or `enabled`, and records it in the log; it writes no run, so the real one still goes out. The answer is the one of the notification test plus what was sent:

```json
{"channel": "webpush", "sent": 1, "removed": 0, "failed": 0, "title": "🚌 Casa → lavoro — PORTA NUOVA (39)", "body": "9B → Falchera 07:43 (+2) tra 3'"}
```

`422` `not_a_summary` for a tracking rule; `404` `rule_not_found`; `502` or `504` when the departures cannot be read.

```bash
curl -s -H "$AUTH" "$BASE/api/rules/1/runs"
```

`{"runs": [...]}`: the last 30 runs of the rule, newest day first, each `{"rule_id", "date", "status", "due_at", "state", "updated_at"}` (`date` is the calendar day in Rome, `state` is `null` for summaries). `404` for an unknown rule.

## Push notifications

Web Push delivers notifications to the phone through the browser's push service, even when InTime is closed ([plan](../plan/phase-3.md#7-notifications-web-push)). Step 4 of Phase 3 adds the subscription and a test; summaries that send come with the scheduler (step 5), trackings with step 6. It needs `VAPID_PRIVATE_KEY` and `VAPID_SUBJECT` ([configuration.md](configuration.md#web-push-keys)); without them the four endpoints below still exist, but the first two answer `503` with `push_disabled` and the test only writes to the log.

The web app does all this from the Notifiche screen: it reads the key, asks the browser for permission and subscribes. The calls are shown for completeness; a subscription needs a real browser's keys, so only the key and the test can be tried with `curl`.

```bash
curl -s -H "$AUTH" "$BASE/api/push/key"
```

`{"public_key": "BK..."}`: the VAPID public key (base64url, 65 bytes), which the browser needs as `applicationServerKey` to subscribe. `503` `push_disabled` when the server has no VAPID key.

```bash
curl -s -H "$AUTH" -H "Content-Type: application/json" -X POST "$BASE/api/push/subscriptions" -d '{"endpoint": "https://fcm.googleapis.com/fcm/send/...", "expirationTime": null, "keys": {"p256dh": "BN...", "auth": "..."}}'
```

Saves this device's subscription: the browser's own `PushSubscription.toJSON()`. `expirationTime` is accepted and ignored; any other unknown field is refused. The same `endpoint` again updates its keys and label and keeps its creation time, so the web app sends it every time the screen opens. Answers `201` for a new subscription, `200` for an existing one:

```json
{"id": 1, "label": "Mac", "created_at": "2026-11-05T11:40:12Z"}
```

`label` is the kind of device, from the `User-Agent`: `iPhone`, `iPad`, `Android`, `Mac`, `Windows`, or `null`. `400` `invalid_request` when:

- the `endpoint` is not `https` on the default port, or its host is not a known push service (`fcm.googleapis.com`, `push.apple.com`, `push.services.mozilla.com`, `notify.windows.com`, or a subdomain of them), has credentials in it or is longer than 2048 characters;
- `keys.p256dh` is not an uncompressed P-256 point or `keys.auth` is not 16 bytes (base64url).

```bash
curl -s -H "$AUTH" -H "Content-Type: application/json" -X DELETE "$BASE/api/push/subscriptions" -d '{"endpoint": "https://fcm.googleapis.com/fcm/send/..."}'
```

Forgets a subscription (204), also when the endpoint is unknown.

```bash
curl -s -H "$AUTH" -X POST "$BASE/api/notifications/test"
```

Sends "🔔 Notifica di prova" to every subscribed device (no body needed):

```json
{"channel": "webpush", "sent": 2, "removed": 0, "failed": 0}
```

- `channel` is `webpush`, or `log` when the server has no VAPID key: the notification is then only written to the log and `sent` is 1.
- `sent`: devices the push service accepted. `removed`: subscriptions deleted because the push service said they no longer exist (404 or 410). `failed`: devices that could not be reached (the push service refused or did not answer); their subscriptions are kept.
- The response is 200 even when `sent` is 0: look at the counts.

```bash
curl -s -H "$AUTH" "$BASE/api/notifications?limit=20"
```

The log of what the scheduler and `POST /api/rules/{id}/test` sent, newest first (`limit` from 1 to 200, default 50; `400` otherwise). The test above is not in it. Rows older than 90 days are deleted.

```json
{"notifications": [{"id": 7, "rule_id": 1, "rule_name": "Casa → lavoro", "kind": "summary", "title": "🚌 Casa → lavoro — PORTA NUOVA (39)", "body": "9B → Falchera 07:43 (+2) tra 3'", "url": "/fermata/39", "channel": "webpush", "sent": 1, "removed": 0, "failed": 0, "sent_at": "2026-11-05T06:40:02Z"}]}
```

`kind` is `summary` (also the "Riepilogo non disponibile" notice) or `alert` (a new GTT alert at the rule's stop). `rule_name` is `null` when the rule was deleted.

## Status and health

```bash
curl -s "$BASE/health"
```

`{"status": "ok", "version": "0.2.0"}` while the process runs. Used by Docker; never needs the token.

```bash
curl -s -H "$AUTH" "$BASE/api/status"
```

Version, current time, `upstream` (`ok`, last success, last failure and error, consecutive failures), `stop_index` (`synced_at`, `count`) and `gtt`. The first place to look when something seems wrong.

`gtt` is `null` when GTT's data is disabled (`GTT_REALTIME_URL` empty). Otherwise:

```json
"gtt": {
  "trip_updates": {"ok": true, "last_success": "2026-11-04T09:55:01Z", "last_failure": null, "last_error": null, "consecutive_failures": 0},
  "feed": {"runs": 388, "newest_update": "2026-11-04T09:55:01Z"},
  "comparison": {"date": "2026-11-04", "used": 10, "mismatches": 0, "compared": 9, "median_difference_seconds": 185},
  "alerts": {
    "feed": {"ok": true, "last_success": "2026-11-04T15:22:51Z", "...": "..."},
    "read": {"read_at": "2026-11-04T15:22:52Z", "alerts": 121, "unknown_stops": 52}
  },
  "static_data": {
    "download": {"ok": true, "last_success": "2026-11-04T15:22:51Z", "...": "..."},
    "data": {"checked_at": "2026-11-04T15:22:51Z", "version": "2026-11-01T02:26:59Z", "stops": 7051}
  }
}
```

- `trip_updates`: outcome of the recent reads of GTT's trip updates, in the same shape as `upstream`.
- `feed`: the last trip updates read (`null` until the first success): runs in the feed and the most recent update of any run.
- `alerts`: `feed` is the outcome of the recent reads of the alerts; `read` (`null` until the first success) counts the alerts in the feed, active or not, and `unknown_stops`, the GTT stop ids named by alerts but missing from the static data (temporary stops, or no static data yet).
- `static_data`: GTT's `stops.txt`, which links alerts to stops; `null` when disabled (`GTT_GTFS_URL` empty). `download` is the outcome of the recent checks; `data` (`null` until the first download) has the last successful check, `version` (when GTT built the archive, its `Last-Modified`) and the number of stops.
- `comparison`: today (Europe/Rome), one entry per run and stop, so reloading a page does not count twice. `used`: departures that took GTT's prediction; `mismatches`: GTT predictions set aside as implausible; `compared`: used departures where OpenTripPlanner had a prediction too; `median_difference_seconds`: median of their absolute differences.

## Errors

Always `{"error": {"code": "...", "message": "..."}}`:

| HTTP | `code` | When |
|------|--------|------|
| 400 | `invalid_request` | bad parameter or body (the message says which) |
| 401 | `unauthorized` | token missing or wrong |
| 404 | `stop_not_found`, `favorite_not_found`, `rule_not_found` | unknown stop, favorite or rule |
| 422 | `not_a_summary` | `POST /api/rules/{id}/test` on a tracking rule |
| 500 | `internal_error` | database problem (details in the logs) |
| 502 | `upstream_error` | upstream error and no recent data to fall back on |
| 503 | `index_not_ready` | stop index not downloaded yet |
| 503 | `push_disabled` | Web Push is not configured on the server (no `VAPID_PRIVATE_KEY`) |
| 504 | `upstream_timeout` | upstream too slow |
