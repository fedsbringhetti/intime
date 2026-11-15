# Backlog

Work not done yet, one item per section, newest last. Defects found while working on something else land here with their evidence instead of being fixed in passing (see CLAUDE.md, How we work). Planned phases and steps live in [plan/](plan/phase-3.md), not here.

Each item: what is wrong or wanted, the evidence (file and line, command and output, date), and why it matters. Remove the item in the PR that resolves it.

## Alerts already notified are global, not per rule

`notified_alerts` has one row per alert id (`migrations/0006_scheduler.sql`), and `Scheduler::notify_alerts` marks an alert handled after sending it for one rule (`src/scheduler/mod.rs`, `mark_alerts_notified`). Two rules on the same stop (or on stops that an alert names together) therefore share it: the rule that runs first gets the notification and the other never does, even when its lines match. Found while writing chapter 18 (2026-11-12). It matters once more than one rule watches the same place; the fix is a `(rule_id, alert_id)` key.

## The alert baseline needs a non-empty first read

`Scheduler::record_alerts` decides "first read" with `alerts_is_empty()`, a count of `notified_alerts` (`src/scheduler/mod.rs`, `src/scheduler/store.rs`). If the first read of GTT's alerts has no active alert, the table stays empty and the next read is stored as the baseline too, so the alerts that appear then are marked handled and never sent. Found while writing chapter 18 (2026-11-12). Real feeds have over 100 alerts (133 on 2026-11-12), so it is unlikely; a flag in `sync_state` would make the baseline explicit.

## Only active alerts are recorded

`record_alerts` stores `alerts.all(now)`, the alerts active at the time of the read (`src/scheduler/mod.rs`). An alert that starts and ends between two reads (5 minutes apart by default, `GTT_ALERTS_POLL_SECS`) is never recorded or sent, and one whose period starts later is "first seen" at that read. Found while writing chapter 18 (2026-11-12). Acceptable for a notification of what matters now; worth noting if alerts are ever used as a history.

## A skipped run blocks a later time the same day

`insert_run_if_absent` keys runs by `(rule_id, date)` (`src/scheduler/store.rs`). A rule created at 08:00 with time 07:00 gets a `skipped` row for today; moving it to 09:00 afterwards sends nothing that day, because a row exists (`src/scheduler/mod.rs`, occurrence handling). Consistent with "one occurrence per rule and day" but surprising. Found in the step 5 review (2026-11-12). A fix would drop a `skipped` row when the rule's time changes.

## `notified_alerts` is never purged

`purge_old` deletes only `notifications` and `rule_runs` (`src/scheduler/store.rs`). Alerts at stops no rule watches keep `notified_at IS NULL` forever and `unnotified_alert_ids` returns them at every occurrence. About 4 new alerts a day network-wide, so harmless for years. Found in the step 5 review (2026-11-12).
