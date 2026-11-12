# Phase 3, step 5: scheduler and summaries

Size: large (persistence, background task, shared contract for step 6). Plan for the implementation session; open questions answered by the user on 2026-11-12 (decisions AS–AV in phase-3.md).

Scope (phase-3.md section 14): the scheduler tick, `rule_runs` (decision AF), summary rules, `POST /api/rules/{id}/test`, notification of new alerts for a rule's stop and lines, the `notifications` log and `GET /api/notifications` (decision AL). Not in scope: tracking (step 6), the rules UI (step 7), health alerts (step 8).

## Verified facts

- `Schedule::on(date) -> Day` (`Due { at }`, `NotScheduled`, `Holiday`, `Paused`, `Disabled`) and `Schedule::next(now)` exist in `src/rules/schedule.rs:48-90`; `local_date(now)` and `local_time(date, time)` at `:94-101` already handle DST gaps and repeats. `Rule::schedule()` uses `config.time()`: a summary's `time`, a tracking's `scheduled_time` (`src/rules/mod.rs:96-130`).
- `RuleStore` has `list`, `get`, `create`, `update`, `delete` (`src/rules/mod.rs:200-300`); the rule view adds `today` and `next` computed at each request (`src/api/rules.rs:68-71, 309-316`).
- `SummaryConfig { time, lines, window_minutes, max_departures }` (`src/rules/mod.rs:45-51`); lines are stored with the index's spelling.
- Departures: `DeparturesService::get(stop_id) -> Fetched { data, stale }`, cached, GTT merged (`src/departures.rs:242-300`); `StopDepartures::select(&Filter { lines, window, limit }, now)` (`:106-122`) filters on expected time and lines, case-insensitive. `Departure` has `line`, `headsign`, `scheduled`, `expected`, `delay_seconds`, `realtime`, `canceled`, `source`.
- Alerts: `AlertsService::current() -> Arc<Alerts>`, refreshed every `GTT_ALERTS_POLL_SECS` (`src/gtt/alerts.rs:345-395`); `Alerts::for_stop(stop_id, stop_lines, now)` returns the active alerts of a stop, deduplicated by title and text (`:235-260`); `Alert::names_line` (`:54`); `Alert.id` is GTT's `entity.id`. `AppState.alerts` is `None` when GTT's data is off.
- **Alert ids are stable**: the fixture of 2026-11-04 (119 alerts) and the live feed of 2026-11-12 (133) share 105 ids (measured with a protobuf read of both). About 3–4 new alerts a day network-wide.
- Notifier: `Arc<dyn Notifier>` with `channel()` and `send(&Notification) -> SendReport { sent, removed, failed }`; `Notification { title, body, url, tag }`, payload ≤ 3000 bytes (`src/notify/mod.rs`). `LogNotifier` reports `sent: 1` (`src/notify/log.rs`).
- `Clock` trait and `FixedClock` in `src/time.rs`; background tasks are spawned in `src/main.rs:133-175` (stop sync, alerts, static data).
- Migrations `0001`–`0005` via `sqlx::migrate!` (`src/db.rs:12`); `rule_runs`, `notifications`, `notified_alerts` do not exist yet. `SCHEDULER_TICK_SECS` and `MISSED_RUN_GRACE_MINUTES` are not in `src/config.rs` yet.
- Stop paths in the web app are `/fermata/<code>` (`web/src/lib/router.ts:46`).

## Decisions

