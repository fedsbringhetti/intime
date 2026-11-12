# 15. GTT alerts

> Phase 3, step 2 · PR [#22](https://github.com/fedsbringhetti/intime/pull/22)

## Goal

GTT's service alerts (detours, limited lines, suspended stops) now reach the stop screen. The service reads them from GTT's open data every 5 minutes, links each one to the stops it concerns, returns them with the departures of a stop and from `/api/alerts`. The web app shows them in a banner closed by default and marks the departures of lines with alerts. Linking alerts to stops needs GTT's own stop ids, so this step also downloads `stops.txt` from GTT's static data, only when GTT has rebuilt it; the info screen says which version it has.

## Decisions

| Decision | Why |
|----------|-----|
| **Linked by stop when the alert lists stops, by line only when it lists none** (decision O changed) | The plan linked every alert by line. The real feed (119 alerts on 2026-11-04) has two kinds: a line alert lists nearly all the stops of its line (median 99%), while a stop alert ("Fermata 750 sospesa", 14 of them) lists one stop **and every line serving it**. By line, the alert of stop 750 would show at all 1,010 stops of its 12 lines. By stop, line alerts keep the same stops, and a detour in one direction shows only on that side. Without static data there is no way to map stops, so every alert falls back to its lines. |
| **An alert listing only unknown stops is shown at no stop** | 52 stop ids named by alerts are missing from `stops.txt` (temporary stops). Falling back to the alert's lines would bring back the problem above; such alerts are counted in `/api/status` and returned by `/api/alerts`. |
| **Static data checked every night with `If-Modified-Since`** (decision Q changed) | The plan said weekly. GTT answers a conditional request with `304` and no body when the 20 MB archive has not changed, so a nightly check costs nothing and new stops arrive the day after GTT rebuilds it. 10 minutes after the stop sync (04:10), so the two downloads do not overlap. |
| **Line names from the stop index** (decision AD) | Alerts know lines by GTT's `route_id` (`17BU`), while departures use OpenTripPlanner's names (`17/`). The stop index already stores id, name and mode of every line, the same names as the departures, so the warning on a departure row is a plain name match. `routes.txt` has the same names but misses the temporary lines too (`S45U`, `B1U`); those keep their id without the final `U` or `E`. |
| **No alerts in the favorites' departures** (decision AE) | No screen shows them yet; `/api/alerts?stop=` gives them when needed. |
| **The web app filters alerts by the lines on screen** | The server returns all the alerts of a stop whatever `?lines=`, as the web app already filters departures on the data it has. A stop alert names every line serving the stop, so it stays whatever the filter. |
| **`zip` with the deflate decoder in pure Rust only** (decision W) | `default-features = false` and `deflate-flate2-zlib-rs`: no bzip2, zstd, lzma or AES, no C library. |
| **The archive read in memory** | 20 MB once a week, freed right after; a temporary file would add cleanup for no gain. Unzipping and parsing run on a blocking thread. |
| **Only `http` and `https` links** | The URL comes from GTT and ends up in an `href`: a `javascript:` link would run in the app. |

## Files

| File | Role |
|------|------|
| `migrations/0003_gtt_static.sql` | `gtt_stops`; `version` column in `sync_state` |
| `src/gtt/static_data.rs` | `GttStatic`: conditional download, `read_stops`, replacement in one transaction, nightly task |
| `src/gtt/alerts.rs` | `Alerts` (one read, linked) and `AlertsService` (reading in the background, last alerts kept) |
| `src/gtt/mod.rs` | `GttRealtime::alerts`, one `read` with retry for both feeds, `alerts_health` |
| `src/stops/mod.rs`, `src/stops/sync.rs` | `StopIndex::lines`; scheduling helpers shared with the static data task |
| `src/api/alerts.rs`, `src/api/stops.rs`, `src/api/status.rs` | `/api/alerts`, `alerts` in the departures, `alerts` and `static_data` in `/api/status` |
| `src/config.rs`, `src/main.rs`, `src/api/mod.rs`, `src/api/error.rs` | `GTT_ALERTS_POLL_SECS`, `GTT_GTFS_URL`, startup of the two tasks, state |
| `tests/alerts.rs`, `tests/gtt_static.rs`, `tests/common/mod.rs`, `tests/fixtures/gtt/` | integration tests on the recorded alerts and real rows of `stops.txt` |
| `web/src/lib/alerts.ts`, `web/src/lib/components/AlertsBanner.svelte` | the banner and what it says |
| `web/src/lib/components/DepartureRow.svelte`, `Icon.svelte`, `web/src/screens/Stop.svelte` | the warning on departures, new icons, the banner on the stop screen |
| `web/src/lib/api.ts`, `web/src/lib/status.ts`, `web/src/screens/Info.svelte` | types; the "Dati statici GTT" row |

## How the code works

### Static data, only when it changed

`GttStatic::check` sends back the `Last-Modified` stored at the last download:

```rust
        if let Some(version) = &stored {
            request = request.header(header::IF_MODIFIED_SINCE, version);
        }
        let response = request.send().await.map_err(UpstreamError::from_reqwest)?;
        let status = response.status();
        if status == StatusCode::NOT_MODIFIED {
            sqlx::query("UPDATE sync_state SET at = ? WHERE key = ?")
```

A `304` only records the check. A `200` is unzipped and parsed off the async threads (`spawn_blocking`); `read_stops` keeps the stop id and our id `gtt:<stop_code>` of each row, skipping stations:

```rust
        let is_station = location_type.is_some_and(|index| field(index) == "1");
        if is_station || field(id).is_empty() || field(code).is_empty() {
            continue;
        }
```

The rows replace `gtt_stops` in one transaction, with the new version in `sync_state`. The nightly task reuses `next_sync_at` and `retry_delay` from the stop sync, and runs a callback after each download: in `main.rs` it asks the alerts to be read again, so they are linked to the new stops at once.

```rust
        static_data::spawn(gtt_static.clone(), clock.clone(), check_time, move || {
            if let Some(alerts) = &alerts {
                alerts.refresh_soon();
            }
        });
```

### Linking an alert

`Alerts::from_feed` walks the entities each alert names. Stops are mapped through `gtt_stops`; lines through the stop index:

```rust
                if let Some(stop) = &informed.stop_id {
                    lists_stops = true;
                    match stop_ids.get(stop) {
                        Some(id) if !stops.contains(id) => stops.push(id.clone()),
                        Some(_) => {}
                        None => {
                            unknown_stops.insert(stop.clone());
                        }
                    }
                }
```

The known stops go into a map from stop to alerts; the alert is marked to be linked by line only when it listed no stop, or when there is no static data:

```rust
                by_line: !lists_stops || stop_ids.is_empty(),
```

`for_stop` puts both together: the alerts listing the stop, then those linked by a line serving it, each once, active now, without duplicate texts:

```rust
        let mut indexes: Vec<usize> = listed.chain(by_line).collect();
        indexes.sort_unstable();
        indexes.dedup();
```

Which periods count: the specification now says `communication_period` and deprecates `active_period`, which is the only one GTT fills:

```rust
#[allow(deprecated)]
fn shown_periods(alert: &proto::Alert) -> &[proto::TimeRange] {
    if alert.communication_period.is_empty() {
        &alert.active_period
    } else {
        &alert.communication_period
    }
}
```

### Reading in the background

`AlertsService` keeps the last linked alerts behind an `RwLock<Arc<Alerts>>`: a request clones the `Arc` and works on it while a new read may replace it. The task waits for whichever comes first, the interval or a request to read again:

```rust
            tokio::select! {
                () = tokio::time::sleep(every) => {}
                () = service.wake.notified() => {}
            }
```

> **Rust concept: `tokio::select!`.** It waits on several futures at once and runs the branch of the first one to finish, dropping the others. Here the sleep is simply abandoned when `refresh_soon` calls `Notify::notify_one`; a notification sent while the task is busy reading is kept and wakes the next wait at once.

### In the API and the web app

The departures handler adds `stop_alerts`, which needs the lines serving the stop from the index for alerts linked by line. `/api/alerts` uses the same function for `?stop=`, `for_line` for `?line=` and `all` without parameters.

On the stop screen, the alerts follow the line filter and mark the departures of their lines:

```svelte
  let alerts = $derived(alertsForLines(data?.alerts ?? [], lines))
  let alertLines = $derived(linesWithAlerts(alerts))
```

`AlertsBanner` starts closed (decision AA): the number of alerts and their titles in one line; opened, each alert with its line badges, title, text (with GTT's line breaks, `white-space: pre-line`) and "Dettagli su gtt.to.it". It sits in a `{#key stopId}` block, so another stop starts closed again.

## Tests

- **Unit, Rust**: `static_data.rs` (3): stop ids with stations and rows without a code skipped, archives without `stops.txt` or its columns refused, `Last-Modified` parsed. `alerts.rs` (7): by stop and by line (the stop alert of 750 stays at 750; an alert with only unknown stops at no stop), everything by line without static data, active periods with their offsets, line names from the index and the fallback, identical alerts once at a stop, only web links and the title from the description, the recorded feed (119 alerts, all with links). `config.rs` (1).
- **Integration**: `tests/gtt_static.rs` (2), against a fake GTT: first download (26 stops, version), then `304` a day later with the check recorded and the stops kept; an archive with no stops and a `503` keep the stored stops, and only successful checks count. `tests/alerts.rs` (6), with the stop index of `all_stops.json`, the departures of stop 39 and the alerts recorded on 2026-11-04: `/api/status`; a failed read keeps the 119 alerts; the 8 alerts of stop 39 in the departures (whatever `?lines=`), the period of an alert in summer time; `/api/alerts` by stop, line, both, the metro stop 8224 (an alert naming line M1 and no stop), a stop without alerts, invalid and unknown stops; without static data, alerts linked by line; without GTT, empty lists.
- **Web** (Vitest, 11 new): the banner's count and titles, alerts for the lines on screen, the lines to mark, the link label; the static data row (version, no date, failed check with and without data, waiting, disabled).
- **By hand**, in the browser at phone size, with the server reading the real feeds and a copy of the data folder: at startup the alerts were read before the archive (3,988 unknown stops), then read again after the download (7,051 stops, version of 1 November; 52 unknown stops). Stop 39: closed banner "8 avvisi GTT", warnings on lines 68+, 9, 67 and 68, the banner opened; stop 750: the stop alert with 12 badges; `?linee=33` hides the banner, `?linee=9,33` shows one alert; dark theme; the info screen in light and dark.

Results: 148 Rust tests (19 new), 88 Vitest tests.

## Check it by hand

```bash
cargo run
```

```bash
curl -s "localhost:8080/api/alerts?stop=39" | jq '.alerts[] | {title, lines: [.lines[].name]}'
```

```bash
curl -s localhost:8080/api/status | jq '.gtt.alerts, .gtt.static_data'
```

To try a branch with a new migration without touching the usual database, start it on a copy: `DATA_DIR=/tmp/gtt-copy cargo run` after copying `data/` there. Once a migration has run, the code of `main` refuses that database (see Pitfalls).

## Pitfalls

- **A stop alert is also a line alert.** The plan's rule looked right on the numbers measured on 2026-11-02 (a median of 72 stops per alert), but those medians hid the 14 "stop suspended" alerts that list every line serving the stop. Counting how many of each alert's stops belong to its lines showed the two kinds.
- **A comma in a date.** The test of the conditional request first failed: wiremock's `header(...)` matcher splits header values at commas, and `Sun, 01 Nov 2026 02:26:59 GMT` has one. A closure comparing the whole value replaced it.
- **A deprecated field.** Reading `active_period` produced a deprecation warning, so `cargo clippy -D warnings` would fail: the newer specification replaces it with `communication_period`. GTT fills only the old one, so both are read, the new one first.
- **Summer time in an alert.** An integration test expected `+01:00` and got `+02:00`: the alert started in September. The code was right; the test now checks the exact period.
- **Rows too long, a third time.** "17:14 · orario programmato" plus the warning does not fit the 181 px of a row at 375 px when the minutes have two digits; the text now gives way with an ellipsis and the warning stays. On the info screen, "Versione del 1 novembre" wrapped three times; the value became "del 1 novembre". A long alert title went below its badge; it now takes the room left beside the badges.
- **A migration and the usual database.** Running this branch on `data/gtt.db` would apply migration 3; back on `main`, sqlx refuses to start on a database with a migration it does not know. The check by hand ran on a copy of the folder.

## Later changes

None yet.
