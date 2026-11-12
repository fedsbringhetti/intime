# 11. Near me

> Phase 2, step 5 · PR [#16](https://github.com/fedsbringhetti/intime/pull/16)

## Goal

The last of the six screens: the stops near you, by distance, with their lines. The position comes from the browser when the screen opens, is used for that one search and is kept nowhere. When there is nothing within 500 m the screen looks within 1.5 km; when the permission is denied it explains how to give it back.

## Decisions

| Decision | Why |
|----------|-----|
| **The position is asked when the screen opens** | The only ways in are the "Vicino" tab and "Fermate vicino a me" on Home: opening the screen is the tap that decision Q asks for. The phone shows the permission request only the first time. |
| **Kept nowhere** | The position lives in one function call: not in `$state`, not in the address (`/vicino` stays as it is), not in the browser's storage. It goes only to our server in that one request, and the server does not log requests (checked: no request logging in `src/main.rs` or the router). |
| **500 m, then 1.5 km** | 500 m is a walk to the stop, as in the mockups. With nothing there, a second search at 1.5 km says so ("Nessuna fermata entro 500 m: ecco quelle entro 1,5 km") instead of an empty page. |
| **Precise position, at most 30 s old, within 10 s** | Precise because 100 m matter between two stops; a position up to 30 s old answers at once when you come back from a stop; after 10 s the screen says it did not arrive. |
| **"Posizione approssimativa" above ±200 m** | Indoors or without GPS the browser may answer with a rough position; the distances are then less reliable and the screen says so. |
| **Help for a denied permission**, not just an error | Once denied, the browser never asks again: the only way back is in the phone's settings. The screen gives the path for iPhone and Android and a link to the search by name. |
| **A failed "Aggiorna" keeps the list**, with a note | The same rule as the stop screen: results from a minute ago are more useful than an error page. |

## Files

| File | Role |
|------|------|
| `web/src/lib/location.ts`, `location.test.ts` | asks the browser for the position; Italian errors |
| `web/src/lib/format.ts`, `format.test.ts` | `formatDistance` |
| `web/src/lib/api.ts`, `api.test.ts` | `NearbyStop`, `nearbyStops` |
| `web/src/screens/Near.svelte` | the screen |
| `web/src/screens/Placeholder.svelte` | removed: no screen uses it any more |
| `docs/plan/phase-2.md` | near-me details |

## How the code works

### Asking for the position

The browser's API takes two callbacks; `locate` turns it into a promise:

```ts
    geolocation.getCurrentPosition(
      ({ coords }) =>
        resolve({ lat: coords.latitude, lon: coords.longitude, accuracy: coords.accuracy }),
      (error) => reject(toLocationError(error)),
      { enableHighAccuracy: true, timeout: TIMEOUT_MS, maximumAge: MAX_AGE_MS },
    )
```

and the browser's error codes become a `LocationError` with a reason (`denied`, `unavailable`, `timeout`, `unsupported`) and a sentence for the screen.

> **Frontend concept: from callbacks to promises.** Older browser APIs report their result by calling a function you pass in. Wrapping them in `new Promise((resolve, reject) => ...)` lets the rest of the code use `await` and `try`/`catch`, like `fetch`. The geolocation object is a parameter (the real one by default), so the tests pass a fake that answers at once.

The geolocation API works only in a *secure context*: HTTPS, or `localhost`. On the Mac the development server is fine; on a real phone it needs the HTTPS of Phase 4.

### The search

```svelte
      const position = await locate()
      if (current.signal.aborted) return
      phase = 'loading'
      let radius = NEAR_M
      let stops = await nearbyStops(position, radius, current.signal)
      if (stops.length === 0) {
        radius = WIDE_M
        stops = await nearbyStops(position, radius, current.signal)
      }
```

`position` is a local variable: when the function ends it is gone. What stays on screen is the result, with the radius used and the accuracy. "Aggiorna" calls the same function; the previous call is aborted first, so two taps cannot mix their results.

### Distances

```ts
export function formatDistance(meters: number): string {
  const rounded = Math.round(meters / 10) * 10
  if (rounded < 1000) return `${rounded} m`
  return `${kilometers.format(rounded / 1000)} km`
}
```

Rounded to 10 m, because a phone's position is rarely more precise than that; kilometers with the Italian decimal comma ("1,2 km") through `Intl.NumberFormat('it-IT')`.

## Tests

- **Vitest** (61 in all, 6 new): `locate` with a fake geolocation (position, the options passed, each error code, no geolocation at all); `formatDistance` (rounding, kilometers, the comma); the nearby request URL.
- **By hand**, at phone size against the real backend, light and dark, with `navigator.geolocation` replaced by a fake in the page (the browser pane has no GPS):
  - in front of Porta Nuova: 20 stops, the nearest at 10 m; the address stays `/vicino`;
  - permission denied: the help with the paths for iPhone and Android;
  - a point with nothing within 500 m (45.02, 7.55): the 1.5 km results with the note; a point with nothing within 1.5 km: the empty message;
  - accuracy ±900 m: "Posizione approssimativa";
  - a timeout after results: the note above the old list; a timeout with no results: the error card with "Riprova";
  - a result opens its stop; back returns to the list; no errors in the console.

Results: 61 Vitest tests; 102 Rust tests, unchanged.

## Check it by hand

```bash
cargo run
```

```bash
cd web && npm run dev
```

Open `http://localhost:5173/vicino` and allow the position. In Chrome's developer tools, the Sensors panel can set any position, to try the 1.5 km search or a denied permission.

Build size: JavaScript 81.9 kB (30.1 kB compressed).

## Pitfalls

- **A default parameter and `undefined`.** `locate(undefined)` used the default (the real geolocation) instead of meaning "no geolocation"; a missing API is now `null`.
- **`GeolocationPositionError` does not exist in Node**, where the tests run: its codes (1 denied, 2 unavailable, 3 timeout) are written out in `location.ts`.
- **A screenshot between two states.** One capture showed "Aggiorno la posizione…" after the error had already appeared; the page state read right after was correct. Screenshots are taken a moment after the page settles.
- **Coming back from a stop asks again.** The screen starts a new search each time it opens; with positions up to 30 s old accepted, the phone answers at once.

## Later changes

None yet.
