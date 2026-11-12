# 2. Departures

> Phase 1, step 2 · PR [#4](https://github.com/fedsbringhetti/intime/pull/4)

## Goal

The first real endpoint: `GET /api/stops/{stop}/departures` returns the next departures from a stop, with scheduled and real-time times, delay, line filters, and a short cache so the upstream is not called on every request.

## Decisions

| Decision | Why |
|----------|-----|
| **One GraphQL query per stop**: `stop(id: "gtt:39")` with `stoptimesWithoutPatterns` | The original PHP project made two calls (one to find an internal id, one for departures) and cached the id. Testing the API showed the first call is unnecessary: the "mysterious id" is just `base64("Stop:gtt:39")`, and `stop(id:)` already returns departures. |
| **Parameters as GraphQL `variables`** | Never building the query string from user input rules out injection by construction. |
| **Time = `serviceDay` + seconds** | Upstream times are seconds from `serviceDay`. The PHP code used "today + seconds", which breaks after midnight (seconds above 86,400) and on daylight saving days. |
| **Fetch 2 hours (up to 200 departures), filter per request** | One cache entry per stop serves every combination of `lines`, `minutes` and `limit`, so changing a filter never calls the upstream again. |
| **In-process cache (`moka`) of 20 seconds** | Real-time data changes slowly enough; one process and one user need no external cache such as Redis. `moka` also merges concurrent requests for the same stop into one upstream call. |
| **`Clock` trait** | Tests need a fixed "now" to compare against recorded data; the service uses the real clock. |
| **JSON errors with a stable `code`** | Clients (later the UI) can react to `stop_not_found` or `upstream_timeout` without parsing messages. |

## Files

| File | Role |
|------|------|
| `src/upstream/mod.rs` | HTTP client for the GraphQL API, error types |
| `src/upstream/queries.rs` | text of the `StopDepartures` query |
| `src/upstream/types.rs` | Rust structs matching the JSON response |
| `src/time.rs` | `service_time`, `Clock`, `SystemClock`, `FixedClock` |
| `src/departures.rs` | `Departure`, `StopDepartures`, `Filter`, `DeparturesService` with the cache |
| `src/api/stops.rs` | the departures handler, parameter validation, stop id parsing |
| `src/api/error.rs` | `ApiError` and the JSON error format |
| `src/config.rs` | `UPSTREAM_URL`, `UPSTREAM_TIMEOUT_SECS`, `DEPARTURES_CACHE_SECS` |
| `tests/departures.rs`, `tests/common/mod.rs`, `tests/fixtures/stop_39.json` | integration tests against a fake upstream |

## How the code works

Follow a request for `/api/stops/39/departures?lines=4,15`.

### 1. The handler extracts and validates

```rust
pub async fn departures(
    State(state): State<AppState>,
    Path(stop): Path<String>,
    params: Result<Query<DeparturesParams>, QueryRejection>,
) -> Result<Json<DeparturesResponse>, ApiError> {
    let Query(params) = params.map_err(|err| ApiError::InvalidRequest(err.body_text()))?;
    let stop_id = resolve_stop_id(&stop)
        .ok_or_else(|| ApiError::InvalidRequest(format!("Invalid stop {stop:?}")))?;
    let filter = params.into_filter()?;
```

- The arguments are **extractors**: `axum` fills them from the request. `State` is the shared application state, `Path` the `{stop}` segment, `Query` the `?lines=...` parameters.
- `Query` is taken as a `Result`: if a parameter is malformed (`minutes=abc`), the handler turns the rejection into our JSON `invalid_request` instead of axum's plain-text error.
- `resolve_stop_id` turns `39` into `gtt:39` and accepts `gtt:39` or `2:S00219` as they are. Anything with characters other than letters, digits, `_` and `-` is rejected before reaching the upstream.
- `into_filter` checks the limits (`minutes` 1–120, `limit` 1–100) and splits `lines` on commas.

### 2. The service: cache first

```rust
self.cache
    .try_get_with(stop_id.to_owned(), async {
        let stop = self
            .upstream
            .stop_departures(stop_id, FETCH_WINDOW, FETCH_MAX_DEPARTURES)
            .await?
            .ok_or_else(|| DeparturesError::StopNotFound(stop_id.to_owned()))?;
        // ...convert and wrap in Arc
    })
    .await
```

`try_get_with` returns the cached value when it is fresh. Otherwise it runs the `async` block, stores the result, and makes any other request for the same stop wait for that same result. Errors are not cached, so the next request tries again.

> **Rust concept: `Arc`.** The cached value is an `Arc<StopDepartures>`, a reference-counted pointer. Many requests read the same data at the same time without copying it; it is freed when the last reference goes away.

### 3. The upstream call

The query, with placeholders filled by `variables`:

```graphql
query StopDepartures($id: String!, $timeRange: Int!, $numberOfDepartures: Int!) {
  stop(id: $id) {
    gtfsId code name
    stoptimesWithoutPatterns(timeRange: $timeRange, numberOfDepartures: $numberOfDepartures, omitCanceled: false) {
      scheduledDeparture realtimeDeparture realtime realtimeState serviceDay stopSequence headsign
      trip { gtfsId tripHeadsign route { shortName mode } }
    }
  }
}
```

The response is decoded into structs from `types.rs`. `#[serde(rename_all = "camelCase")]` maps `gtfsId` to `gtfs_id`; fields that can be missing are `Option`. A stop that does not exist comes back as `"stop": null`, which becomes `Ok(None)`.

The order of checks on the response matters:

```rust
// GraphQL errors come with HTTP 500, so look at the body before the status.
match serde_json::from_slice::<GraphQlResponse<T>>(&body) {
    Ok(response) if !response.errors.is_empty() => Err(UpstreamError::GraphQl(/* ...messages joined with "; " */)),
    _ if !status.is_success() => Err(UpstreamError::Status(status.as_u16())),
    Ok(GraphQlResponse { data: Some(data), .. }) => Ok(data),
    Ok(_) => Err(UpstreamError::InvalidResponse("missing data".to_owned())),
    Err(err) => Err(UpstreamError::InvalidResponse(err.to_string())),
}
```

Checking the status first would lose the real message ("Field nonEsiste is undefined"), which is the signal that the upstream schema changed.

`UpstreamError` distinguishes `Timeout`, `Unreachable`, `Status`, `GraphQl` and `InvalidResponse`, so the API can answer 504 for a timeout and 502 for the rest.

### 4. Converting times

```rust
pub fn service_time(service_day: i64, seconds: i64) -> Option<DateTime<Tz>> {
    DateTime::from_timestamp(service_day.checked_add(seconds)?, 0)
        .map(|time| time.with_timezone(&TIME_ZONE))
}
```

`serviceDay` is not midnight: GTFS defines it as **noon minus 12 hours** on the service date. On a normal day that is midnight; on daylight saving days it is 23:00 or 01:00. Because the definition is built in, a plain sum is always right. A test confirms that the computed `serviceDay` for 2026-10-29 equals the value seen upstream (`1793228400`), and other tests cover a trip at 25:10 (01:10 the next day) and both daylight saving Sundays.

Each upstream departure becomes a `Departure`:

- `scheduled` and `expected` from `service_time`;
- `delay_seconds = realtimeDeparture − scheduledDeparture` (negative when early);
- `canceled` when `realtimeState` is `CANCELED`;
- `headsign` from the stop time, or from the trip when missing;
- `line` stays a `String`: lines are `9B`, `M1S`, `68+`, not numbers (the PHP code stored them as integers).

Departures are sorted by expected time. If the upstream has no `code` (metro stops), the code is taken from the id (`gtt:8224` → `8224`).

### 5. Filtering and the response

```rust
pub fn select(&self, filter: &Filter, now: DateTime<Utc>) -> Vec<&Departure> {
    let now = now.with_timezone(&TIME_ZONE);
    let until = now + filter.window;
    self.departures
        .iter()
        .filter(|departure| departure.expected >= now && departure.expected <= until)
        .filter(|departure| {
            filter.lines.is_empty()
                || filter.lines.iter().any(|line| line.eq_ignore_ascii_case(&departure.line))
        })
        .take(filter.limit)
        .collect()
}
```

`now` is a parameter instead of being read inside, which is what makes this function testable with fixed times. The handler then adds `minutes` (rounded down, never negative) and builds the JSON.

> **Rust concept: iterators.** `.iter().filter(...).take(...).collect()` describes the pipeline; nothing runs until `collect` builds the result. The closures (`|departure| ...`) are small inline functions.

### 6. Errors

`ApiError` implements `IntoResponse`, so a handler can return it with `?`:

| Variant | HTTP | `code` |
|---------|------|--------|
| `InvalidRequest` | 400 | `invalid_request` |
| `StopNotFound` | 404 | `stop_not_found` |
| `UpstreamTimeout` | 504 | `upstream_timeout` |
| `Upstream` | 502 | `upstream_error` |

`From<Arc<DeparturesError>>` converts the service error into the right variant and logs upstream problems as warnings.

## Tests

- **Unit**: time conversion (normal day, after midnight, both daylight saving days, overflow), departure mapping, code fallback, sorting, filters (window, lines ignoring case, limit), stop id parsing, minutes rounding.
- **Integration** (`tests/departures.rs`): the app talks to a `wiremock` server that replays `fixtures/stop_39.json`, a real response saved from the upstream. The clock is fixed 30 seconds before the first departure in that file, so the tests pass any day.
  - The exact JSON of the first departure is checked field by field.
  - The fake server counts calls: `.expect(1)` proves that three requests with different filters make a single upstream call (cache), and `.expect(0)` proves that invalid parameters never reach the upstream.
  - Unknown stop → 404, GraphQL error with HTTP 500 → 502 with the real message, slow upstream (2 s delay, 200 ms timeout) → 504.

## Check it by hand

```bash
cargo run
```

```bash
curl -s 'http://localhost:8080/api/stops/39/departures?limit=5'
```

Compare with the same stop on muoversiatorino.it: `https://www.muoversiatorino.it/stops/gtt%3A39`. At verification time both showed the same lines in the same order (68+, 64, 68, 52, 61), the same destinations and the same minutes.

## Pitfalls

- **`serviceDay` is not midnight.** Testing against the real value from the upstream is what proved the "noon minus 12 hours" definition.
- **Status before body** loses GraphQL error messages, as explained above.
- **`generated_at` with microseconds** looked noisy; it is truncated to whole seconds.
- **Lines are strings.** `9B` and `68+` would break any numeric handling.

## Later changes

- [Step 3](03-stop-index.md) moves the code fallback to `stops::display_code`, shared with the index.
- [Step 5](05-robustness.md) adds a retry on transient failures, `fetched_at`, and `stale: true` responses from a "last good" cache.
- [Chapter 14](14-gtt-predictions.md): the query also asks for `stopSequence`; each departure gets `source`, and `DeparturesService::get` merges GTT's predictions into OpenTripPlanner's departures.
