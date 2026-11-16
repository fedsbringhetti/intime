# Phase 3, step 6: trip tracking

Size: large (state machine on persistent state, new API, two new screens). Plan for the implementation session; open questions answered by the user on 2026-11-16, all as recommended (decisions AW–BA in phase-3.md).

Scope (phase-3.md sections 4–6, 10, 11, 14): tracking one run, on the fly from a departure ("Avvisami") or by a recurring `track` rule; reminders, delay, cancellation and not-found alerts; vehicle and stops away; the live page `/corsa/{id}`; trackings in progress on the Notifiche screen. Not in scope: rule editor and `/test` for tracking rules (step 7, the `422` stays), health alerts (step 8), `/api/delays` and the history on the live page (step 9).

## Verified facts

- `TrackConfig { line, scheduled_time, tolerance_minutes, watch_from_minutes, alert_before_minutes, delay_alert_minutes }`, `deny_unknown_fields`, **no `trip_id`** (`src/rules/mod.rs:54-68`). Defaults and limits in `src/api/rules.rs:44-55` (reminders ≤ 4, each ≤ 60; delay 1–30; watch 10–120, ≥ longest reminder, `:501`).
- `Schedule` handles one-off `date` rules (`src/rules/schedule.rs:50`, test `one_off_dates` `:231`); `Rule::schedule()` uses `scheduled_time` for tracks (`src/rules/mod.rs:96-130`). `/api/rules` refuses `date` (AJ).
- Scheduler: `tick` loops today and yesterday, **skips non-summary rules** (`src/scheduler/mod.rs:354-379`); `recover` fails only `running` rows (`src/scheduler/store.rs:163`), so tracking statuses survive a restart. `rule_runs.status` has no CHECK and `state` is free JSON (`migrations/0006_scheduler.sql`). `send_and_log`, `fit_payload`, `stop_label` (title-cased names) are reusable (`src/scheduler/mod.rs:90-300`). `notify_alerts` takes a `SummaryConfig` (`:568`).
- Departures: `DeparturesService::get(stop_id)`, cache 20 s (`DEPARTURES_CACHE_SECS`), fetches 2 h from now (`src/departures.rs:20, 280-330`); OTP query has `omitCanceled: false` (`src/upstream/queries.rs:7`), so cancelled runs stay in the list. `Departure` has `trip_id` (serialized; the web app has it, `web/src/lib/api.ts:45`), `line`, `headsign`, `scheduled`, `expected`, `delay_seconds`, `canceled`, `source`; `stop_sequence` and `service_date` are `serde(skip)` (`src/departures.rs:33-53`). `StopDepartures::select` drops departures expected before now (`:106`): tracking must search `data.departures` directly.
- GTT trip updates: `TripUpdates.runs: HashMap<(trip_id, service_date), Run { canceled, updated_at, stops }>` (`src/gtt/trip_updates.rs:17-35`); the vehicle descriptor is **not kept**.
- **Measured on the live feeds, 2026-11-16 11:30** (raw protobuf read, scratch script):
  - `vehicle_position.aspx` (368 vehicles): `trip` has **only `route_id` and `schedule_relationship`**: no trip id, no start date, no `current_stop_sequence`, no `stop_id`. Each has position, timestamp and vehicle `id`/`label` (`9654U`/`9654`). A vehicle cannot be linked to a run from this feed.
  - `trip_update.aspx` (389 runs): **every** run has a vehicle descriptor (`id` `9648U`, `label` `9648`); 377 of 389 ids are in the positions feed. Stop updates start at the vehicle's next stop (e.g. 38–44 of a run started an hour before); 71 runs not started yet (start time ahead) already have a vehicle and start at sequence 1.
  - So "vehicle assigned" and "stops away" (tracked `stop_sequence` − first update's sequence) come from the trip update alone; the positions feed adds only coordinates (no use without a map).
- Mockups (design canvas, `project/Corsa.dc.html`, `FermataAvvisami.dc.html`; dark variant `CorsaScuro.dc.html`): sheet with line, headsign, expected/scheduled/delay, source, reminder chips 15/10/5/2 (10 and 5 on), delay stepper 1–15 (3), "Avvisami per questa corsa", link "Tutti i giorni? Crea una regola". Live page: back to Notifiche; "Parte tra N min"; expected, scheduled, delay; "Previsione GTT · aggiornata alle HH:MM"; "Mezzo assegnato", "a N fermate", dots, "Mezzo 2817 · posizione delle HH:MM"; "Avvisi" (sent with time, planned "in programma", delay "solo se succede"); history (step 9); "Smetti di seguire".
- Web: `DepartureRow` is a plain `<li>`, no interaction (`web/src/lib/components/DepartureRow.svelte:52`); stop list `web/src/screens/Stop.svelte:267`; routes in `web/src/lib/router.ts` (`/notifiche` `:39`); `lib/poll.ts` exists. Notifiche has no trackings section yet.
- Backlog: "alerts already notified are global" becomes likely once tracking rules send alerts (a summary and a tracking on the same stop). The other four items do not touch this step.

## Decisions

1. **Vehicle from trip updates** (AW): `Run` keeps `vehicle: Option<String>` (label); `TripUpdates::vehicle(trip_id, date) -> Option<RunVehicle { label, next_sequence, updated_at }>`. Stops away = tracked sequence − `next_sequence`, `None` when negative or the update is older than 3 minutes. `vehicle_position.aspx` is not read; `GTT_VEHICLES_POLL_SECS` is dropped. Changes decision P and section 4.
2. **No `TRACK_POLL_SECS`**: trackings are evaluated at every scheduler tick (15 s), departures from the 20 s cache, shared with summaries and the web app. Dropped from section 12.
3. **`TrackConfig.trip_id: Option<String>`** (`serde(default, skip_serializing_if)`), one-offs only; `TrackInput` of `/api/rules` does not accept it.
4. **One-off tracking** (`POST /api/trackings`): body `{stop, trip_id, alert_before_minutes?, delay_alert_minutes?}` (same checks as rules). The departure is looked up in `DeparturesService::get`; not found → `404 trip_not_found`, cancelled or already departed → `422`. Creates a `track` rule: name `<line> delle HH:MM`, `date` = local date of `scheduled`, `days []`, `scheduled_time` local, `tolerance 0`, `watch_from` = max(10, longest reminder), and inserts today's run as `tracking` with its state. Same stop and trip already tracked → its config is replaced and it is returned (`200`, else `201`).
5. **Statuses** of track runs: `searching`, `tracking`, `done`, `cancelled`, `not_found`, `stopped` (new: "Smetti di seguire"), `skipped`, `failed`. `waiting` is never stored: it is the API's phase before `at − watch_from` with no row.
6. **Occurrence** (recurring): with `Day::Due { at }` and no row, at `now ≥ at − watch_from` insert `searching`. Search: same line (case-insensitive), `|scheduled − at| ≤ tolerance`, closest, earlier on ties; store `trip_id`, `service_date`, `stop_sequence` → `tracking`.
7. **State JSON** (typed `TrackState` in `src/scheduler/tracking.rs`): `trip_id`, `service_date`, `stop_sequence`, `headsign`, `scheduled`, `expected`, `delay_seconds`, `source`, `checked_at`, `reminders_done: [u32]`, `delay_alerted: Option<u32>` (minutes), `events: [{kind, minutes?, text, sent_at}]`. The evaluation is a **pure function** `step(state, config, departures, now) -> (state, Vec<Event>)`, tested without DB.
8. **Reminders** (AX): at each evaluation take the smallest threshold T not done with `remaining ≤ T`; mark it and every larger one done; send only if `remaining > T − 2 min`, so a threshold already well past (late start, restart, delay shrinking) is never sent late (decision M).
9. **Delay**: `source != scheduled` and delay ≥ threshold → alert once; again only at +3 more minutes (section 6).
10. **Cancelled** (GTT or OTP, merge rule 4): one alert with the next departure of the same line after `scheduled`, then `cancelled`.
11. **Not found**: recurring still `searching` at `at − 10 min` → `not_found`, alert with the two closest departures of the line, **only if `now ≤ at`**; silent otherwise. One-off: trip missing from the list while `expected − now > 2 min` → keep the last state; at `expected + 60 min` with no data → `failed`, silent.
12. **Done**: `now ≥ expected`, or the run vanished from the list within 2 minutes of its expected time → `done`; final delay kept in `state` and upserted into `observed_delays` (AY).
13. **At most once**: the event is written into `state` **before** sending (a missing reminder beats a duplicate). Updates use `WHERE status IN ('searching','tracking')` so a concurrent `stopped` or a deleted rule is never overwritten.
14. **Messages** (J, section 7), `tag: track-<rule_id>`, `url: /corsa/<rule_id>`, kinds `reminder`, `delay`, `cancelled`, `not_found`: `⏰ Il 67 delle 08:05 passa tra 5 minuti` / `08:07, +2 · Porta Nuova (39)` (`(orario)` when scheduled); `🐢 Il 67 delle 08:05 è in ritardo di 4 minuti` / `Previsto alle 08:09 · Porta Nuova (39)`; `⚠️ Il 67 delle 08:05 è cancellato` / `Il prossimo: 08:17` (or `Nessun altro 67 nelle prossime 2 ore`); `❓ Il 67 delle 08:05 non si trova` / `Più vicine: 07:58, 08:12`.
15. **Alerts** (AS): a recurring tracking rule sends its stop's new alerts for its line when its run starts (`searching`); one-offs do not (the stop page shows the banner). `notify_alerts` takes the lines instead of `SummaryConfig`.
16. **Alerts per rule** (AZ, backlog item, removed from `docs/backlog.md` in this PR): new table `rule_notified_alerts (rule_id FK cascade, alert_id, notified_at, PK (rule_id, alert_id))`. An alert is new for a rule when its `notified_alerts.notified_at` is NULL (not baseline), `first_seen_at` is after the rule's `created_at` and the rule has no row for it; sending writes the rule's row only. Existing non-null `notified_at` stay handled for every rule: no data migration.
17. **Migration `0007`**: `observed_delays` as in phase-3 section 9 (PK `service_date, trip_id, stop_id`; upsert at `done`, `source` of the final time; `cancelled` runs not written) and `rule_notified_alerts`.
18. **API**: `GET /api/trackings` (runs `searching`/`tracking` of today and yesterday, soonest first); `GET /api/trackings/{id}` (rule id: `phase`, line, headsign, stop `{id, code, name}`, scheduled, expected, delay, source, `checked_at`, `vehicle {label, stops_away, updated_at} | null`, `events`, reminders and delay threshold for the planned rows); `DELETE /api/trackings/{id}`: one-off → rule deleted; recurring → today's run `stopped` (inserted if absent), the rule goes on tomorrow (BA). `GET /api/rules` hides one-offs; `/api/rules/{id}` routes answer `404` for them.
19. **Cleanup**: at the daily purge, one-off rules whose `date` is before yesterday are deleted (runs cascade, the log keeps them with `rule_id` null).
20. **Web**: a departure row opens the sheet (`TrackSheet.svelte`; row becomes a button, cancelled rows not tappable); POST then navigate to `/corsa/{id}`. "Crea una regola" hidden until step 7. Live page `screens/Tracking.svelte` at `/corsa/{id}`, polling every 15 s with `lib/poll.ts`, countdown from `expected`; `404` → "Corsa non più seguita". Notifiche gets "Corse seguite" from `GET /api/trackings` (as in `Notifiche.dc.html`). History section left out (step 9).

## Pitfalls

- Never "today + seconds": `at` from `Schedule::on`, departures' `scheduled`/`expected` are already absolute. A one-off at 00:30 tapped at 23:50 has tomorrow's `date`. Tests: DST days, midnight, yesterday's tracking still running at 00:05.
- Do not use `select` (drops passed departures); do not match recurring runs by a stored `trip_id` across days.
- Trip update vehicles exist before the run starts: stops away from sequence 1 means "not left the terminus yet", not "near".
- Hold no lock or transaction across the notifier's `.await`; keep the 3000-byte payload via `fit_payload`.
- `AppState.gtt == None`: tracking works on OTP; `vehicle` is always null.

## Done criteria

- `step` tests from fixtures: on time, getting later (alerts at 3 and 6), cancelled, never found (recurring), found then vanished, late start (no stale reminder), restart mid-run (no duplicate), stopped while evaluating.
- Scheduler tests with `FixedClock`: recurring rule Mon–Fri starts at `at − watch_from`, never on holidays or paused days; one-off starts at creation.
- API tests for the four tracking routes and the one-off hiding in `/api/rules`; web unit tests (router, sheet payload, countdown); `npm run check`.
- The three command groups pass; migration (if any) tried on a copy of `DATA_DIR`.
- By hand in Chrome on the Mac: "Avvisami" on a real urban departure 12 minutes away gets the 10 and 5 reminders and the live page shows vehicle and stops away.
- Docs: architecture (Scheduler, new Tracking section, code map), `docs/user/` (API, web app), chapter `docs/development/19-tracking.md`, phase-3.md (P, sections 4, 6, 9, 10, 12).

## Agent tasks (Sonnet, effort medium)

1. Backend (alone): `TrackConfig.trip_id`, `TripUpdates::vehicle`, `src/scheduler/tracking.rs` (`step`, messages), tick integration, store helpers, migration `0007` (AY, AZ), tests.
2. API (`src/api/trackings.rs`, `src/api/rules.rs`, `src/api/mod.rs`) with tests — **in parallel with 3** (no common files).
3. Web: router, `api.ts`, sheet, live page, Notifiche section, Vitest — reads the three mockups and `Notifiche.dc.html`.
4. Docs (docs-sync skill). Then one Opus review of the diff against this plan.

## Commit split (proposed, approved before committing). One PR from `feat/tracking`.

| Type(scope) | Subject | Covers |
|---|---|---|
| `docs(plan)` | plan phase 3 step 6, trip tracking | this file, phase-3.md decisions |
| `feat(scheduler)` | track runs with reminders, delay and cancellation alerts | task 1 (migration 0007, backlog item removed), its docs |
| `feat(api)` | trackings endpoints | task 2, API docs |
| `feat(web)` | avvisami sheet, live tracking page, trackings in Notifiche | task 3, user docs, chapter 19 |

## Answered questions (2026-11-16, all as recommended)

1. Vehicle and stops away from GTT's trip updates only; `vehicle_position.aspx` not read (AW, changes P).
2. A reminder already well past when tracking starts or resumes is skipped, never sent late (AX).
3. `observed_delays` is written from this step; step 9 keeps `/api/delays` and the UI (AY).
4. Alerts already notified become per rule (AZ, backlog item).
5. "Smetti di seguire" on a recurring rule stops today's run only (BA).

## Requested during the manual check (2026-11-16)

- Alert for a run that is **early**, not only late: not done in this step, recorded in [backlog.md](../../backlog.md).
- Delay label in the expanded views (sheet and live page) reads "−3 min in anticipo" / "+9 min in ritardo"; the stop list keeps "−3" / "+9". Done.

## Report (2026-11-16)

**Built.** Tracking of one run: `TrackConfig.trip_id`, `TripUpdates::vehicle` (vehicle and stops away from trip updates only), the pure `step` and the tick integration in `src/scheduler/tracking.rs`, migration `0007` (`observed_delays`, `rule_notified_alerts`), alerts per rule, `/api/trackings` (create, list, detail, delete), one-offs hidden in `/api/rules`, the "Avvisami" sheet, the live page `/corsa/{id}` and "Corse seguite" in Notifiche. Checked by hand in Chrome on a copy of the data folder: reminders at 10 and 5 minutes, vehicle and stops away on the live page.

**Where the plan was wrong.**
- `step` also needs the occurrence time and the stop label, so it takes a `StepInput` and returns a `Step`, not a bare tuple.
- The tick must also look at tomorrow for every track rule (a rule at 00:10 starts the evening before), not only for one-offs; found in the review, with the detail and delete of a recurring rule following the active run across midnight.
- `create_one_off` must match the rule by stop, trip and date (trip ids repeat every day).
- Recurring runs with no row are skipped when more than 10 minutes late or when the rule was created after `at` (not in the plan).

**For the next work.** Backlog items from the review: loop lines (one-off follows the first passage), paused rule leaving its run `tracking`, two narrow creation races, `rule_notified_alerts` growth, alert for an early run. Step 7 adds the rule editor and `/test` for tracking rules, and the "Crea una regola" link in the sheet. Reminders and a delay alert in the same tick share one `tag`, so the second replaces the first on the device (plan decision 14); worth a look on iPhone.
