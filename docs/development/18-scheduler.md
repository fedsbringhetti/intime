# 18. Scheduler and summaries

> Phase 3, step 5 · PR (to be opened) · plan [phase-3-step-5.md](../plan/steps/phase-3-step-5.md)

## Goal

Rules now act. A background task looks at the summary rules every 15 seconds and, at each rule's time, sends the next departures of its stop as a notification, then the new GTT alerts of that stop. Every send is recorded in a log, `GET /api/notifications` shows it, `GET /api/rules/{id}/runs` shows how each day went, and `POST /api/rules/{id}/test` sends a rule's summary at once. Tracking rules are stored but ignored until step 6.

## Decisions

| Decision | Why |
|----------|-----|
| **One task, `tick(now)` called by an interval** | The time is a parameter, so tests call `tick` with chosen instants (daylight saving, midnight) and never wait. The interval uses `MissedTickBehavior::Skip`: after a long pause there is one tick, not a burst. |
| **Occurrences of today and yesterday** | A summary at 23:58 seen at 00:01 is still within the grace. Older days are never looked at. |
| **The row is inserted before sending** | `INSERT OR IGNORE` on `(rule_id, date)` claims the occurrence: a second tick or a second process cannot send it again. A crash leaves `running`, which startup turns into `failed`: a missing summary is better than a duplicate or a late one. |
| **`date` is the calendar day in Rome** | Decision AI: a 00:30 summary belongs to its own day, not to GTT's service date. |
| **`status` has no `CHECK`** | Step 6 adds `waiting`, `searching`, `tracking`... without a migration. `state` (JSON) is for it too and stays `NULL` for summaries. |
| **A `pending` status, not in the plan** | When the departures cannot be read, the occurrence must be retried at each tick until the grace ends, and `running` would be turned into `failed` by a restart. `pending` means "waiting for the upstream": a restart leaves it alone and the retry goes on. Once the grace is over a `pending` run never sends the summary, even if the upstream answers: it becomes `failed`, with the notice only if it is at most twice the grace late. |
| **Missed runs are skipped** | Decision M: more than `MISSED_RUN_GRACE_MINUTES` late is `skipped`, as is a rule created or moved to a time already past. An occurrence earlier than the rule's `created_at` is not even recorded for yesterday. |
| **New alerts only after the summary** | Decision AS: alerts never wake the phone at night, and on paused or holiday days they wait. The scheduler stores the ids of the active alerts each time they are read, so "new" is measurable; the first read is a baseline of handled alerts (AT). |
| **`notifications` keeps title and body apart** | The log shows what the phone showed. `rule_id` is `ON DELETE SET NULL`, so deleting a rule keeps its history. |
| **Test sends are logged, write no run** | The real summary of the day must still go out; the log shows the test. `POST /api/notifications/test` stays unrecorded (decision AL). |
| **Message builders are shared** | `summary_notification` is used by the scheduler and by the test endpoint, so what the test shows is what the scheduler sends. |

## Files

| File | Role |
|------|------|
| `migrations/0006_scheduler.sql` | `rule_runs`, `notifications`, `notified_alerts` |
| `src/scheduler/store.rs` | `SchedulerStore`: runs, log, alert ids; 90-day retention |
| `src/scheduler/mod.rs` | `Scheduler::tick`, occurrences, messages, alerts, `spawn` |
| `src/scheduler/tests.rs` | tests with `FixedClock`, a recording notifier and fake sources |
| `src/api/rules.rs`, `src/api/notifications.rs`, `src/api/error.rs`, `src/api/mod.rs` | `test`, `runs`, `list`, `today.run`, `not_a_summary`, routes, `AppState.scheduler` |
| `src/config.rs`, `.env.example` | `SCHEDULER_TICK_SECS`, `MISSED_RUN_GRACE_MINUTES` |
| `src/lib.rs`, `src/main.rs` | the module and the wiring: the scheduler is created and spawned before the server |
| `tests/rules.rs`, `tests/common/mod.rs` | API tests |

## How the code works

### The tick

`tick` purges old rows once a day, records the alert ids, then walks the rules. For each summary rule it asks the calendar of step 3 what the rule does on yesterday and today:

```rust illustrative
for date in [yesterday, today] {
    if let Day::Due { at } = schedule.on(date) {
        self.occurrence(rule, config, date, date == today, at.to_utc(), now).await;
    }
}
```

`Schedule::on` and `local_time` already handle paused, holiday and disabled days and the hours that do not exist or repeat, so the scheduler never computes "today + time" itself.

### One occurrence

`occurrence` returns at once when `now < at` or when a run exists that is not `pending`. A run is absent: if it is more than the grace late, or before the rule existed, it is recorded as `skipped`. Otherwise the summary is built first (the departures come from the cache, stale up to 2 minutes). On an upstream error the run becomes `pending` and the next tick tries again; a `pending` run found past the grace (checked before any fetch) becomes `failed` without sending the summary, with a short notice only if it is at most twice the grace late. When the summary is ready the occurrence is claimed:

