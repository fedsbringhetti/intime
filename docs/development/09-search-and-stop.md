# 9. Search and stop

> Phase 2, step 3 · PR [#14](https://github.com/fedsbringhetti/intime/pull/14)

## Goal

The first two real screens. Cerca finds stops while you type and shows them with their lines; Fermata shows the departures of a stop, refreshed every 30 seconds while the screen is visible, with filters by line. Around them: a typed client for the API, Italian error messages, names turned from capitals into titles, and the states every screen meets (loading, nothing found, old data, server unreachable).

## Decisions

| Decision | Why |
|----------|-----|
| **Names shown as titles** ("Porta Nuova"), done in the web app | The upstream writes names in capitals. The server keeps them as they are (search and codes rely on them); only the display changes. Codes, line names, acronyms, Roman numerals and dotted abbreviations ("C.so", "P.za") are kept, checked on a sample of real names. |
| **Search after 250 ms of pause**, the query in the address | Each letter does not start a call; `?q=` lets the back button come back to the results. The address is *replaced*, so typing does not fill the history. |
| **An older search is aborted**, not ignored | With `AbortController` the previous call stops when a new one starts, so a slow answer for "por" can never replace the one for "porta". |
| **Matches in bold follow the server's rule** | Words of 3 letters or more match anywhere, shorter ones only at the start of a word, as in the search itself. So "porta n" bolds the "N" of "Nuova" but not every "n". |
| **Line filters in the app, several at once, in the address** (`?linee=4,15`) | The server already returns the next 60 minutes; filtering what is on screen is instant and costs no call. In the address the choice survives a reload, and the favorites step can reuse it. |
| **Refresh every 30 s, only while visible** | No calls while the phone is locked or another app is open; when the screen comes back it reloads at once if the data is older than 30 s. |
| **A failed refresh keeps the data on screen**, with a note | Departures from a minute ago are more useful than an error page. The note says how old they are and offers "Riprova". |
| **Canceled runs stay in the list**, struck through, "soppressa" | Hiding them would leave you waiting for a bus that is not coming. |
| **A 502–504 without the API's error format means "server unreachable"** | The API's own 502/504 carry a code (`upstream_error`, `upstream_timeout`). Without it, the answer came from a proxy in front of the server: Vite in development, Nginx once deployed. |

## Files

| File | Role |
|------|------|
| `web/src/lib/api.ts`, `api.test.ts` | API types (mirroring the Rust structs), calls, `ApiError`, Italian error messages, stop paths |
| `web/src/lib/format.ts`, `format.test.ts` | Rome time, names as titles, search matches |
| `web/src/lib/poll.ts`, `poll.test.ts` | reloading at an interval while the page is visible |
| `web/src/lib/departures.ts`, `departures.test.ts` | filter chips, filtering, toggling a line |
| `web/src/lib/router.ts`, `router.test.ts` | the stop route gets `lines` (`?linee=`) |
| `web/src/screens/Search.svelte` | search screen |
| `web/src/screens/Stop.svelte` | stop screen |
| `web/src/lib/components/Highlighted.svelte` | text with matches in bold |
| `web/src/lib/components/ErrorCard.svelte`, `Notice.svelte` | an error in place of content; a note above content |
| `web/src/lib/components/DepartureRow.svelte` | canceled variant |
| `web/src/App.svelte` | passes the line filter to the stop screen |
| `web/src/screens/DesignPreview.svelte` | canceled row, notes and error card |
| `docs/plan/phase-2.md` | search and stop details |

## How the code works

### The API client

Every call goes through one function, which turns the three ways a call can fail into one error type:

```ts
  try {
    response = await fetch(path, {
      method,
      signal,
      headers,
      body: body === undefined ? undefined : JSON.stringify(body),
    })
  } catch (error) {
    // An aborted call is not a failure: the caller asked for it and ignores it.
    if (signal?.aborted) throw error
    throw new ApiError(0, 'network', 'Server non raggiungibile')
  }
```

- `fetch` throws only when no answer arrived at all (no network, server down): that becomes the code `network`.
- An answer with an error status is read as the API's `{"error": {"code", "message"}}`; a body in another format (a proxy's HTML page) gives the code `http_error`.
- An aborted call is passed on as it is, so the screen can tell it apart and ignore it.

`errorText` turns the code into a sentence for the screen ("Server non raggiungibile: controlla la connessione.", "L'elenco delle fermate si sta scaricando: riprova tra un minuto.").

> **Frontend concept: `AbortController`.** `fetch` takes a `signal`; calling `abort()` on its controller stops the request and makes the promise fail with an `AbortError`. It is how a screen cancels work it no longer needs: an older search, the departures of a stop you just left.

### Names as titles

```ts
export function displayName(name: string): string {
  if (/\p{Ll}/u.test(name)) return name
```

A name that already has lowercase letters (other companies write "Torino Porta Nuova") is left alone. Otherwise each word is rewritten: small words in lowercase inside the name ("Strada del Drosso") but capitalized at its start or after a dash, bracket or slash ("Porta Nuova - M1 - Stazione", "Arsenale (Di Fronte)"); words with digits, acronyms and Roman numerals unchanged ("XXV Aprile", "Vittorio Emanuele II"). Running it on 65 real names showed one problem, abbreviations glued to a dot ("C.SO" became "C.So"), fixed by this rule:

```ts
    if (before.endsWith('.') && /^\p{L}{1,3}$/u.test(token) && /\p{L}\.$/u.test(before)) {
      return token.toLowerCase()
    }
```

