# 10. Home and favorites

> Phase 2, step 4 · PR [#15](https://github.com/fedsbringhetti/intime/pull/15)

## Goal

Make favorites the heart of the app. A new endpoint returns every favorite with its next departures in one call; the Home screen shows them as cards, refreshed every 30 seconds. A star on the stop screen adds or removes a favorite, keeping the line filter on screen; the Preferiti screen renames them, chooses their lines, changes their order and removes them.

## Decisions

| Decision | Why |
|----------|-----|
| **One endpoint for the Home screen**, `GET /api/favorites/departures` | With five favorites the Home would make five calls every 30 seconds, each with its own latency on a phone network. One call, with the stops read in parallel on the server, is faster and simpler for the app. |
| **Parallel reads with `tokio::task::JoinSet`** | Already in Tokio, no new dependency. Reads go through the departures cache, which also joins concurrent requests for the same stop, so two favorites of one stop make one upstream call. |
| **One failing stop does not fail the response** | Each favorite carries its own `error` (`{code, message}`, the same codes as the API errors) and the response stays 200. The Home shows "Partenze non disponibili" on that card only. |
| **The star saves the lines on screen** | `/fermata/39?linee=4` starred gives a favorite with line 4 only. The star is full when a favorite has that stop with exactly those lines, so one stop can have "all lines" and "only 4" as two favorites; the database already allowed it. |
| **A Home card shows the filter** ("39 · solo 9") | Without a name, two favorites of one stop looked the same in the first test. |
| **Every change on Preferiti is saved at once**, then the list is read again | No "Salva" button to forget. Reading the list back keeps positions and names exactly as the server has them; the buttons wait while a change is saved, so two taps cannot cross. |
| **Remove with a second tap** within 4 seconds | One tap by mistake does nothing; no dialog box. The button turns into "Conferma rimozione" and goes back by itself. |
| **An empty name means the stop name** | The field shows the stop name as placeholder; clearing it sends `label: null`. |

## Files

| File | Role |
|------|------|
| `src/api/favorites.rs` | `departures` handler, its parameters and response |
| `src/api/stops.rs` | `check_range`, `DepartureView` and `departure_views` shared with favorites |
| `src/api/error.rs` | `ErrorBody` and `ApiError::into_parts`, for an error inside a larger response |
| `src/api/mod.rs` | the `/api/favorites/departures` route |
| `tests/favorites.rs` | three integration tests for the new endpoint |
| `web/src/lib/api.ts`, `api.test.ts` | favorites types; `POST`, `PATCH`, `DELETE` with JSON bodies |
| `web/src/lib/favorites.ts`, `favorites.test.ts` | stop ids, same lines, the favorite behind the star |
| `web/src/lib/components/FavoriteCard.svelte` | a favorite on the Home screen |
| `web/src/lib/components/DepartureRow.svelte` | compact variant for the cards |
| `web/src/screens/Home.svelte` | Home screen |
| `web/src/screens/Stop.svelte` | the star |
| `web/src/screens/Favorites.svelte` | Preferiti screen |
| `docs/user/api.md` | the new endpoint |
| `docs/plan/phase-2.md` | Home, Preferiti and star details |

## How the code works

### The endpoint

After checking the parameters (`limit` 1–10, default 3; `minutes` 1–120, default 60), the handler starts one task per favorite:

```rust
    let mut tasks = JoinSet::new();
    for (index, favorite) in favorites.iter().enumerate() {
        let service = state.departures.clone();
        let stop_id = favorite.stop_id.clone();
        tasks.spawn(async move { (index, service.get(&stop_id).await) });
    }
```

Tasks finish in any order, so each one returns its index and the result goes back in its place:

```rust
    while let Some(joined) = tasks.join_next().await {
        let (index, result) = joined.map_err(|err| {
            tracing::error!("departures task failed: {err}");
            ApiError::Internal
        })?;
        fetched[index] = Some(result);
    }
```

> **Rust concept: spawned tasks and `'static`.** `tasks.spawn` hands the future to Tokio, which may run it on another thread and after the handler has moved on; so it cannot borrow anything from the handler. That is why it gets its own copies: `state.departures.clone()` (an `Arc`, so only a counter goes up) and `favorite.stop_id.clone()`, moved in with `async move`. `join_next` waits for the next task to finish; it returns an error only if a task panicked.

Then each favorite is built with its own filter (its lines) and the same functions as `/api/stops/{stop}/departures`:

```rust
            Ok(fetched) => FavoriteDepartures {
                departures: departure_views(&fetched.data, &filter, now),
                stale: fetched.stale,
                error: None,
                view,
            },
            Err(err) => FavoriteDepartures {
                departures: Vec::new(),
                stale: false,
                error: Some(ApiError::from(err).into_parts().1),
                view,
            },
```

`into_parts` is the first half of what `ApiError` did to become an HTTP response: the status, the code and the message. The response uses all three; here only the `ErrorBody` (`code`, `message`) is kept, inside the favorite.

> **Rust concept: `#[serde(flatten)]`.** `FavoriteDepartures` contains a `FavoriteView` (which contains a `Favorite`), but in JSON all their fields sit side by side: `id`, `label`, `stop`, `missing`, `departures`, `error`. Flattening reuses the existing types without repeating their fields.

### Sending changes from the app

The API client has one function for every method; a body is sent as JSON:

```ts
    response = await fetch(path, {
      method,
      signal,
      headers,
      body: body === undefined ? undefined : JSON.stringify(body),
    })
```

A delete answers `204 No Content`, which has no body to parse:

```ts
  return response.status === 204 ? (undefined as T) : response.json()
```

### The star

```ts
export function matchingFavorite(
  favorites: Pick<Favorite, 'id' | 'stop_id' | 'lines'>[],
  stopId: string,
  lines: string[],
) {
  return favorites.find(
    (favorite) => favorite.stop_id === stopId && sameLines(favorite.lines, lines),
  )
}
```

`sameLines` ignores order and case, so `?linee=15,4` matches a favorite saved as `["4", "15"]`. The stop screen reads the favorites once; changing the filter changes `lines` and so the star, with no call. A tap adds or removes, then reads the list again.

### The Home screen

The same pattern as the stop screen: one effect, `startPolling` every 30 seconds while visible, the last good data kept with a note when a reload fails. Each card is a link to its stop with the favorite's lines:

```svelte
  let href = $derived(
    pathOf({ name: 'stop', stop: stopParamFromId(favorite.stop_id), lines: favorite.lines }),
  )
```

`stopParamFromId` works from the id alone (`gtt:39` → `39`), because a favorite whose stop disappeared from the index has no stop details.

### The Preferiti screen

Every control calls `save` with the change to make:

```svelte
  async function save(change: () => Promise<unknown>) {
    busy = true
    saveFailure = null
    try {
      await change()
      favorites = await getFavorites()
```

Moving up sends `position - 1`; the server shifts the other favorites. The name field saves on `change`, which a text field fires when it loses focus; Enter blurs it, which also closes the phone's keyboard.

## Tests

- **Rust, `tests/favorites.rs`** (3 new):
  - `departures_of_every_favorite_in_one_call`: a favorite with line 67 gets only line 67, three by default; a second favorite of the same stop gets all lines; `generated_at`, `stale`, `error`.
  - `one_failing_stop_does_not_hide_the_others`: the fake upstream answers 503 for stop 40; that favorite has `error.code = upstream_error` and no departures, stop 39 has its 5.
  - `favorites_departures_without_favorites_or_with_bad_parameters`: empty list; `limit` 0, 11, `x` and `minutes` 121 are 400.
  - The fake upstream answers stop queries with `body_partial_json` on the query variables and `with_priority(1)`, ahead of the catch-all answer used for the stop sync.
- **Vitest** (55 in all, 6 new): JSON bodies, `label: null`, an empty 204; `stopParamFromId`; `stopIdOf`, `sameLines`, `matchingFavorite`.
- **By hand**, at phone size against the real backend, light and dark, on an empty local database:
  - Home without favorites;
  - star on stop 39, then with the filter 9 (empty star, then full), then on the metro stop;
  - Home with three cards and their departures; a card opens `/fermata/39?linee=67` with the star full;
  - Preferiti: rename to "Casa", move the third up, choose line 67, "Conferma rimozione" back to "Rimuovi" after 4 seconds, removal with two taps;
  - no errors in the console after a fresh load. The test favorites were removed afterwards.

Results: 102 Rust tests (3 new), 55 Vitest tests.

## Check it by hand

```bash
cargo run
```

```bash
curl -s "http://localhost:8080/api/favorites/departures?limit=2"
```

```bash
cd web && npm run dev
```

Open `http://localhost:5173` at phone size, star a stop, come back to Home, then try Preferiti. Favorites are stored in `data/gtt.db`.

Build size: JavaScript 77.0 kB (28.5 kB compressed), CSS 14.7 kB (3.4 kB compressed).

## Pitfalls

- **Two cards that look the same.** Two favorites of one stop with different lines had the same title and code; the line filter is now shown next to the code.
- **A 204 in tests.** `new Response('', { status: 204 })` throws: a 204 must have no body at all, so the fake gives `null`.
- **An error left over from hot reload.** After `App.svelte` started passing `lines`, a hot reload ran the new stop screen with the old `App.svelte` for a moment (`lines` undefined). A fresh load, with a listener on `error`, showed nothing.
- **Long lines in `cargo fmt`.** The new test lines were longer than rustfmt's limit; `cargo fmt --check` caught it before the commit.

## Later changes

None yet.