```rust
        // Claim the occurrence before sending: a crash leaves `running`, never a duplicate.
        let claimed = if pending {
            self.store.set_run_status(rule.id, date, RUNNING, now).await
        } else {
            self.store
                .insert_run_if_absent(rule.id, date, at, now)
                .await
        };
```

The claim is the idempotency: `INSERT OR IGNORE` returns whether a row was really inserted, so only one caller goes on to send. After sending, the run becomes `sent` (or `failed`) and `notify_alerts` runs. At startup `recover` marks leftover `running` rows as `failed`.

### The message

`summary_notification` filters with `StopDepartures::select` (the rule's lines, window and limit), writes one line per departure (`4 → Falchera 07:43 (+2) tra 3'`; `(orario)` when only the timetable is known; `cancellata`) and `fit_payload` cuts lines from the end so the 3000-byte limit never stops a summary. An empty window gives "Nessuna partenza nei prossimi N minuti." The `tag` is `rule-<id>`: today's replaces yesterday's on the phone.

### New alerts

`record_alerts` runs when the alerts' `read_at` changed. With an empty `notified_alerts` it stores every id as handled (the baseline); afterwards it only inserts unknown ids with `notified_at` NULL. `notify_alerts` takes the unhandled ids first seen after the rule was created, keeps those that `for_stop` returns for the rule's stop and that name one of the rule's lines or none, and sends them (one notification each up to two, one list above that) before marking them handled. If a send fails, none is marked and the next occurrence tries again.

### The API

`POST /api/rules/{id}/test` builds the same notification at the clock's time, sends it with `send_and_log` and returns `channel`, the counts, `title` and `body`; a tracking rule answers `422 not_a_summary`. `GET /api/rules/{id}/runs` returns `recent_runs` (30), `GET /api/notifications` the log (`limit` 1 to 200). The rule view's `today` gains `run`, read with `get_run` for today's calendar day.

**Rust concept**: the scheduler takes its sources as `Arc<dyn DepartureSource>` and `Arc<dyn AlertSource>`, small traits implemented by the real services and by fakes in tests, like `Notifier` in chapter 17.

## Tests

`src/scheduler/tests.rs` drives `Scheduler::tick` with `FixedClock` on an in-memory database: a summary sent once across ticks and across a new `Scheduler` on the same database; 5 minutes late sent, 6 skipped; a rule created after its time skipped; paused, unscheduled, disabled and holiday days sending nothing; 23:58 caught at 00:01; the DST gap (29 March 2026) and repeat (25 October 2026); empty window; upstream failure retried, then reported once, and a retry that succeeds; a run left `running` failing at recovery; the payload cut; the test send logged without a run; new alerts sent once and never the baseline, filtered by the rule's lines and by `created_at`, grouped above two. `src/scheduler/store.rs` tests the queries and retention. `tests/rules.rs` covers `/test`, `/runs`, `today.run` and `/api/notifications` through HTTP.

## Check it by hand

```bash
cargo run
curl -s -H "Content-Type: application/json" -X POST localhost:8080/api/rules -d '{"name": "Prova", "kind": "summary", "stop": "39", "days": ["mon","tue","wed","thu","fri","sat","sun"], "config": {"time": "HH:MM"}}'
curl -s -X POST localhost:8080/api/rules/1/test
curl -s localhost:8080/api/notifications
```

Replace `HH:MM` with a time two minutes ahead in Rome: within a minute of it the notification arrives (in the log without a VAPID key, in Chrome with one) and `GET /api/rules/1/runs` shows today as `sent`. Use `SCHEDULER_TICK_SECS=2` to see it sooner.

Checked by hand on 2026-11-12 in Chrome on the Mac: a summary two minutes ahead and `/test` both arrived. They share the tag `rule-<id>`, so the later one replaces the earlier in the notification centre; this is by design.


## Pitfalls

- **Never "today + time"**: DST days are tested because `local_time` does the conversion; the scheduler only compares instants.
- **Do not hold a guard across the notifier**: neither the `Alerts` lock nor a transaction lives over an `.await` on a send.
- **Known limitations**:
  - `notified_alerts` is global, not per rule: two rules on one stop share an alert, and the first to run gets it; the other never does.
  - The baseline needs a non-empty first read. While the table is empty each read is stored as the baseline, so if the first read has no active alerts, the first ones that appear are swallowed as baseline instead of being sent.
  - Only alerts active at the time of a read are recorded. An alert whose period starts later is "first seen" then, which is right, but one that was active and ended between two reads is never seen at all.
  - These three are in [../backlog.md](../backlog.md).

## Later changes

None yet.
