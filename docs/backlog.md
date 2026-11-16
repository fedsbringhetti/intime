# Backlog

Work not done yet, one item per section, newest last. Defects found while working on something else land here with their evidence instead of being fixed in passing (see CLAUDE.md, How we work). Planned phases and steps live in [plan/](plan/phase-3.md), not here.

Each item: what is wrong or wanted, the evidence (file and line, command and output, date), and why it matters. Remove the item in the PR that resolves it.

## The alert baseline needs a non-empty first read

`Scheduler::record_alerts` decides "first read" with `alerts_is_empty()`, a count of `notified_alerts` (`src/scheduler/mod.rs`, `src/scheduler/store.rs`). If the first read of GTT's alerts has no active alert, the table stays empty and the next read is stored as the baseline too, so the alerts that appear then are marked handled and never sent. Found while writing chapter 18 (2026-11-12). Real feeds have over 100 alerts (133 on 2026-11-12), so it is unlikely; a flag in `sync_state` would make the baseline explicit.

## Only active alerts are recorded

`record_alerts` stores `alerts.all(now)`, the alerts active at the time of the read (`src/scheduler/mod.rs`). An alert that starts and ends between two reads (5 minutes apart by default, `GTT_ALERTS_POLL_SECS`) is never recorded or sent, and one whose period starts later is "first seen" at that read. Found while writing chapter 18 (2026-11-12). Acceptable for a notification of what matters now; worth noting if alerts are ever used as a history.

## A skipped run blocks a later time the same day

`insert_run_if_absent` keys runs by `(rule_id, date)` (`src/scheduler/store.rs`). A rule created at 08:00 with time 07:00 gets a `skipped` row for today; moving it to 09:00 afterwards sends nothing that day, because a row exists (`src/scheduler/mod.rs`, occurrence handling). Consistent with "one occurrence per rule and day" but surprising. Found in the step 5 review (2026-11-12). A fix would drop a `skipped` row when the rule's time changes.

## `notified_alerts` is never purged

`purge_old` deletes only `notifications` and `rule_runs` (`src/scheduler/store.rs`). Alerts at stops no rule watches keep `notified_at IS NULL` forever and `unnotified_alert_ids` returns them at every occurrence. About 4 new alerts a day network-wide, so harmless for years. Found in the step 5 review (2026-11-12).

## One-off trackings follow the first passage on loop lines

`create_one_off` and `find_departure` match the departure by `trip_id` only (`src/scheduler/tracking.rs`). A run that passes the same stop twice within the 2 h window (loop line) is followed at its first passage, because `stop_sequence` is `serde(skip)` and the sheet does not send it. Found in the step 6 review (2026-11-16). Fix: send the departure's `stop_sequence` (or scheduled time) with `POST /api/trackings`.

## A paused or disabled rule leaves its active run `tracking`

`tick` stops evaluating a recurring track rule when `Schedule::on` is no longer `Due` (pause, "Salta oggi", disabled) while its run is `searching` or `tracking`, so the row keeps that status and `GET /api/trackings` lists it until the date window passes (`src/scheduler/mod.rs`). No notification is sent: only the list is stale. Found in the step 6 review (2026-11-16). Fix: mark such runs `stopped` when the rule is paused or disabled.

## Narrow races in one-off creation

(1) `update_track_run` is conditional on status only, so a tick that read the old state can overwrite the state `create_one_off` has just reset (`src/scheduler/store.rs`); no duplicate is sent. (2) Two concurrent `POST /api/trackings` for the same run can create two one-off rules, because the lookup uses `rules.list()` and nothing is unique (`src/scheduler/tracking.rs`); the sheet blocks double taps. Found in the step 6 review (2026-11-16); single user, unlikely.

## `rule_notified_alerts` grows with long-lived summary rules

Rows are removed only by the rule's cascade (`migrations/0007_tracking.sql`); a summary rule that lives for years keeps one row per alert it was sent, about 4 a day network-wide at most. Same family as `notified_alerts`. Found in the step 6 review (2026-11-16).

## An alert when the run is early

Tracking alerts only on delay (`delay_alert_minutes`). A run that comes much earlier than scheduled (the user could miss it) sends nothing. Asked by the user during the step 6 manual check (2026-11-16). Idea: an `early_alert_minutes` threshold with the same once-only rule as delay, and a `kind: early` event.
