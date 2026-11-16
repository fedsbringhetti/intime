# 19. Trip tracking

> Phase 3, step 6 · PR (this PR) · plan [phase-3-step-6.md](../plan/steps/phase-3-step-6.md)

## Goal

Follow one run. From a departure on the stop screen ("Avvisami") or from a recurring `track` rule, the scheduler sends reminders before the run passes, an alert when it is late (and again when it gets later), one when it is cancelled and one when a recurring run cannot be found. A live page, `/corsa/{id}`, shows the expected time, the delay, the assigned vehicle and how many stops away it is, and the notifications sent; Notifiche lists the trackings in progress. Rule editing for trackings (step 7), health alerts (step 8) and the delay history screens (step 9) are not part of this step.

## Decisions

| Decision | Why |
|----------|-----|
| **Vehicle from trip updates** (AW, changes P) | Measured on the live feeds on 2026-11-16: `vehicle_position.aspx` vehicles carry only the route, with no trip id, start date or stop sequence, so none can be linked to a run; every run in `trip_update.aspx` has a vehicle descriptor, and its stop updates start at the vehicle's next stop. Stops away is the tracked sequence minus that first sequence. `GTT_VEHICLES_POLL_SECS` is dropped. |
| **No `TRACK_POLL_SECS`** | Trackings are evaluated at every scheduler tick (15 s) on the departures cache (20 s) shared with summaries and the web app: no extra upstream calls, and the feeds change every 14–38 s anyway. |
| **A one-off is a `track` rule with a `date`** | The calendar (`Schedule`), the store and `rule_runs` already handle it. `TrackConfig.trip_id` is set only for one-offs; `/api/rules` neither accepts it nor shows these rules. |
| **`step` is a pure function** | State, config, departures and time in; new state, status and notices out. All the cases (late start, restart, delay, cancellation) are tested without a database or a clock. |
| **A late reminder is skipped, never sent** (AX) | The smallest threshold reached is marked done with every larger one, and sent only within 2 minutes of it: after a restart or a late start "passa tra 10 minuti" would be wrong. |
| **State saved before sending** | A crash between the two loses a notice instead of sending it twice (the choice of step 5). The update applies only to `searching`/`tracking` rows, so a "Smetti di seguire" or a deleted rule during the evaluation is not overwritten. |
| **Alerts per rule** (AZ) | `rule_notified_alerts (rule_id, alert_id)`: a summary and a tracking on the same stop both get a new alert. It resolves the backlog item found in step 5; no data migration, since alerts already handled stay handled for every rule. |
| **Delay history written now** (AY) | `observed_delays` is filled at the end of every `done` run, so the history exists when step 9 shows it. |
| **"Smetti di seguire" stops today only** (BA) | A recurring rule is not turned off by one tap: today's run becomes `stopped`, tomorrow it runs again. A one-off is deleted. |

## Files

| File | Role |
|------|------|
| `migrations/0007_tracking.sql` | `observed_delays`, `rule_notified_alerts` |
| `src/rules/mod.rs` | `TrackConfig.trip_id` |
| `src/gtt/trip_updates.rs` | `Run.vehicle`, `TripUpdates::vehicle`, `RunVehicle::stops_away` |
| `src/scheduler/tracking.rs` (+ `tracking/tests.rs`) | `TrackState`, `step`, notices, `create_one_off`, the live `view` |
| `src/scheduler/mod.rs` | tick over track rules, `track_occurrence`, `start_run`, observed delays, alerts per rule, purge of old one-offs |
| `src/scheduler/store.rs` | track run helpers (`insert`, `update`, `reset`, `get`, `active`, `stop`), `observed_delays`, per-rule alerts |
| `src/api/trackings.rs` | the four `/api/trackings` handlers |
| `src/api/rules.rs`, `error.rs`, `mod.rs` | one-offs hidden, shared checks, `trip_not_found`, `trip_not_trackable`, `tracking_not_found`, routes |
| `web/src/lib/tracking.ts`, `components/TrackSheet.svelte` | sheet choices, request body, countdown |
| `web/src/screens/Tracking.svelte`, `router.ts`, `App.svelte` | the live page and its route `/corsa/{id}` |
| `web/src/lib/api.ts`, `screens/Notifications.svelte`, `Stop.svelte`, `components/DepartureRow.svelte` | API calls, "Corse seguite", tappable rows |
| `tests/trackings.rs` | API tests |

## How the code works

### Starting

`POST /api/trackings` checks the body like a rule (same reminder and delay limits), finds the departure in `DeparturesService::get(stop)` by `trip_id` (searching the whole list: `select` drops departures already past), and refuses a cancelled or departed one. `create_one_off` then builds the rule and inserts today's run as `tracking`, already seeded with what was seen:

```rust illustrative
let config = TrackConfig {
    line: departure.line.clone(),
    scheduled_time: scheduled.time(),
    tolerance_minutes: 0,
    watch_from_minutes: watch_from,   // at least 10, the longest reminder
    trip_id: Some(departure.trip_id.clone()),
    ..
};
```

