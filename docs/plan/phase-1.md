# Phase 1 — Core backend: detailed plan

Status: **done**, released as `v0.1.0` on 2026-10-30. Decisions confirmed on 2026-10-29.

> Phase numbering changed on 2026-10-30: the web app and first deployment became Phase 2, rules and notifications Phase 3. On 2026-11-02 the first deployment moved to Phase 4. References below use the new numbering.

## 1. Goal

An HTTP API, packaged as a Docker image for the Orange Pi 5, that can:
- search for a stop by name, by code, or by location;
- show the next departures from a stop, filtered by line;
- store favorite stops.

### Definition of done

- [x] Searching `porta nuova` returns GTT stops 39, 40 and 253 among the first results.
- [x] Searching `39` returns stop 39 as the first result.
- [x] Departures for stop 39 match those shown on muoversiatorino.it.
- [x] Times stay correct after midnight and across daylight saving changes (covered by tests).
- [x] If the upstream is down, stop search still works and departures return a clear error (or stale data up to 2 minutes old).
- [x] The container survives a restart without losing data and uses less than 50 MB of RAM at idle (32 MB measured, arm64 under Docker on the Mac).
- [ ] The container runs on the Orange Pi: deferred until the first deployment, planned in Phase 4.

## 2. Decisions

| # | Question | Decision |
|---|----------|----------|
| A | Which feeds to index? | All four (about 19,400 stops, 5.7 MB, downloaded in 2 seconds); filtering happens at search time, defaulting to `gtt` only |
| B | Authentication | No login; optional token (`API_TOKEN`) required when set |
| C | Users | Single user: favorites are not tied to an account |
| D | Delay history | Not in this phase; the schema does not rule it out. Decided later: yes, for tracked runs, in Phase 3 |
| E | Compatible `/query.php?stop=` endpoint | No |
| F | How the image gets to the Orange Pi | Built on the Mac (`linux/arm64`, native on Apple Silicon) and pushed to Docker Hub as a public image (`federicosbringhetti/intime`), following the home server conventions. Changed from GHCR on 2026-10-29 |

## 3. What we know about the upstream

Verified on 2026-10-29 against `https://plan.muoversiatorino.it/otp/routers/mato/index/graphql`.

**Feeds**
| Feed | Content | Stops |
|------|---------|-------|
| `gtt` | GTT urban, suburban, rail (SFM), tourist | 7,058 |
| `2` | Trenitalia | 447 |
| `3` | Arriva Italia | 1,358 |
| `4` | Regional bus companies (GrandaBus, SAV, etc.) | 10,559 |

**Quirks**
- For GTT the stop code matches the part after `gtt:`, except for the 23 metro stops (`gtt:8213`–`gtt:8227`), which have `code: null`. The id is used as a fallback code.
- Codes repeat across feeds (1,523 duplicates), so the internal key is always the full `gtfsId`.
- Names are uppercase with abbreviations (`IVREA (V. CASALE/PONTE BREDA)`, `C.SO`, `P.ZA`): they must be normalized for search.
- `stops(name: ...)` always returns an empty list: name search must be done locally.
- Times are seconds from `serviceDay`, a Unix timestamp marking the start of the service day. They can exceed 86,400 for trips after midnight.
- Unknown stop: `{"data":{"stop":null}}` with HTTP 200.
- Invalid query: HTTP 500 with an `errors` array.
- No rate limit or cache headers in responses.

## 4. Project structure

A single Rust crate.

```
intime/
├── Cargo.toml
├── migrations/               # SQL migrations (sqlx)
├── src/
│   ├── main.rs               # startup, wiring, graceful shutdown
│   ├── config.rs             # configuration from environment variables
│   ├── db.rs                 # SQLite pool and migrations
│   ├── time.rs               # serviceDay + seconds → Europe/Rome time
│   ├── upstream/
│   │   ├── mod.rs            # HTTP client, timeouts, retry, error handling
│   │   ├── queries.rs        # GraphQL queries
│   │   └── types.rs          # response types
│   ├── stops/
│   │   ├── sync.rs           # index download and refresh
│   │   ├── normalize.rs      # name normalization
│   │   └── search.rs         # search by text, code and location
│   ├── departures.rs         # departures and cache
│   ├── favorites.rs
│   └── api/
│       ├── mod.rs            # router
│       ├── stops.rs
│       ├── favorites.rs
│       ├── health.rs
│       └── error.rs          # JSON error format
├── tests/
│   └── fixtures/             # recorded upstream responses
├── Dockerfile
└── docker-compose.yml
```

