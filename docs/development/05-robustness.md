# 5. Robustness

> Phase 1, step 5 · PR [#7](https://github.com/fedsbringhetti/intime/pull/7)

## Goal

Make the service behave well when the unofficial upstream misbehaves, show what is going on, and allow restricting access:

- one retry on transient failures;
- the last good departures served as `stale` when the upstream fails;
- `GET /api/status` with the upstream health and the stop index state;
- an optional bearer token (`API_TOKEN`) for `/api/*`.

## Decisions

| Decision | Why |
|----------|-----|
| **Retry once, after 300 ms, only on connection errors and HTTP 502–504** | These are often temporary. Timeouts are not retried: they already waited the whole timeout (10 s), and retrying would double the wait. GraphQL errors and invalid responses would fail the same way again. The plan said "network errors"; 502–504 were added and timeouts excluded. |
| **Stale data for at most 2 minutes**, with `fetched_at` in every response | Departures older than that are more misleading than useful. The client sees how old the data is. |
| **Passive health tracking** (every call records its outcome) | No extra traffic to the upstream. Phase 3 will add a probe and alerts on top. |
| **Token optional, `/health` always open** | Without a token the service is usable on a trusted network; Docker's health check has no token. |
| **Constant-time comparison; token never logged** | Response times do not reveal how much of a guess was right; logs and config errors stay safe to share. |

## Files

| File | Role |
|------|------|
| `src/upstream/mod.rs` | retry logic, health recording |
| `src/upstream/health.rs` | `UpstreamHealth`, `HealthSnapshot` |
| `src/departures.rs` | `last_good` cache, `Fetched { data, stale }`, `fetched_at` |
| `src/api/status.rs` | `GET /api/status` |
| `src/api/auth.rs` | `ApiToken`, `require_token` middleware |
| `src/api/mod.rs` | protected routes, open `/health` |
| `src/api/error.rs` | `unauthorized` with `WWW-Authenticate: Bearer` |
| `src/config.rs`, `src/main.rs` | `API_TOKEN`, warning when unset |
| `tests/status.rs`, `tests/auth.rs`, `tests/departures.rs` | new integration tests; `Options` in `tests/common` |

## How the code works

### Retry and health

Every query goes through one function:

```rust
let mut result = self.send_query(query, &variables, timeout).await;
if let Err(err) = &result
    && err.is_transient()
{
    tracing::debug!("retrying after transient upstream failure: {err}");
    tokio::time::sleep(RETRY_DELAY).await;
    result = self.send_query(query, &variables, timeout).await;
}

match &result {
    Ok(_) => self.health.record_success(Utc::now()),
    Err(err) => self.health.record_failure(Utc::now(), err),
}
result
```

```rust
fn is_transient(&self) -> bool {
    matches!(self, Self::Unreachable(_) | Self::Status(502..=504))
}
```

> **Rust concept: `if let ... && ...`.** Since Rust 2024, `if let` can be chained with conditions ("let chains"): the block runs only if `result` is an error *and* that error is transient.

`UpstreamHealth` holds a `HealthSnapshot` behind a `Mutex`, shared through an `Arc` by the client, the sync task and the status endpoint:

```rust
pub struct HealthSnapshot {
    pub ok: Option<bool>,                 // None until the first call
    pub last_success: Option<DateTime<Utc>>,
    pub last_failure: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    pub consecutive_failures: u32,
}
```

A success resets `consecutive_failures`; a failure increments it and keeps the message. `last_failure` survives a later success, for diagnosis. Because the stop sync uses the same client, a failed nightly sync shows up here too.

> **Rust concept: `Mutex` poisoning.** If a thread panics while holding a `std::sync::Mutex`, the lock is marked "poisoned". Here a panic cannot leave the snapshot half-updated, so `lock()` recovers the guard with `into_inner()` instead of failing.

### Stale departures

`DeparturesService` now has two caches: the fresh one (20 s) and `last_good` (2 minutes), filled on every successful fetch:

```rust
match result {
    Ok(data) => Ok(Fetched { data, stale: false }),
    Err(err) if matches!(*err, DeparturesError::Upstream(_)) => {
        match self.last_good.get(stop_id).await {
            Some(data) => {
                tracing::warn!("serving stale departures for {stop_id}: {err}");
                Ok(Fetched { data, stale: true })
            }
            None => Err(err),
        }
    }
    Err(err) => Err(err),
}
```

Only upstream errors fall back; "stop not found" is a real answer and is returned as such. `StopDepartures` gained `fetched_at`, taken from the `Clock`, and the response exposes it next to `stale`.

### Status

`GET /api/status` combines the version, the current time, `upstream_health.snapshot()` and the last row of `sync_state`:

```json
{
  "version": "0.1.0",
  "time": "2026-10-29T22:19:51+01:00",
  "upstream": {"ok": true, "last_success": "...", "last_failure": null, "last_error": null, "consecutive_failures": 0},
  "stop_index": {"synced_at": "2026-10-29T16:50:15Z", "count": 19422}
}
```

### Token

The router now has two parts:

```rust
let mut api = Router::new()
    .route("/api/status", get(status::status))
    // ... all /api routes
if let Some(token) = state.api_token.clone() {
    api = api.route_layer(middleware::from_fn_with_state(token, auth::require_token));
}

// `/health` stays open: the Docker health check has no token.
Router::new()
    .route("/health", get(health::health))
    .merge(api)
    .with_state(state)
```

`route_layer` applies the middleware only to the routes defined so far, so new `/api` routes are protected automatically and unknown paths still give 404.

The middleware accepts `Authorization: Bearer <token>` (scheme case-insensitive) and otherwise returns `401 unauthorized` with `WWW-Authenticate: Bearer`. The comparison does not stop at the first different byte:

```rust
fn matches(&self, candidate: &str) -> bool {
    let (expected, candidate) = (self.0.as_bytes(), candidate.as_bytes());
    expected.len() == candidate.len()
        && expected.iter().zip(candidate).fold(0u8, |diff, (a, b)| diff | (a ^ b)) == 0
}
```

`ApiToken` implements `Debug` by hand as `ApiToken(***)`, and a config error for `API_TOKEN` prints `***` instead of the value. When `API_TOKEN` is unset, `main` logs a warning at startup.

## Tests

- **Unit**: which errors are transient; health counters; token parsing, matching and `Debug`; `check_range` rejecting NaN; config reading the token without leaking it.
- **Integration**:
  - a 503 followed by a 200 gives a normal response after one retry (the fake server counts exactly one call each);
  - a GraphQL error is not retried;
  - stale fallback: with a 50 ms cache, a good response then a failing upstream gives `stale: true` with the same departures and `fetched_at`;
  - `/api/status` before any call (`ok: null`), after a sync (`ok: true`, count 44), after failures (`ok: false`, two consecutive failures, the error message);
  - token missing, wrong, other scheme or without scheme gives 401 with the header; correct token in either case gives 200; `/health` stays open; no token configured means open.

`tests/common` gained `Options { cache_ttl, api_token }` and `test_app_with` for these cases.

## Check it by hand

```bash
API_TOKEN=secret cargo run
```

```bash
curl -s -o /dev/null -w '%{http_code}\n' http://localhost:8080/api/status
```

Prints 401.

```bash
curl -s -H "Authorization: Bearer secret" http://localhost:8080/api/status
```

```bash
UPSTREAM_URL=http://127.0.0.1:9/graphql RUST_LOG=info,intime::upstream=debug cargo run
```

With an unreachable upstream, a departures request shows the retry in the logs, answers 502 after about 0.3 s, and `/api/status` reports the failure.

## Pitfalls

- **Testing stale data** needs the fresh cache to expire: the test uses a 50 ms TTL and sleeps 100 ms, and the fake upstream answers once with data (`up_to_n_times(1)`), then with errors.
- **Stale data cannot be shown live**, since the real upstream cannot be made to fail on demand; it is covered by tests only.

## Later changes

- [Step 6](06-docker.md) uses the open `/health` for the Docker health check.
- [Step 7](07-web-skeleton.md) serves the web app outside the token check: it holds no data, and its API calls go through the check.
- Phase 3 builds the upstream alerts on `UpstreamHealth`.
- [Chapter 14](14-gtt-predictions.md): GTT's client reuses `UpstreamError` and `UpstreamHealth` and the same single retry; `/api/status` gets a `gtt` entry.
