# 14. GTT predictions

> Phase 3, step 1 · PR [#21](https://github.com/fedsbringhetti/intime/pull/21)

## Goal

Departures of urban runs now use GTT's own real-time predictions, read from GTT's open data, with OpenTripPlanner as fallback (Phase 3, decision N). Every departure says where its time comes from (`source`), `/api/status` reports on GTT's feed and on how its predictions compare with OpenTripPlanner's, and the info screen shows both.

Step 0 came first: the mockups of the Phase 3 screens, approved on the design canvas, and the decisions taken with them (Z–AC in the [plan](../plan/phase-3.md)).

## Decisions

| Decision | Why |
|----------|-----|
| **GTFS Realtime types generated once and kept in the repository** (`src/gtt/proto.rs`, by `tools/gtfs-rt-codegen`) | The plan chose the `gtfs-realtime` crate believing its types were already generated (decision U). They are not: its build script runs `prost-build`, which needs `protoc`, missing on the Mac, in Docker and in CI. Generating once with `protox` (a protobuf compiler in pure Rust) leaves the service with a single runtime dependency, `prost`, and no build script. Rejected: a `build.rs` with `prost-build` + `protox` (about thirty more crates at every clean build, in Docker and CI too); installing `protoc` in three places. |
| **No GTFS static in this step** | With badges kept in the colors by vehicle type (decision Z), `routes.txt` is not needed; `stops.txt` only links alerts to stops, so it moves to step 2. Predictions need no stop ids: GTT gives the stop's position in the run. |
| **`stopSequence` from OpenTripPlanner** | It matched GTT's `stop_sequence` and scheduled time for 6 runs out of 6 at stop 39. `stopPosition`, the name in newer OpenTripPlanner versions, does not exist in this server's schema. |
| **Merged at each request, not in the cache** (decision AB) | The two sources have their own caches and are read in parallel; the merge is a few hundred comparisons in memory. A failing source never spoils the other's cached data. |
| **Arrival used as the departure time** | GTT sends arrivals only; the stop lasts seconds. |
| **Each run is all times or all delays** | Measured on the recorded feed: 78% of runs give delays, 22% absolute times, never both. A delay carries over to the following stops; an absolute time at an earlier stop does not, so those departures stay on OpenTripPlanner. |
| **A cancellation from either source wins**, however old GTT's update is | A cancellation does not get "stale" like a prediction: it holds until the run would have left. |
| **GTT left alone for 30 seconds after a failure** | Departures wait for GTT. Without a pause, every request would wait for the timeout while GTT is down. The timeout itself is 5 seconds (GTT answers in under one). |
| **Comparison per day, once per run and stop** (decision AC) | Reloading a page must not count twice; a day matches how the info screen is read ("oggi"). |
| **GTT's client reuses `UpstreamError` and `UpstreamHealth`** | Same errors, same retry rule (once on connection errors and 502–504), same shape in `/api/status`, and the upstream alerts of step 8 can treat both alike. |

## Files

| File | Role |
|------|------|
| `tools/gtfs-rt-codegen/` | generator of `src/gtt/proto.rs`: the official `gtfs-realtime.proto`, `prost-build` + `protox`; its own Cargo workspace |
| `src/gtt/proto.rs` | generated GTFS Realtime types |
| `src/gtt/mod.rs` | `GttRealtime`: reading `trip_update.aspx`, cache, retry, pause, health, last feed summary |
| `src/gtt/trip_updates.rs` | `TripUpdates`: the feed indexed by run; `prediction` for one stop |
| `src/gtt/comparison.rs` | today's outcomes and their summary |
| `src/departures.rs` | `source`, `stop_sequence`, `service_date`; `with_gtt`; `get` reads both sources |
| `src/upstream/queries.rs`, `types.rs` | `stopSequence` in the departures query |
| `src/time.rs` | `service_date` |
| `src/config.rs`, `src/main.rs`, `src/api/mod.rs`, `src/api/status.rs` | settings, startup, state, the `gtt` entry of `/api/status` |
| `tests/gtt.rs`, `tests/fixtures/gtt/` | integration tests on GTT's feed and OpenTripPlanner's departures recorded together |
| `web/src/lib/api.ts`, `status.ts`, `screens/Info.svelte` | `source`, the `gtt` status, rows for GTT and the comparison |
| `docs/plan/phase-3.md`, `PLAN.md`, `docs/plan/phase-2.md` | decisions of step 0 and of this step |

## How the code works

### Types without a protobuf compiler

`tools/gtfs-rt-codegen` is a separate Cargo project (it has its own `[workspace]`, so building the service never compiles it). It compiles the `.proto` with `protox` and hands the result to `prost-build`:

```rust illustrative
    let descriptors = protox::compile(["gtfs-realtime.proto"], [dir])?;
```

The output is written to `src/gtt/proto.rs` with a header saying it is generated. `src/gtt/mod.rs` includes it with `#[rustfmt::skip]` and lints allowed, so `cargo fmt` and clippy leave it as generated.

### Reading the feed

`TripUpdates::from_feed` turns the message into a map from run (trip id, service date) to its stop updates:

```rust
                    let event = stop.departure.or(stop.arrival)?;
                    let event = match (event.time, event.delay) {
                        (Some(time), _) => Event::Time(DateTime::from_timestamp(time, 0)?),
                        (None, Some(delay)) => Event::Delay(TimeDelta::seconds(delay.into())),
                        (None, None) => return None,
                    };
```

The run's `start_date` (`20261104`) is its service date. On our side, `time::service_date` computes it from `serviceDay`: the date at its noon, so a run after midnight still belongs to the day before.

### The prediction for one stop

```rust
        let Some(update) = run
            .stops
            .iter()
            .rev()
            .find(|stop| stop.sequence <= sequence)
        else {
            return Prediction::None;
        };
        let expected = match update.event {
            Event::Time(time) if update.sequence == sequence => time,
            // An absolute time at an earlier stop says nothing about the delay here.
            Event::Time(_) => return Prediction::None,
            Event::Delay(delay) => scheduled + delay,
        };
```

Before this, a cancelled run returns `Canceled` and an update older than 3 minutes returns `None`. After it, a time more than 60 minutes from the scheduled one returns `Implausible`.

> **Rust concept: `let ... else`.** `let Some(update) = ... else { return ...; };` binds `update` when the pattern matches and otherwise runs the `else` block, which must leave the function. It keeps the main path unindented, instead of a `match` or `if let` around the rest of the function.

### Merging

`DeparturesService::get` reads both sources at the same time:

```rust
        let (fetched, gtt) = tokio::join!(self.get_otp(stop_id), gtt);
```

`get_otp` is the previous `get`, unchanged: OpenTripPlanner's cache, shared calls, stale data. When GTT answered, `StopDepartures::with_gtt` builds a copy where each departure goes through `Departure::with_gtt`:

```rust
                let merged = Self {
                    expected,
                    delay_seconds: (expected - self.scheduled).num_seconds(),
                    realtime: true,
                    source: Source::Gtt,
                    ..self.clone()
                };
```

> **Rust concept: struct update syntax.** `..self.clone()` fills every field not listed with the value from another instance. The merged departure lists only what GTT changes; line, headsign, trip id and `canceled` (a cancellation from OpenTripPlanner stays) come from the original.

The copy is sorted again by expected time: a late run can move after the next one. The cached data is never touched, so the next request merges again with newer predictions.

### When GTT fails

```rust
        if lock(&self.paused_until).is_some_and(|until| Instant::now() < until) {
            return None;
        }
```

`trip_updates` returns `None` instead of an error: a cached feed if there is one, otherwise nothing during the 30 seconds after a failure, otherwise a new read (shared by concurrent requests through `try_get_with`, like the departures cache). Departures then come from OpenTripPlanner, and `/api/status` shows the failure.

### The info screen

Three rows replace the single real-time row: "Previsioni GTT", "Orari in tempo reale" (muoversiatorino.it) and "Scarto tra le due" for today. `describeHealth` in `status.ts` describes both feeds; `describeComparison` turns the median into whole minutes and counts the predictions set aside:

```ts
  if (median !== null) value = median < 60 ? 'meno di 1 min' : `${Math.round(median / 60)} min`
```

## Tests

- **Unit, Rust**: `trip_updates.rs` (12): delay and time at the stop, delay carried over, time not carried over, stop passed, stale update (180 s used, 181 s ignored), implausible, cancellation even if old, unknown run or date, skipped stops, the recorded feed decoded (383 runs) and an invalid message rejected. `departures.rs` (5 new): merge and new order, cancellations from either source, implausible and stale predictions keep OpenTripPlanner, other feeds left alone, `source` of a departure without real time. `comparison.rs` (2), `config.rs` (1), `time.rs` (1: the service date, also on both daylight saving days).
- **Integration** (`tests/gtt.rs`, 5), on `fixtures/gtt/` recorded together on 2026-11-04 at 10:46:30: at stop 39, 14 departures out of 64 use GTT (the 64 of 10:57:36 expected at 10:52:36) and `/api/status` reports 383 runs, 14 used, 11 compared, median 191 s; the trip updates are read once for two requests and counted once; GTT disabled; GTT answering 503 (one retry, then not called again for the second request); an invalid message (not retried).
- **Web** (Vitest, 4 new): GTT working with the time of its newest data, failing, disabled; the comparison in minutes, "meno di 1 min", the singular and the empty day.
- **By hand**, in the browser at phone size, with the server reading the real feeds: departures at stop 39 with 10 of 30 from GTT; the info screen in light and dark; GTT failing (with `fetch` replaced in the page) shows "Non rispondono" and the technical detail; no console errors.

Results: 129 Rust tests (26 new), 77 Vitest tests.

## Check it by hand

```bash
cargo run
```

```bash
curl -s "localhost:8080/api/stops/39/departures?limit=10" | jq '.departures[] | {line, scheduled, expected, source}'
```

```bash
curl -s localhost:8080/api/status | jq .gtt
```

To see the fallback, start the server with `GTT_REALTIME_URL=` (empty): every `source` is `otp` or `scheduled` and `gtt` is `null`.

## Pitfalls

- **The crate that was not what it seemed.** `gtfs-realtime` 0.2 failed to build with "Could not find `protoc`": it generates its types at build time. Hence the generator in `tools/` (decision U changed in the plan).
- **A folder lost in a URL.** Joining `trip_update.aspx` to `.../das_gtfsrt` (no final `/`) replaces the last segment and gives `.../trip_update.aspx`. The integration tests caught it: the fake GTT answered 404. `GttRealtime::new` now adds the `/`.
- **GTT is often earlier, not later.** On the recorded feed most GTT predictions at Porta Nuova were minutes earlier than the timetable and than OpenTripPlanner (median difference about 3 minutes). It is what GTT says; the daily comparison is there to judge decision N after some weeks.
- **Old updates.** In the feed the median update is 50 seconds old, but one in ten is more than 23 minutes old: the 3-minute limit matters.
- **A row too long, again.** "Confronto tra le due · 3 min di scarto · nessuna previsione GTT scartata su 10" wrapped three times at 375 px, as in chapter 13; it became "Scarto tra le due · 3 min · nessuna scartata su 10".
- **A formatter that is not the project's.** Running `npx prettier` reformatted four web files with its own defaults (semicolons, double quotes); the files were restored and edited again. The project has no formatter for the web app: follow the style of the file.
- **The old version in the browser.** After `npm run build` the info screen still showed the old rows: the service worker serves its cached files until "Aggiorna" is tapped (chapter 12).

## Later changes

- [Chapter 15](15-gtt-alerts.md): `GttRealtime` also reads the alerts; both feeds go through one `read` with the retry, each with its own health.