1. **Tick**: one task (`src/scheduler/mod.rs`), `tokio::time::interval(SCHEDULER_TICK_SECS)` with `MissedTickBehavior::Skip`, calling `Scheduler::tick(now)`; tests call `tick` directly with chosen times. Occurrences handled one after another within a tick.
2. **Occurrences** come from `Schedule::on` for **today and yesterday** (a 23:58 summary seen at 00:01 is still within the grace). An occurrence earlier than the rule's `created_at` is never sent (yesterday's is not even recorded). Summary rules only; tracking rules are ignored until step 6.
3. **Missed (decision M)**: due and no `rule_runs` row: if `now - at <= MISSED_RUN_GRACE_MINUTES`, send; otherwise insert `skipped` and log. A rule created or moved to a time already past today gets `skipped`, not a late summary.
4. **Idempotency**: the row `(rule_id, date)` is inserted (`running`) **before** sending, then set to `sent` or `failed`. A row left `running` by a crash becomes `failed` at startup and is never sent again (a `pending` row is left alone: it still waits for its retry): a missing summary is better than a duplicate or a late one. One occurrence per rule and day: changing the time after it ran does not send again that day.
5. **`rule_runs`** (migration `0006`): `rule_id` (FK, cascade), `date` (**calendar day** in Europe/Rome, decision AI, not service date as the phase-3 sketch says), `status` (TEXT, no CHECK, so step 6 can add `waiting`, `searching`, `tracking`, `done`, `not_found`, `cancelled` without a migration), `due_at`, `state` (JSON, NULL for summaries; step 6 stores its tracking state there), `updated_at`. PK `(rule_id, date)`. Step 5 statuses: `running`, `pending`, `sent`, `skipped`, `failed` (`pending` was added by the implementation, see decision 10).
6. **`notifications`** (same migration): `id`, `rule_id` (nullable, `ON DELETE SET NULL`, so the log survives a deleted rule), `kind` (`summary`, `alert`; later `reminder`, `delay`, `cancelled`, `not_found`, `system`), `title`, `body`, `url`, `channel`, `sent`, `removed`, `failed`, `sent_at`. Title and body apart (the sketch had one `text`) because the log shows them as the phone did. The test notification is not recorded (decision AL); `/api/rules/{id}/test` is.
7. **Alerts once**: `notified_alerts (alert_id PK, first_seen_at, notified_at NULL)`. The scheduler upserts the ids of the current alerts when `Alerts::summary().read_at` changes, so "new" is measurable. **Baseline** (AT): at the very first read (empty table) every id is stored with `notified_at = first_seen_at`, meaning handled, never sent. An alert is new for a rule when `notified_at` is NULL and `first_seen_at` is after the rule's `created_at`. **Timing** (AS): only at the rule's occurrence, right after its summary, so never at night and, on paused or holiday days, at the next occurrence; an alert that ended meanwhile is not sent.
8. **Alerts of a rule**: `for_stop(rule.stop_id, stop lines from the index, now)`, then, when the rule has `lines`, only alerts naming one of them or naming no line. Sent as their own notification after the summary: `🚧 <title>`, body the description cut to ~400 chars, `tag: alert-<id>`, url the stop page; more than two at once → one `🚧 N nuovi avvisi a <stop>` listing titles.
9. **Summary message** (decision J, examples in phase-3 section 7): title `🚌 <rule name> — <stop name> (<code>)`; one line per departure `4 → Falchera 07:43 (+2) tra 3'`, no padding (bodies are not monospace); cancelled `… cancellata`; delay shown only when ≥ 1 minute. `tag: rule-<id>` (today's replaces yesterday's), url `/fermata/<code>`. Departures with `source: scheduled` get `(orario)`: `15 → Sassi 07:47 (orario) tra 7'` (AV). No departure in the window: body `Nessuna partenza nei prossimi 30 minuti.` (AU).
10. **Upstream failure**: a stale response (≤ 2 min old) is used as it is; an error is retried at each tick until the grace ends (the run is `pending` meanwhile and becomes `running` when a retry succeeds), then `failed` and one notification `⚠️ <rule name>` with body `Riepilogo non disponibile: dati non raggiungibili.` (AU); its alerts wait for the next occurrence.
11. **`POST /api/rules/{id}/test`**: summary rules only (a tracking rule answers `422` until step 6); ignores days, pause and `enabled`; sends now, records in `notifications` (kind `summary`), writes no `rule_runs` row and marks no alert. Response: `channel`, the `SendReport`, and the `title`/`body` sent.
12. **API**: `GET /api/notifications?limit=` (default 50, max 200, newest first, with the rule's name when it still exists); `GET /api/rules/{id}/runs` (last 30 rows); the rule view's `today` gains `run: { status, updated_at } | null`.
13. **Retention**: rows of `notifications` and `rule_runs` older than 90 days are deleted at the first tick of each day.
14. **Config**: `SCHEDULER_TICK_SECS` (15), `MISSED_RUN_GRACE_MINUTES` (5), in `src/config.rs`, `.env.example`, `docs/user/`.

No new ADR: these are phase decisions, recorded in phase-3.md as AS–AV (AF and AL are fulfilled, not changed).

## Pitfalls

- Never "today + time": `local_time(date, time)` and `Schedule::on` only. Test DST days (29 March, 25 October 2026) and midnight with `FixedClock`.
- `rule_runs.date` is the calendar day (AI); a summary at 00:30 belongs to its own day, not to GTT's service date.
- Do not hold the `Alerts` `RwLock` guard or a DB transaction across `.await` on the notifier.
- `window_minutes` is already capped at `FETCH_WINDOW` (2 h) and `max_departures` at 20 (`src/api/rules.rs:34-37, 371-386`): 20 lines fit in the payload.
- Payload limit: build the body, then cut lines to stay under 3000 bytes (`NotifyError::PayloadTooLarge` must never stop a summary).
- `AppState.alerts == None` (GTT off): summaries still work, no alerts.
- Startup with an empty `notified_alerts`: without the baseline the first summary sends months-old alerts.

## Done criteria

- With `FixedClock`: a due summary is sent once across two ticks and across a restart (new `Scheduler` on the same DB); a summary 6 minutes late is `skipped`; paused, holiday, not-scheduled and disabled days send nothing; 23:58 caught at 00:01; DST days.
- Alerts: a new alert at the rule's stop and line is notified once, never again after a restart; filtered by the rule's lines; old alerts never sent (baseline and `created_at`).
- `notifications` rows for each send, with counts; `GET /api/notifications`, `GET /api/rules/{id}/runs`, `today.run`, `POST /api/rules/{id}/test` covered by API tests.
- The three command groups in CLAUDE.md pass; migration `0006` tried on a copy of `DATA_DIR`.
- By hand: a summary rule two minutes ahead arrives in Chrome on the Mac; `/test` arrives at once.
- Docs: `docs/architecture.md` (Rules, Notifications, new Scheduler section, code map), `docs/user/api.md` and configuration, chapter `docs/development/18-scheduler.md`, phase-3.md (decisions, schema section 9 aligned with the migration).

## Agent tasks (Sonnet, effort medium; sequential, they share `src/main.rs` and `src/api/`)

1. Migration `0006`, `src/scheduler/store.rs` (rule runs, notifications log, notified alerts) with tests.
2. `src/scheduler/mod.rs` (tick, occurrences, summary message, alerts), config, spawn in `main.rs`, tests with `FixedClock` and a recording notifier.
3. API: `/test`, `/runs`, `/api/notifications`, `today.run`, with tests.
4. Docs (docs-sync skill). Then one Opus review of the diff against this plan.

## Commit split (proposed, approved before committing)

| Type(scope) | Subject | Covers |
|---|---|---|
| `docs(plan)` | plan phase 3 step 5, scheduler and summaries | this file, decisions AS–AV in phase-3.md (first commit) |
| `feat(scheduler)` | store rule runs, notifications and notified alerts | migration 0006, store, its docs |
| `feat(scheduler)` | send summaries and new alerts on schedule | tick, messages, config, main, docs |
| `feat(api)` | rule test, rule runs and notifications log | endpoints, `today.run`, API docs, chapter 18 |

Branch `feat/scheduler`, one PR.

## Answered questions (2026-11-12, all as recommended)

1. New alerts are sent with the rule's summary, not as soon as read (AS).
2. Alerts active before the scheduler's first start or before the rule existed are not sent (AT).
3. An empty or failed summary still sends a short message (AU).
4. Scheduled-only departures are marked `(orario)` (AV).

## Report (2026-11-12)

**Built.** Migration `0006` (`rule_runs`, `notifications`, `notified_alerts`), `src/scheduler/` (store, tick, summary and alert messages, spawn in `main.rs`), `SCHEDULER_TICK_SECS` and `MISSED_RUN_GRACE_MINUTES`, `POST /api/rules/{id}/test`, `GET /api/rules/{id}/runs`, `GET /api/notifications`, `today.run`. Docs: architecture, user API and configuration, chapter 18, phase-3.md. Verified: fmt, clippy, cargo tests, web check and tests, docs check, migration on a copy of `data/`. One Opus review, one fix round (on its two findings only).

**Where the plan was wrong.**
- Decisions 3, 4 and 10 conflicted: a row inserted `running` before sending cannot also be retried on upstream errors. Added status `pending` (kept across restarts). The review found a `pending` run could send late; fixed: past the grace it never sends, at most 2×grace late it sends only the failure notice, beyond that it fails silently.
- A rule's occurrence before its `created_at` is ignored (yesterday) or `skipped` (today).
- Alerts are marked per alert right after each send.

**Backlog added:** global `notified_alerts` (two rules on one stop), baseline needs a non-empty first read, only active alerts recorded, `skipped` blocks a later time the same day, `notified_alerts` never purged.

**By hand (2026-11-12, Chrome on the Mac, data copy):** a summary rule two minutes ahead arrived on time (push logged in `chrome://gcm-internals`); `/test` and `/api/notifications/test` arrived at once; `GET /api/notifications`, `/runs` and `today.run` showed the sends. It found one defect: texts kept the upstream capitals; fixed by porting the web app's `displayName` to the server. One of three automatic sends was accepted by the push service (`sent: 1`) but not seen by the user; it happened before the recording started, so it is not reproducible. `/test` and the scheduled summary share the tag `rule-N`, so the later replaces the earlier (by design).

**Not done:** no test for per-alert marking; decision 10 text above is not updated for the 2×grace rule.

**For the next work (step 6):** tracking reuses `rule_runs.state` and the statuses; the scheduler ignores tracking rules for now.