**Main dependencies**: `axum`, `tokio`, `reqwest` (with `rustls`, no OpenSSL), `sqlx` (SQLite), `serde`, `chrono` + `chrono-tz`, `moka` (cache), `tracing`, `thiserror`.

## 5. Configuration

Everything through environment variables, with sensible defaults.

| Variable | Default | Purpose |
|----------|---------|---------|
| `BIND_ADDR` | `0.0.0.0:8080` | listen address |
| `DATA_DIR` | `data` | SQLite database directory (the Docker image sets `/data`) |
| `UPSTREAM_URL` | muoversiatorino URL | GraphQL endpoint |
| `UPSTREAM_TIMEOUT_SECS` | `10` | upstream call timeout |
| `DEPARTURES_CACHE_SECS` | `20` | departures cache lifetime |
| `STOPS_SYNC_TIME` | `04:00` | nightly sync time (Europe/Rome) |
| `DEFAULT_FEEDS` | `gtt` | feeds used in search when not specified |
| `API_TOKEN` | empty | when set, `/api/*` requires `Authorization: Bearer <token>`; `/health` stays open. Without it the service logs a warning at startup |
| `RUST_LOG` | `info` | log level |

## 6. Upstream client

- A single shared `reqwest::Client` instance, with timeouts, an explicit `User-Agent` and gzip compression.
- Two queries:
  - `StopDepartures`: `stop(id)` with details and `stoptimesWithoutPatterns` (number of departures and time window as parameters);
  - `AllStops`: full list with coordinates, vehicle mode and lines.
- GraphQL variables are always passed as `variables`, never concatenated into the query string.
- Distinct errors: upstream unreachable, timeout, non-2xx HTTP, response with `errors`, unknown stop (`stop: null`).
- A single retry after 300 ms for transient failures: connection errors and HTTP 502–504. No retry for timeouts (they already took the whole timeout), GraphQL errors or invalid responses.
- Every call records its outcome; `/api/status` shows it (last success, last failure and error, consecutive failures).

## 7. Times

A single function in `time.rs`:

```
time = serviceDay (Unix) + seconds  →  DateTime<Europe/Rome>
```

- Handles seconds above 86,400 without special cases, since it starts from the absolute timestamp.
- Delay = `realtimeDeparture - scheduledDeparture`, in seconds, possibly negative (early).
- Dedicated tests: trip after midnight, last Sunday of March, last Sunday of October.

## 8. Stop index

### Schema

```sql
CREATE TABLE stops (
  id          TEXT PRIMARY KEY,   -- gtfsId, e.g. "gtt:39"
  feed        TEXT NOT NULL,      -- "gtt", "2", "3", "4"
  code        TEXT NOT NULL,      -- code shown to the user
  name        TEXT NOT NULL,      -- original name
  description TEXT,
  lat         REAL NOT NULL,
  lon         REAL NOT NULL,
  mode        TEXT,               -- BUS, TRAM, SUBWAY, RAIL, FUNICULAR
  lines       TEXT NOT NULL,      -- JSON: [{"id": "...", "name": "4", "mode": "TRAM"}]
  search_name TEXT NOT NULL,      -- normalized name, used for ranking
  search_text TEXT NOT NULL       -- normalized name + description, used for matching
);
CREATE INDEX stops_code ON stops(code COLLATE NOCASE);
CREATE INDEX stops_lat_lon ON stops(lat, lon);

-- normalized text (name + description + expanded abbreviations)
CREATE VIRTUAL TABLE stops_fts USING fts5(id UNINDEXED, text, tokenize = 'trigram');

CREATE TABLE sync_state (
  key   TEXT PRIMARY KEY,         -- e.g. "stops"
  at    TEXT NOT NULL,            -- last successful sync
  count INTEGER NOT NULL
);
```

### Sync
- On startup if the index is empty or older than 24 hours, then every night at `STOPS_SYNC_TIME`.
- Full replacement within a single transaction: searches always see either the old or the new data, never a half-written index.
- If the sync fails, the old data stays and it retries with backoff (5 minutes, 15, 1 hour).

