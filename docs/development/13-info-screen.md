# 13. Info screen

> Phase 2, step 7 · PR [#18](https://github.com/fedsbringhetti/intime/pull/18)

## Goal

The last screen of Phase 2: the version, whether the real-time data works, how fresh the list of stops is, and how to install InTime on the phone. It is the first place to look when something seems wrong, from the phone, without `curl`.

## Decisions

| Decision | Why |
|----------|-----|
| **Everything from `GET /api/status`** | The endpoint already has what the screen needs (version, upstream health, stop index). No backend change. |
| **The status call moves to `api.ts`** | It now gets the same errors and Italian messages as the other screens. `status.ts` keeps only what the screen says about the status. |
| **Sentences, not raw values** | "Funzionano · ultimo contatto 15:40", "Non rispondono · ultimo errore ieri alle 23:10", "19.414 fermate · aggiornato oggi alle 11:16". The upstream's own error message is in English and technical: it is shown only when the data does not work, small, as "Dettaglio tecnico". |
| **"oggi", "ieri" or the date, in Turin time** | A bare time is ambiguous for the nightly stop sync. The day is computed in `Europe/Rome`, so 00:30 in Turin is "oggi" even when it is still yesterday in UTC. In the tight real-time row, today is just the time, as in the mockup. |
| **No install instructions inside the installed app** | Opened from the Home Screen icon, the app runs in its own window (`display-mode: standalone`); the screen then says "InTime è installata su questo telefono". `navigator.standalone` covers older iPhones. |
| **Loaded once when the screen opens** | It is a screen to glance at; opening it again reloads it. |

## Files

| File | Role |
|------|------|
| `web/src/screens/Info.svelte` | the screen |
| `web/src/lib/status.ts`, `status.test.ts` | what to say about the upstream and the stop index |
| `web/src/lib/format.ts`, `format.test.ts` | `formatDayTime` |
| `web/src/lib/api.ts` | `Status` and `getStatus`, moved here |
| `docs/development/07-web-skeleton.md` | the old status line shown as it was |

## How the code works

### What the screen says

```ts
function describeHealth(health: Status['upstream'], now?: Date): Line {
  const { ok, last_success, last_failure } = health
  if (ok === null) return { value: 'Nessun contatto ancora', detail: null, tone: 'neutral' }
```

`describeUpstream` passes `status.upstream` to `describeHealth` (since chapter 14 the same function also describes GTT's feed). `ok` is `null` before the server has called the upstream once (just started), `true` after a success and `false` after a failure. Each case becomes a short value, a detail under it and a tone, which picks the color (green, red or gray).

### Days in Turin

```ts
export function formatDayTime(iso: string, now: Date = new Date(), short = false): string {
  const date = new Date(iso)
  const yesterday = new Date(now.getTime() - 24 * 60 * 60 * 1000)
  const time = formatTime(iso)
  if (romeDay(date) === romeDay(now)) return short ? time : `oggi alle ${time}`
```

`romeDay` writes the calendar day as Turin sees it (`toLocaleDateString` with `timeZone: 'Europe/Rome'` in the `en-CA` format, `2026-11-02`, which compares as plain text). Comparing days, not differences in hours, keeps "ieri" right at 00:30 and across the change to and from summer time.

### Installed or not

```svelte
  const installed =
    matchMedia('(display-mode: standalone)').matches ||
    (navigator as Navigator & { standalone?: boolean }).standalone === true
```

> **Frontend concept: `display-mode`.** The manifest's `display: standalone` (chapter 12) is also a CSS media feature: a page can ask, with `matchMedia`, whether it is running in the browser or as an installed app. The same works in CSS (`@media (display-mode: standalone)`), for example to hide a "install" hint.

## Tests

- **Vitest** (73 in all, 7 new; the two old tests of `describeStatus` were replaced): `describeUpstream` working, failing, never called; `describeStopIndex` with Italian grouping and missing list; `formatDayTime` today, yesterday, older, the Turin day around midnight, the short form.
- **By hand**, at phone size, light and dark: the real status (version 0.1.0, real-time working, 19,414 stops updated today); with `fetch` replaced in the page, a failing upstream with "ieri" and the technical detail; with `matchMedia` replaced, the installed state; no console errors.

Results: 73 Vitest tests; 103 Rust tests, unchanged.

## Check it by hand

```bash
cargo run
```

```bash
cd web && npm run dev
```

Open `http://localhost:5173/info`. To see the failing state, stop the network of the machine for a minute while a stop screen is open (the upstream calls fail), then open Info.

## Pitfalls

- **A row too narrow.** "ultimo contatto oggi alle 15:40" wrapped twice next to "Orari in tempo reale" on a 375 px screen; the mockup had just the time, so the short form leaves out "oggi alle".
- **An excerpt of code that no longer exists.** Chapter 7 showed the old status line as current code, and the docs check stopped it; it is now marked as the code of that step.

## Later changes

- [Chapter 14](14-gtt-predictions.md): rows for GTT's predictions and for today's difference between GTT and OpenTripPlanner; `describeHealth` shared by the two real-time rows.
- [Chapter 15](15-gtt-alerts.md): the "Dati statici GTT" row (`describeStaticData`).