> **Frontend concept: Unicode classes in regular expressions.** With the `u` flag, `\p{L}` is any letter in any alphabet and `\p{Ll}` any lowercase letter, so "LIBERTÀ" or "SANT'ANNA" are handled like plain ASCII. Without it, `[A-Z]` would split words at every accented letter.

### Search

The field and the address are kept in step in two directions:

```svelte
  let text = $state('')
  $effect(() => {
    text = query
  })
```

```svelte
  function onInput() {
    clearTimeout(pending)
    pending = setTimeout(() => showResultsFor(text), PAUSE_MS)
  }
```

Typing changes `text`; after the pause the address changes, and so does `query`. When `query` changes in any other way (the back button), the field follows it. The search itself depends only on `query`:

```svelte
  $effect(() => {
    const searched = query.trim()
    void attempt
```

and returns `() => controller.abort()`. Svelte calls that function before running the effect again, which is exactly when the previous search has become useless. `attempt` is read only so that "Riprova" (which increments it) runs the effect again.

> **Frontend concept: effects and their cleanup.** `$effect` runs after the screen is drawn and again whenever a reactive value it *read* changes. The function it returns cleans up the previous run. Values it only *writes* (`results`, `loading`) are not dependencies, which is what prevents an effect from triggering itself.

### The stop screen

The effect that loads data must depend on the stop only, not on the line filter:

```svelte
  const stopId = $derived(stop)
```

The `stop` prop comes from `navigation.route`, which is a new object every time the address changes, also when only `?linee=` changed. Read directly, it made the effect start over (and call the server again) at every tap on a filter; the browser's timing data showed 6 extra calls for 3 taps. A `$derived` value compares old and new and notifies only when the stop really changes.

Departures load at once and then through `startPolling`:

```ts
  const onVisibilityChange = () => {
    if (page.visibilityState !== 'visible') {
      clearTimeout(timer)
      return
    }
    const elapsed = now() - last
    if (elapsed >= intervalMs) run()
    else schedule(intervalMs - elapsed)
  }
```

`visibilitychange` fires when the page goes to the background (another app, the phone locked) and when it comes back. The page is passed in as a parameter (`document` by default), so the tests drive a fake one with fake timers.

Stop details (description, lines for the filters) are asked once; if that call fails, each reload asks again until they arrive. The state of that call is a plain variable inside the effect, not `$state`: the first load runs inside the effect, and reading reactive state there would make it a dependency.

Filtering happens on the data already received:

```ts
export function filterDepartures<T extends Pick<Departure, 'line'>>(
  departures: T[],
  chosen: string[],
): T[] {
```

and a tap on a chip only changes the address, with `replace` so that the back button leaves the stop instead of undoing filters.

## Tests

- **Vitest** (49 tests in all, 31 new):
  - `api.test.ts`: query and path encoding; API error code and message; a non-API error body; an unreachable server; an aborted call passed on; `stopParam`; `errorText`, including a proxy's 502.
  - `format.test.ts`: Rome time (winter, summer, after midnight); titles with small words, codes, acronyms, Roman numerals, dotted abbreviations, apostrophes, mixed-case names; matches with case and accents, short words only at word starts.
  - `poll.test.ts`: loads at each interval, stops in the background and catches up when visible, no double load when it comes back quickly.
  - `departures.test.ts`: chip order, case, filtering, toggling.
  - `router.test.ts`: the line filter in the address, and back.
- **Type check**: 0 errors, 0 warnings on 150 files.
- **By hand**, at phone size against the real backend, light and dark:
  - "porta n": 15 stops, the address follows after the pause, history unchanged;
  - a result opens `/fermata/39`; chips 9 and 67 give `?linee=9,67` and only those lines; filters cause no calls; back returns to the search with the text in the field;
  - one pair of calls when the stop opens, one departures call every 30 s;
  - `/fermata/999999`: "La fermata 999999 non esiste";
  - backend stopped: data kept with "Non riesco ad aggiornare", a new stop shows "Server non raggiungibile" and "Riprova"; backend back, "Riprova" loads details, filters and departures.

Results: 49 Vitest tests; 99 Rust tests, unchanged.

## Check it by hand

```bash
cargo run
```

```bash
cd web && npm run dev
```

Open `http://localhost:5173/cerca` at phone size, type a name, open a stop, tap the filters. To see the error states, stop `cargo run` while a stop is open and wait 30 seconds.

Build size: JavaScript 67.2 kB (25.8 kB compressed), CSS 10.0 kB (2.8 kB compressed); the design page is still left out.

## Pitfalls

- **A prop that changes without changing.** See the stop screen above: an effect reading a prop computed from a larger object re-runs whenever that object is replaced. Found by counting calls in the browser's timing data, not by any test.
- **Vite's hot reload repeats calls.** After saving a file, the page swaps the module and the effects run again, so call counts right after an edit look doubled. Counts were taken after a fresh load.
- **Upstream modes.** A replacement bus for the metro (`M1S`) is a `BUS` and gets the bus color, which is correct.
- **Short words in matches.** Ignoring words shorter than 2 letters left "porta n" with "Nuova" not bold; matching them everywhere would bold every "n". The server's own rule (start of a word) is the right one.

## Later changes

- [Chapter 10](10-home-and-favorites.md): the API client sends `POST`, `PATCH` and `DELETE` with JSON bodies; the stop screen has the favorite star; `DepartureRow` has a compact variant.
- [Chapter 15](15-gtt-alerts.md): the stop screen shows GTT's alerts in a banner; `DepartureRow` marks the departures of lines with alerts.
