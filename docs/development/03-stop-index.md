# 3. Stop index

> Phase 1, step 3 · PR [#5](https://github.com/fedsbringhetti/intime/pull/5)

## Goal

A local copy of all upstream stops in SQLite, refreshed every night, with three endpoints on top of it: search by name or code (`/api/stops?q=`), stops near a point (`/api/stops/nearby`) and stop details (`/api/stops/{stop}`).

## What the upstream offers

Checked before writing any code:

- `stops(name: ...)` always returns an empty list, so **name search must be local**.
- `stops { ... }` returns every stop of every feed in one query: about **19,400 stops, 5 MB, 2 seconds**.
- Four feeds: `gtt` (GTT urban, suburban, SFM rail, tourist; 7,058 stops), `2` Trenitalia (447), `3` Arriva (1,358), `4` regional bus companies (10,559).
- Codes repeat across feeds (1,523 duplicates), so the key is always the full `gtfsId`.
- The 23 GTT metro stops have `code: null`.
- Names are uppercase and full of abbreviations. Counting the tokens with a dot in all names gave the list to expand: `v.` (1,148 times), `bv.` (389), `c.so` (246), `s.` (229), `fraz.` (212), `m.to` (198), `p.za` (197)...

## Decisions

| Decision | Why |
|----------|-----|
| **Index all four feeds, search `gtt` by default** (`DEFAULT_FEEDS`) | Indexing everything costs little; GTT is what matters day to day; `feeds=all` covers trains and regional buses. |
| **SQLite with `sqlx`**, runtime-checked queries | One file, no server, fine for one user. `sqlx` compile-time checked queries would need a database at build time; runtime queries keep the build simple. |
| **FTS5 with the trigram tokenizer** | Trigrams match parts of words (`nuov` finds `nuova`) without a custom tokenizer. |
| **Normalize names and queries the same way** | Abbreviations and accents are handled once, in both directions. |
| **Full replacement in one transaction** | Searches never see a half-written index; no diffing logic to get wrong. |
| **Sync on startup if missing or older than 24 h, then nightly** | The service works right after the first start and stays fresh without load during the day. |
| **`503 index_not_ready` before the first sync** | An empty result would be misleading ("no stop called Porta Nuova"). |
| **Nearby computed locally** | No upstream call needed: coordinates are in the index. |

## Files

| File | Role |
|------|------|
| `migrations/0001_stops.sql` | tables `stops`, `stops_fts`, `sync_state` |
| `src/db.rs` | opening the database, migrations, in-memory database for tests |
| `src/stops/mod.rs` | `StopIndex` (storage), `StopInfo`, `Feeds`, line sorting, `display_code` |
| `src/stops/normalize.rs` | normalization and the abbreviation list |
| `src/stops/search.rs` | search ranking and nearby stops |
| `src/stops/sync.rs` | `sync_once` and the background schedule |
| `src/upstream/` | `all_stops()` with the `AllStops` query, 60 s timeout |
| `src/api/stops.rs` | `search`, `nearby`, `details` handlers |
| `src/config.rs` | `DATA_DIR`, `STOPS_SYNC_TIME`, `DEFAULT_FEEDS` |
| `tests/stops.rs`, `tests/fixtures/all_stops.json` | integration tests on 44 real stops |

## How the code works

### Database

`db::connect` creates `DATA_DIR` if needed, opens `gtt.db` in WAL mode (readers do not block the writer) and applies pending migrations:

```rust
static MIGRATOR: Migrator = sqlx::migrate!("./migrations");
```

`sqlx::migrate!` embeds the SQL files into the binary at compile time, so the Docker image needs no `migrations/` folder. Applied migrations are recorded in the database and never run twice.

`db::connect_in_memory` is for tests: a single connection that is never recycled, because every SQLite in-memory connection is a separate empty database.

The schema:

```sql
CREATE TABLE stops (
    id TEXT PRIMARY KEY,        -- "gtt:39"
    feed TEXT NOT NULL, code TEXT NOT NULL, name TEXT NOT NULL, description TEXT,
    lat REAL NOT NULL, lon REAL NOT NULL, mode TEXT,
    lines TEXT NOT NULL,        -- JSON: [{"id": "...", "name": "4", "mode": "TRAM"}]
    search_name TEXT NOT NULL,  -- normalized name, used for ranking
    search_text TEXT NOT NULL   -- normalized name + description, used for matching
);
CREATE INDEX stops_code ON stops (code COLLATE NOCASE);
CREATE INDEX stops_lat_lon ON stops (lat, lon);
CREATE VIRTUAL TABLE stops_fts USING fts5 (id UNINDEXED, text, tokenize = 'trigram');
CREATE TABLE sync_state (key TEXT PRIMARY KEY, at TEXT NOT NULL, count INTEGER NOT NULL);
```

### Normalization

`normalize` turns any name or query into lowercase ASCII words separated by single spaces:

```rust
pub fn normalize(text: &str) -> String {
    let folded: String = text.chars().flat_map(char::to_lowercase).map(fold).collect();
    let mut words: Vec<&str> = Vec::new();

    for mut rest in folded.split_whitespace() {
        while !rest.is_empty() {
            if let Some((abbreviation, expansion)) = ABBREVIATIONS
                .iter()
                .find(|(abbreviation, _)| starts_with_abbreviation(rest, abbreviation))
            {
                words.push(expansion);
                rest = &rest[abbreviation.len()..];
                continue;
            }
            let (word, tail) = rest.split_once('.').unwrap_or((rest, ""));
            if !word.is_empty() {
                words.push(canonical(word));
            }
            rest = tail;
        }
    }

    words.join(" ")
}
```

1. `fold` lowercases, removes accents (`à` → `a`), keeps letters, digits and dots, and turns everything else into spaces.
2. Each chunk is consumed from the left: if it starts with a known abbreviation, the expansion is emitted and the abbreviation removed; otherwise the text up to the next dot is a word. So `v.cavagnolo` gives `via cavagnolo`, and `g.cesare` gives `g cesare`.
3. `canonical` merges word variants: `santa`, `santo`, `sant` → `san`; `cso` → `corso`; `pza` → `piazza`.

The list in `ABBREVIATIONS` puts longer entries first (`v.le` before `v.`, `c.so` before `c.`). An entry ending with a dot also matches when glued to the next word; one without a dot must be the whole token or be followed by a dot, so `corso` is never mistaken for `c.` + `orso`.

Examples from the tests:

| Input | Output |
|-------|--------|
| `ASTI (C.SO FERRARIS/V. TORRICELLI)` | `asti corso ferraris via torricelli` |
| `TO-c.so G.Cesare-v.Cavagnolo` | `to corso g cesare via cavagnolo` |
| `Sant'Anna` | `san anna` |
| `MONTALDO M.VI'` | `montaldo mondovi` |

> **Rust concept: borrowing.** `words` is a `Vec<&str>`: slices pointing into `folded` or into the static abbreviation list, not new strings. The compiler checks that `folded` lives longer than `words`. Only `join` at the end allocates the result.

### Sync

`sync_once` downloads all stops and calls `StopIndex::replace_all`:

1. Each upstream stop becomes a `NewStop`: feed from the id prefix, display code (upstream code or the part after `:`), lines deduplicated by name and sorted naturally (`4`, `9`, `9B`, `15`, `M1S`), mode (from the stop or its first line), `search_name` and `search_text`.
2. In one transaction: delete everything, insert the stops in batches of 500 rows (one `INSERT` per batch, built with `QueryBuilder::push_values`), fill the full-text index from the table, update `sync_state`.

```rust
sqlx::query("INSERT INTO stops_fts (id, text) SELECT id, search_text FROM stops")
    .execute(&mut *tx)
    .await?;
```

If the upstream returns zero stops, `sync_once` fails with `SyncError::Empty` and the old index stays.

The background task (`sync::spawn`) loops forever:

- if the last sync is missing or older than 24 hours, sync now; otherwise sleep until the next `STOPS_SYNC_TIME`;
- on failure, retry after 5 minutes, 15 minutes, then every hour.

The next sync time is computed in local time and converted to UTC:

```rust
fn next_sync_at(now: DateTime<Utc>, sync_time: NaiveTime) -> DateTime<Utc> {
    let today = now.with_timezone(&TIME_ZONE).date_naive();
    [today, today + Days::new(1), today + Days::new(2)]
        .into_iter()
        .filter_map(|day| local_to_utc(day.and_time(sync_time)))
        .find(|at| *at > now)
        .unwrap_or(now + MAX_AGE)
}
```

`local_to_utc` handles the two daylight saving cases: a local time that does not exist (02:30 on the last Sunday of March) moves one hour later; one that exists twice (02:30 on the last Sunday of October) takes the first.

### Search

The SQL is built with `QueryBuilder`, so every user value is a bound parameter. A stop matches if its code or id equals the query, or if every word of the normalized query appears in its search text:

```rust
// Words are [a-z0-9] only, so quoting them is enough for FTS5.
sql.push(" AND id IN (SELECT id FROM stops_fts WHERE stops_fts MATCH ")
    .push_bind(fts_query)
    .push(")");
// ...
// Short words must start a word: "po" finds "porta", not "campo".
sql.push(" AND (' ' || search_text) LIKE ").push_bind(format!("% {word}%"));
```

Trigrams need at least 3 characters, so shorter words use `LIKE` on word starts. Then the ranking:

```sql illustrative
ORDER BY CASE
    WHEN code = ? COLLATE NOCASE OR id = ? THEN 0  -- exact code or id
    WHEN search_name = ? THEN 1                    -- exact name
    WHEN search_name LIKE ? THEN 2                 -- name starts with the query
    WHEN instr(search_name, ?) > 0 THEN 3          -- name contains the query
    ELSE 4 END,                                    -- description only
  feed <> 'gtt', length(name), name, length(code), code, id
```

So `porta nuova` returns 39, 40, 252, 253, 3492 (all named "PORTA NUOVA"; shorter codes first), then "METRO PORTA NUOVA", then stops that mention Porta Nuova only in their description (e.g. "ARSENALE", described as "STAZIONE PORTA NUOVA").

### Nearby

```rust
let delta_lat = radius_m / METERS_PER_DEGREE_LAT;
let delta_lon = radius_m / (METERS_PER_DEGREE_LAT * lat.to_radians().cos().max(0.01));
```

A rectangle around the point uses the `lat, lon` index to pick candidates; the exact great-circle distance (haversine) then filters them to the radius and sorts them. Each result carries `distance_m`.

### Endpoints

`search`, `nearby` and `details` validate their parameters, call `ensure_index_ready` (503 if `sync_state` is empty), then query the index. `feeds` is parsed by `Feeds::from_str`: `all`, or a comma-separated list of alphanumeric feed ids.

## Tests

- **Unit**: normalization (accents, abbreviations, glued and dotless forms, longest match first, no false matches inside words, saints), feed parsing, display code, natural sorting, line deduplication, distance, sync scheduling (before and after the sync time, both daylight saving days), retry delays.
- **Integration** (`tests/stops.rs`): the fake upstream serves `all_stops.json`, 44 real stops chosen around Porta Nuova plus stops with abbreviations, saints, metro and duplicate codes. Tests cover the exact ranking for `porta nuova`, codes (`39` first; with `feeds=all`, `gtt:39` then `4:39`), feed filter, abbreviations (`c.so vittorio` = `corso vittorio`), saints, partial and short words, nearby ordering and radius, details with sorted lines, metro code, 404, 503 before the first sync, a re-sync without duplicates, an empty upstream response keeping the index, and invalid parameters.

## Check it by hand

```bash
rm -rf data && cargo run
```

The log shows `stop index synced: 19422 stops in ...` within a few seconds.

```bash
curl -s 'http://localhost:8080/api/stops?q=porta%20nuova&limit=5'
```

```bash
curl -s 'http://localhost:8080/api/stops/nearby?lat=45.0712&lon=7.6853&radius=200'
```

Stop and start again: the log shows only `next stop sync at ...04:00`, without a new download.

## Pitfalls

- **sqlx 0.9 API.** `QueryBuilder` no longer has a lifetime parameter, and `query_as` refuses a `String` built at runtime unless it is wrapped in `AssertSqlSafe`, a marker that the SQL was checked by hand (here it only concatenates a constant column list).
- **Release builds on the Mac** fail: the linker produces a malformed `sqlx-macros` library ("mis-aligned LINKEDIT string pool"). Debug builds and Linux builds (CI, Docker) are fine.
- **Description-only matches.** Early on, stops matching only through their description ranked among exact names; rank 3 ("name contains the query") was added to push them down.
- **Known limit**: `pza castello` lists towns' "P.ZA CASTELLO" stops before Turin's "CASTELLO", whose name lacks "piazza".

## Later changes

- [Step 4](04-favorites.md) reads stop details from the index for each favorite.
- [Step 5](05-robustness.md): `/api/status` shows the last sync and stop count; sync failures are recorded in the upstream health.
- [Chapter 15](15-gtt-alerts.md): `StopIndex::lines` names GTT's alert lines; `next_sync_at` and `retry_delay` also schedule the nightly check of GTT's static data.
