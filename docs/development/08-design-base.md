# 8. Design base

> Phase 2, step 2 · PR [#13](https://github.com/fedsbringhetti/intime/pull/13)

## Goal

Turn the placeholder page into the frame of the real app: design tokens for light and dark, a router that maps each path to a screen, the bottom bar, and the pieces every screen will reuse (line badges, departure rows with times and delays, the bus loader, skeleton rows, icons). The six screens exist but are still empty. A page at `/prova-grafica`, in development builds only, shows every piece together, to compare with the mockups approved on 2026-11-02.

## Decisions

| Decision | Why |
|----------|-----|
| **Mockups first**, then code | The six screens, the dark theme, the offline and loading states were drawn on a design canvas and approved before writing code ([plan, section 6](../plan/phase-2.md#6-look-and-feel-decision-o)). The code follows them; the design page is how the two are compared. |
| **Tokens as CSS variables** in `app.css`, a second set under `prefers-color-scheme: dark` | Components never name a color, only a variable, so the dark theme is a list of values and nothing else. The phone's setting decides; there is no switch in the app. |
| **Accent dark teal**, badges by vehicle type | Blue is the bus color, so the accent had to differ from it. Badge colors keep white text readable (4.5:1 or more) in both themes. |
| **Our own router**, about 100 lines, no library | Six screens and one parameter (`/fermata/{stop}`). The logic is in pure functions (`router.ts`), tested without a browser; a small reactive module connects it to the address bar. |
| **Normal `<a href>` links everywhere**, intercepted by one listener | Links keep working as links (long press, open in a new tab, screen readers). A single listener on the document turns plain clicks on the app's own links into screen changes without reloading the page. |
| **History position in `history.state`** | A back link must go back inside the app. When a stop is opened directly (a shared link, a favorite on the Home Screen) there is nothing to go back to, and `history.back()` would leave the app: the link then goes to the screen of the highlighted tab instead. |
| **The bottom bar highlights where a stop came from** | As in the iPhone's own apps: a stop opened from Cerca keeps Cerca highlighted, and its back link says "Cerca". |
| **Delays rounded to the minute**: amber 1–4, red from 5, accent when early | What the mockups show. 20 seconds late is "in orario". The minus sign is U+2212, as wide as "+". |
| **Inline SVG icons** in one component | Fifteen icons, a few hundred bytes each; no library to download, and they take the text color. |
| **Design page loaded on demand, only in development** | It is a tool for us, not a screen. `import.meta.env.DEV` is `false` in a production build, so the import disappears and the file is not shipped (checked in `dist/`). |

## Files

| File | Role |
|------|------|
| `web/src/app.css` | design tokens (light and dark), base styles, `.card`, `.visually-hidden`, reduced motion |
| `web/src/App.svelte` | starts navigation; one screen per route; bottom bar |
| `web/src/lib/router.ts`, `router.test.ts` | routes, paths, highlighted tab, which clicks to intercept |
| `web/src/lib/navigation.svelte.ts` | current route as reactive state; `navigate`, `goBack`, `startNavigation` |
| `web/src/lib/tabs.ts` | the four bottom bar entries |
| `web/src/lib/lines.ts`, `lines.test.ts` | vehicle type from the upstream mode; delay label and color |
| `web/src/lib/components/Icon.svelte` | the stroke icons |
| `web/src/lib/components/LineBadge.svelte` | colored line badge, two sizes |
| `web/src/lib/components/DepartureRow.svelte` | one departure: badge, destination, time and delay, minutes; offline variant |
| `web/src/lib/components/BusLoader.svelte`, `SkeletonRows.svelte` | loading states |
| `web/src/lib/components/ScreenHeader.svelte`, `TabBar.svelte` | title with back link and actions; bottom bar |
| `web/src/screens/*.svelte` | the six screens (placeholders), not found, design page |
| `docs/plan/phase-2.md` | approved look and feel, navigation, step 2 |

## How the code works

### Tokens

Every color, size and radius is a variable:

```css
:root {
  color-scheme: light dark;

  /* Neutrals */
  --bg: #f2f2f7;
  --surface: #ffffff;
```

and the dark theme redefines the same names:

```css
@media (prefers-color-scheme: dark) {
  :root {
    --bg: #000000;
    --surface: #1c1c1e;
```

A component writes `background: var(--surface)` and is right in both themes. `color-scheme: light dark` also tells the browser to draw its own controls (scroll bars, form fields) in the matching theme.

> **Frontend concept: CSS custom properties.** A name starting with `--` is a variable; `var(--surface)` reads it. Unlike variables in Sass or Less, they live in the page while it runs: when the media query starts matching (the phone switches to dark at sunset), the values change and everything using them repaints, with no JavaScript.

### From an address to a screen

`matchRoute` reads the path and returns a *route*, a value that says which screen and with which parameter:

```ts
export type Route =
  | { name: 'home' }
  | { name: 'search'; query: string }
  /** `lines`: the line filter, `?linee=4,15`; empty for all lines. */
  | { name: 'stop'; stop: string; lines: string[] }
```

```ts
  const stop = /^\/fermata\/([^/]+)$/.exec(path)
  if (stop) {
    try {
      return { name: 'stop', stop: decodeURIComponent(stop[1]), lines: readLines(search) }
    } catch {
      // A malformed escape such as `%E0` cannot be a stop.
      return { name: 'not-found' }
    }
  }
```

> **Frontend concept: union types.** `Route` is like a Rust `enum` with data: each variant has a `name` and its own fields. After `if (route.name === 'stop')` TypeScript knows that `route.stop` exists, as a `match` arm does in Rust. `pathOf` does the opposite (a route to a path), and a test checks that the two agree for every route.

`navigation.svelte.ts` keeps the current route in reactive state:

```ts
export const navigation = $state<{ route: Route; tab: Tab; canGoBack: boolean }>({
  route: matchRoute(location.pathname, location.search, options),
  tab: 'home',
  canGoBack: position() > 0,
})
```

> **Frontend concept: runes.** `$state` is a Svelte 5 *rune*: the compiler turns the object into one that records who reads it, and re-renders those parts of the page when a field changes. Runes work in `.svelte` files and in modules named `.svelte.ts`, which is why this file has that extension.

`App.svelte` reads it and shows one screen:

```svelte
  {#if navigation.route.name === 'home'}
    <Home />
  {:else if navigation.route.name === 'search'}
    <Search query={navigation.route.query} />
  {:else if navigation.route.name === 'stop'}
    <Stop stop={navigation.route.stop} lines={navigation.route.lines} />
```

### Links without page loads

Screens use ordinary links (`<a href="/info">`). One listener on the whole document decides whether a click is for the app:

```ts
export function isInternalClick(click: LinkClick): boolean {
  return (
    click.button === 0 &&
    !click.modifier &&
    !click.defaultPrevented &&
    (click.target === '' || click.target === '_self') &&
    !click.download &&
    click.url.origin === click.origin &&
    !click.url.pathname.startsWith('/api/')
  )
}
```

If it is, the listener cancels the browser's page load and calls `navigate`, which adds a history entry with `history.pushState` and updates the state; the screen changes, the address bar shows the new path, and the browser's back button works. A middle click, Cmd+click or a link to another site are left to the browser.

Each entry the app adds records its position:

```ts
    history.pushState({ position: position() + 1 } satisfies EntryState, '', path)
```

The first screen has position 0, so a back link knows whether it can call `history.back()` or must go to its fallback:

```ts
export function goBack(fallback: string) {
  if (navigation.canGoBack) {
    history.back()
  } else {
    navigate(fallback, { replace: true })
  }
}
```

### The bottom bar and the header

`tabOf` picks the entry to highlight: the four main screens are their own entry, a stop or the info screen keep the previous one:

```ts
export function tabOf(route: Route, previous: Tab): Tab {
  switch (route.name) {
    case 'home':
    case 'search':
    case 'near':
    case 'favorites':
      return route.name
    default:
      return previous
  }
}
```

`ScreenHeader` uses the same entry for its back link ("‹ Cerca"), and takes the buttons on the right as a *snippet*:

```svelte
<ScreenHeader title="Partenze">
  {#snippet actions()}
    <a class="icon-button" href="/info" aria-label="Informazioni">
```

> **Frontend concept: snippets.** A snippet is a piece of markup passed to a component, like a closure that returns HTML. The header decides where to put it (`{@render actions?.()}`); the screen decides what it contains.

The bar is fixed at the bottom and grows by the iPhone's home indicator area, `env(safe-area-inset-bottom)`; the page leaves the same space free under its content. The viewport in `index.html` has `viewport-fit=cover`, without which those `env()` values are 0.

### Badges, times and delays

The badge color comes from the upstream mode (OpenTripPlanner uses `BUS`, `TRAM`, `SUBWAY`, `RAIL`, `FUNICULAR`):

```ts
export function vehicleType(mode: string | null | undefined): VehicleType {
  switch (mode?.toUpperCase()) {
```

and becomes a class, `<span class="badge {vehicleType(mode)} {size}">`, with one rule per type (`.tram { background: var(--tram) }`).

`describeDelay` turns seconds into the label and color of the mockups:

```ts
  const minutes = Math.round(seconds / 60)
  if (minutes === 0) return { label: 'in orario', level: 'on-time' }
  // U+2212 is the minus sign: as wide as "+", unlike the hyphen.
  if (minutes < 0) return { label: `−${-minutes}`, level: 'early' }
  return { label: `+${minutes}`, level: minutes >= VERY_LATE_MINUTES ? 'very-late' : 'late' }
```

`DepartureRow` puts them together: the real-time icon and the delay only when the data is real time, "orario programmato" otherwise, "in arrivo" at 0 minutes, and the clock time instead of minutes when the data is old (offline), since a countdown on old data would mislead.

### Loading and motion

`BusLoader` is an SVG bus moving along a dotted line drawn with `border-top: 3px dotted`; `SkeletonRows` are gray shapes with the layout of a departure row, pulsing. Both use CSS animations, and `app.css` stops every animation when the phone asks for reduced motion:

```css
@media (prefers-reduced-motion: reduce) {
```

### The design page

```svelte
  const designPage = import.meta.env.DEV ? () => import('./screens/DesignPreview.svelte') : null
```

Vite replaces `import.meta.env.DEV` with `true` in `npm run dev` and with `false` in `npm run build`. In the build the condition is constant, so the bundler drops the `import()` and never emits `DesignPreview.svelte`. `matchRoute` also returns `not-found` for `/prova-grafica` when `dev` is false.

## Tests

- **Vitest, `router.test.ts`** (11 tests): main screens, search query, stop by code and by full id (`gtt%3A39`), trailing slash, design page only in development, unknown and malformed paths; `pathOf` as the inverse of `matchRoute` for every route, including a query with `&` and spaces; `tabOf`; `isInternalClick` with each case left to the browser.
- **Vitest, `lines.test.ts`** (5 tests): vehicle types, case and missing modes; delay rounding (±29 s is on time), the 5-minute threshold, the minus sign.
- **Type check**: `svelte-check` with 0 errors and 0 warnings on 139 files.
- **By hand**, in the browser at phone size (375 × 812), light and dark: the design page against the mockups; Home → Info → back; the Cerca tab; `/fermata/39` opened directly, whose back link goes to Home; an unknown path; no page reloads (one navigation entry in the browser's timing data) and no console errors.

Results: 18 Vitest tests; 99 Rust tests, unchanged.

## Check it by hand

```bash
cargo run
```

```bash
cd web && npm run dev
```

Open `http://localhost:5173/prova-grafica` with the browser's device toolbar at phone size, then switch the system (or the developer tools' rendering settings) to dark. Tap the bottom bar, the info icon on Home and the back links.

The production build:

```bash
cd web && npm run build && grep -rl "Prova grafica" dist || echo "not shipped"
```

```
dist/index.html                  0.45 kB │ gzip:  0.28 kB
dist/assets/index-CbYExDsj.css   3.63 kB │ gzip:  1.42 kB
dist/assets/index-Dot62Fmw.js   47.49 kB │ gzip: 18.79 kB
```

About 6 KB more (compressed) than the skeleton: the router, the components and the six screens.

## Pitfalls

- **`Math.round(-2.5)` is `-2`.** JavaScript rounds halves towards positive infinity, so 150 seconds early is "−2", not "−3". Found by the first test run; the test now uses −170 seconds. For delays half a minute either way does not matter, so the rounding stayed.
- **Back link and the global listener.** The back link handles its own click (`goBack`). Its `onclick` calls `preventDefault()` first, and element handlers run before the listener on the document, which then sees `defaultPrevented` and leaves the click alone. Otherwise both would act and the app would move twice.
- **Zoom on focus in iOS.** Safari zooms the page when a field with text smaller than 16 px gets focus. Inputs use the body size, 17 px.
- **Emulated phone size in the browser** resets when the window changes size; screenshots for the comparison were taken right after setting it.

## Later changes

- [Chapter 17](17-web-push.md): the bottom bar has a fifth entry, "Notifiche" (`/notifiche`, a new `bell` icon, `Tab` and `tabOf` extended, `TabBar` in five columns).
- [Chapter 11](11-near-me.md): the last placeholder screen became the near-me screen; `Placeholder.svelte` was removed.
- [Chapter 9](09-search-and-stop.md): the stop route carries the line filter (`?linee=`); `DepartureRow` has a canceled variant; the design page shows notices and errors.