The rule's `date` is the local date of the departure's `scheduled`, so a run at 00:30 tapped at 23:50 belongs to tomorrow, and the tick looks one day ahead for every track rule (so a rule at 00:10 starts the evening before). The same stop, trip and date again replaces the rule's settings and returns `200`.

A recurring rule has no run until `now ≥ at − watch_from`; `start_run` then inserts `searching` and, for the first time that day, sends the stop's new alerts for the line. An occurrence more than 10 minutes late, or from before the rule existed, is recorded as `skipped`.

### Each tick

`track_occurrence` reads the run, takes the departures of the stop and calls `step`. The order inside `step`: search the run if not yet found (or give up: `not_found`); if it is missing from the list, keep the state until its time; cancelled → one alert, `cancelled`; `now ≥ expected` → `done`; otherwise reminders, then the delay alert.

```rust
    state.reminders_done.extend(reached);
    if remaining <= TimeDelta::minutes(i64::from(smallest)) - REMINDER_SLACK {
        return;
    }
```

The reached thresholds are marked done before the test of the slack, which is how a stale one is skipped. The delay alert needs a real-time `source` (a timetable-only run is never "late") and then repeats only at 3 more minutes than the last one.

The scheduler saves the new state and status, and only if the row was still `searching` or `tracking` sends the notices through `send_and_log` (kinds `reminder`, `delay`, `cancelled`, `not_found`; `tag: track-<id>`; `url: /corsa/<id>`). At `done` the final delay goes to `observed_delays`.

### The vehicle

```rust
    pub fn stops_away(&self, sequence: u32, now: DateTime<Utc>) -> Option<u32> {
        if self.updated_at.is_some_and(|at| now - at > MAX_AGE) {
            return None;
        }
        sequence.checked_sub(self.next_sequence?)
    }
```

`checked_sub` is the **Rust concept** here: on unsigned numbers `a - b` panics (debug) or wraps (release) when `b > a`; `checked_sub` returns `None` for a stop already behind the vehicle. The live view calls it only while `tracking` and only for GTT runs (`trip_id` starts with `gtt:`); without GTT data `vehicle` is `null`.

### The live view and the web app

`view(rule, run, updates, now)` builds the detail JSON, including `phase` (`waiting` is the absence of a row) and `planned_reminders`. `GET /api/trackings/{id}` takes the active run (yesterday's, today's or tomorrow's) for a recurring rule, else today's, and the rule's own date for a one-off.

The web app: `DepartureRow` becomes a button when given `onselect` (never on a cancelled row, nor with old data); `TrackSheet.svelte` shows the run, the reminders 15/10/5/2 and the delay stepper and posts `buildTrackRequest(...)`; the answer's `id` is the route `/corsa/{id}`. `Tracking.svelte` polls every 15 seconds with `startPolling`, derives the countdown from `expected` (`formatCountdown`) and answers a `404` with "Corsa non più seguita". Notifiche reads `GET /api/trackings` for "Corse seguite".

## Tests

- `src/scheduler/tracking/tests.rs`: `step` from fixtures: closest run and ties, on time (two reminders once), getting later (alerts at 3 and 6), timetable-only never late, cancelled with the next departure, never found, not found after the time (silent), found then vanished, late start, restart mid-run, a reminder passed while delayed, a one-off without search; the live view.
- `src/scheduler/tests.rs`: with `FixedClock` a recurring rule starts at `at − watch_from`, not on holidays or paused days, the watch time across the clock change, a one-off at creation and for just after midnight, yesterday's tracking after midnight, reminders once across restarts, a stopped run not overwritten, done and cancelled runs and `observed_delays`, a one-off failing silently after an hour, alerts per rule, purge of old one-offs; `src/gtt/trip_updates.rs`: vehicle and stops away.
- `tests/trackings.rs`: the four routes, errors, replacing settings, one-offs hidden from `/api/rules`.
- `web/src/lib/tracking.test.ts`, `api.test.ts`, `router.test.ts`: reminders, request body, countdown, calls, the route.

## Check it by hand

```bash
cargo run
curl -s localhost:8080/api/stops/39/departures
curl -s -H "Content-Type: application/json" -X POST localhost:8080/api/trackings -d '{"stop": "39", "trip_id": "gtt:..."}'
curl -s localhost:8080/api/trackings
curl -s -X DELETE localhost:8080/api/trackings/1
```

Use the `trip_id` of an urban departure 12 minutes away. In the web app (`npm run build` first, or `npm run dev`) tap that row, then "Avvisami per questa corsa": the 10 and 5 minute reminders arrive in the log (or in Chrome with a VAPID key) and the live page shows the vehicle and its stops away.

## Pitfalls

- **Never "today + seconds"**: `at` comes from `Schedule::on`, and the departures' times are already absolute; tests cover DST days, midnight and yesterday's tracking still running at 00:05.
- **Stops away before the start**: a run not yet started has a vehicle whose next stop is its first one, so the number means "not left yet", not "near".
- **No lock across the notifier**: the state is saved, then the notices are sent, with no transaction or guard held over the `.await`.
- **Known limits**: a run is seen only while it is in the stop's 2-hour list; `vehicle_position.aspx` is not read, so there are no coordinates.

## Later changes

None yet.