### Name normalization
- Lowercase and accent removal.
- Abbreviation expansion, from the most frequent abbreviations in real names: `c.so` → `corso`, `v.` → `via`, `p.za` / `p.zza` → `piazza`, `v.le` → `viale`, `fraz.` → `frazione`, `bv.` → `bivio`, `m.to` → `monferrato`, `t.se` → `torinese`, and others (full list in `src/stops/normalize.rs`). Abbreviations ending with a dot also match when glued to the next word (`v.cavagnolo`).
- Dotless forms (`cso`, `pza`) are expanded, and `santa` / `santo` / `sant` / `s.` all become `san`.
- The same normalization is applied to the search text.

### Search
1. A stop matches if its code or id equals the search text, or if every word of the search text appears in its name or description.
2. Words of 3 or more characters use the trigram index (also finds partial words, e.g. `nuov`); shorter words must start a word (`LIKE`).
3. Ordering: exact code or id; exact name; name starting with the text; name containing the text; match in the description only. Then `gtt` before other feeds, shorter names first, shorter codes first (`39` before `252`).
4. Searches return `503 index_not_ready` until the first sync has completed.

### Nearby stops
Computed locally, without calling the upstream: first a bounding-box filter on lat/lon (uses the index), then the exact distance (haversine) for ordering.

## 9. Departures

- Upstream call with the requested time window and number of departures.
- In-memory cache per stop (`moka`, 20 seconds). Concurrent requests for the same stop result in a single upstream call.
- Filters (lines, maximum count) are applied to the cached response, so changing a filter does not trigger new calls.
- Besides the fresh cache, the last good response per stop is kept for 2 minutes. If the upstream fails, that response is returned with `"stale": true`; otherwise an error. Every response has `fetched_at`, the time the data was received.

## 10. Favorites

```sql
CREATE TABLE favorites (
  id         INTEGER PRIMARY KEY,
  stop_id    TEXT NOT NULL,       -- gtfsId; no foreign key, the stop may disappear after a sync
  label      TEXT,                -- e.g. "Home → work"
  lines      TEXT NOT NULL,       -- JSON array of line names; empty means all lines
  position   INTEGER NOT NULL,    -- 0-based manual order, kept contiguous
  created_at TEXT NOT NULL
);
```

- New favorites go to the end. Moving one shifts the others; deleting one closes the gap.
- Creating a favorite requires a synced index and a stop that exists in it (`404 stop_not_found` otherwise), so typos do not create dead favorites.
- Each favorite in a response carries its `stop` details. If the stop no longer exists after a sync, `stop` is `null` and `"missing": true`, instead of hiding the favorite.
- Labels are trimmed (at most 100 characters, empty means none); lines are trimmed and deduplicated (at most 20, each at most 10 characters).
- Request bodies reject unknown fields.

## 11. API

All responses are JSON. Times are in ISO 8601 format with time zone.

In paths, `{stop}` accepts the GTT code (`39`) or the full `gtfsId` (`gtt:39`, `2:S00219`).

| Method | Path | Description |
|--------|------|-------------|
| GET | `/api/stops?q=&feeds=&limit=` | search by name or code (`feeds`: `all` or a list, default `DEFAULT_FEEDS`; `limit` 1–100, default 20) |
| GET | `/api/stops/nearby?lat=&lon=&radius=&feeds=&limit=` | nearby stops with `distance_m` (radius 1–2000 m, default 300 m) |
| GET | `/api/stops/{stop}` | stop details with lines |
| GET | `/api/stops/{stop}/departures?lines=&minutes=&limit=` | next departures (`minutes` 1–120, default 60; `limit` 1–100, default 20) |
| GET | `/api/favorites` | list favorites in order, with stop details |
| POST | `/api/favorites` | add a favorite: `{"stop": "39", "label": "...", "lines": ["4"]}` → 201 |
| PATCH | `/api/favorites/{id}` | change `label` (`null` clears it), `lines` or `position` |
| DELETE | `/api/favorites/{id}` | delete a favorite → 204 |
| GET | `/health` | the process is alive (for Docker) |
| GET | `/api/status` | version, outcome of recent upstream calls, last stop sync and stop count |

### Example: departures

```json
{
  "stop": { "id": "gtt:39", "code": "39", "name": "PORTA NUOVA" },
  "generated_at": "2026-10-29T16:12:05+01:00",
  "fetched_at": "2026-10-29T16:12:01+01:00",
  "stale": false,
  "departures": [
    {
      "line": "67",
      "mode": "BUS",
      "headsign": "MONCALIERI, TESTONA - PIAZZA MARCONI",
      "scheduled": "2026-10-29T15:59:00+01:00",
      "expected": "2026-10-29T16:11:29+01:00",
      "delay_seconds": 749,
      "realtime": true,
      "canceled": false,
      "trip_id": "gtt:28459640U",
      "minutes": 0
    }
  ]
}
```

### Error format

```json
{ "error": { "code": "stop_not_found", "message": "Stop 99999 not found" } }
```

| Case | HTTP | `code` |
|------|------|--------|
| Invalid parameters | 400 | `invalid_request` |
| Missing or wrong token | 401 | `unauthorized` |
| Unknown stop | 404 | `stop_not_found` |
| Upstream error | 502 | `upstream_error` |
| Upstream timeout | 504 | `upstream_timeout` |
| Unknown favorite | 404 | `favorite_not_found` |
| Stop index not synced yet | 503 | `index_not_ready` |
| Database error | 500 | `internal_error` |

## 12. Docker

- **Multi-stage Dockerfile**:
  - build with `rust:1.95-slim-bookworm` and `cargo-chef`, so dependencies stay cached between builds;
  - runtime `debian:bookworm-slim` with only `ca-certificates` and `tzdata`;
  - non-root user.
- **Health check** without curl in the image: the binary itself has a `healthcheck` subcommand that calls `/health`.
- **docker-compose.yml**, following the home server conventions: folder `/home/federico/intime/`, explicit `container_name`, host port 8100 → container 8080 (8080 is taken on the host), bind mount `./data:/data` (what the backup saves), `env_file: .env` with `.env.example` in the repository, `TZ: Europe/Rome`, `healthcheck`, `restart: unless-stopped`, 128 MB memory limit. The `data` folder must belong to uid 10001.
- **`scripts/build-image.sh [--push]`** builds the arm64 image tagged with the Cargo version and `latest`, and optionally pushes it to Docker Hub.
- Target: image under 40 MB. Measured: 38 MB to download, 138 MB uncompressed (108 MB Debian base, 15 MB binary). A distroless base would roughly halve the uncompressed size but removes the shell used for debugging on the Pi.
- Deployment steps: [../user/deploy.md](../user/deploy.md).

## 13. Tests

- **Unit**: time conversion (midnight, daylight saving), name normalization, search ordering.
- **With fixtures**: real upstream responses saved in `tests/fixtures/` and served by a fake server (`wiremock`). Cases covered: normal response, unknown stop, HTTP 500, timeout.
- **API**: endpoint tests against a temporary SQLite database.
- **Manual smoke test**: a script that calls the real upstream, to notice when something changes. Not part of the automated tests.
- **Code quality**: `cargo fmt`, `cargo clippy` and `cargo test` in GitHub Actions on every push.

## 14. Development order

Each step ends with something verifiable and is developed in its own branch with a PR to `main`. When all steps are complete, `v0.1.0` is released.

1. **Skeleton**: crate, configuration, logging, `/health`, CI. Check: `curl /health` responds.
2. **Departures**: upstream client, time conversion, `departures` endpoint with cache. Check: manual comparison with muoversiatorino.it for stop 39.
3. **Stop index**: migrations, sync, normalization, search, nearby stops, details. Check: the search criteria in section 1.
4. **Favorites**: table and endpoints.
5. **Robustness**: `stale` responses, retry, `/api/status`, optional token.
6. **Docker and deployment**: arm64 image, compose, deployment guide. Check: all criteria in section 1 in a local container. The first start on the Orange Pi is deferred to Phase 4.

## 15. Out of scope for this phase

- Rules, scheduler and notifications (Phase 3)
- Delay history (Phase 3)
- Service alerts (`alerts`): the upstream always returns an empty list, to be re-checked later
- Web interface (Phase 2)
- Multiple users

## 16. Risks

| Risk | Mitigation |
|------|------------|
| The upstream changes schema or stops responding | Clear errors, smoke test, search independent of the upstream; automatic alert in Phase 3 |
| The upstream starts rate limiting | Cache, a single call per stop, sync once a day |
| Incomplete stop data (e.g. metro without code) | Fallback code from the id, tests on known cases |
