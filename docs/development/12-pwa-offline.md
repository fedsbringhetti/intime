# 12. Installable and offline

> Phase 2, step 6 · PR [#17](https://github.com/fedsbringhetti/intime/pull/17)

## Goal

Make the web app an app: an icon and a name for the Home Screen ("InTime"), a full-screen window, and a service worker that keeps the app's files on the phone so it opens without network. Without network, the Home and stop screens show the last data they received, marked with their time; when a new version is deployed, the app offers it with "Aggiorna".

## Decisions

| Decision | Why |
|----------|-----|
| **Name "InTime" and a train icon**, chosen from mockups | "GTT" is the company's brand. The icon (a train in profile with a dish on its roof sending radio waves, on red) was drawn in three variants on the design canvas; variant A, white and solid, reads best at small size. |
| **Icons generated once and committed**, generator not a dependency | `@vite-pwa/assets-generator` brings a native image library (`sharp`) that Docker and CI would install on every build for files that change only with the icon. It runs with `npx` when needed; its config imports nothing so it works without being installed. |
| **`vite-plugin-pwa` in "inject manifest" mode** | We write the service worker (`src/sw.ts`) and the plugin only adds the list of files to cache. Phase 3 will add Web Push to the same file. |
| **API calls never cached by the service worker** | Departures are only useful fresh. The app decides what to show offline, from data it saved itself, and says how old it is. |
| **Last data in `localStorage`**: favorites and the last 10 stops | A few tens of KB, read synchronously, nothing the server does not already have. IndexedDB would be more powerful and more code for no gain at this size. |
| **"Offline" means the server cannot be reached** | No network, or a proxy answering 502–504 instead of the server (the API's own errors have a code). The banner says "Sei offline" when the phone reports no network, "Server non raggiungibile" otherwise. An error from the server itself keeps the "Non riesco ad aggiornare" note. |
| **Offline rows show the clock time and hide departures already gone** | "alle 08:03" stays true with old data; "6 min" would not. |
| **New versions with confirmation** ("Aggiorna"), not automatic | Chosen on 2026-11-02, after explaining that with a service worker a new version waits until the app is fully closed, which on an iPhone can take days. The new version is downloaded in the background; the user decides when to reload. |
| **Look for a new version when the app comes back**, at most once a minute | An installed app is often resumed rather than reopened, and resuming does not check by itself. |

## Files

| File | Role |
|------|------|
| `web/public/icon.svg` | the icon, source of every PNG |
| `web/public/*.png`, `favicon.ico` | generated icons (64, 192, 512, maskable 512, Apple 180, favicon) |
| `web/pwa-assets.config.ts` | how the icons are generated |
| `web/vite.config.ts` | the PWA plugin and the web manifest |
| `web/src/sw.ts`, `web/tsconfig.sw.json` | the service worker and its TypeScript settings |
| `web/index.html` | title, theme colors, iPhone tags, icons |
| `web/src/main.ts`, `web/src/lib/update.svelte.ts` | registering the service worker; new versions |
| `web/src/lib/components/UpdateBanner.svelte` | "Nuova versione disponibile · Aggiorna" |
| `web/src/lib/lastData.ts`, `lastData.test.ts` | last data in the browser's storage |
| `web/src/lib/components/OfflineBanner.svelte` | "Sei offline · dati delle HH:MM" |
| `web/src/lib/api.ts`, `poll.ts` | `isUnreachable`; reload when the network comes back |
| `web/src/screens/Home.svelte`, `Stop.svelte`, `web/src/lib/components/FavoriteCard.svelte` | save and show last data |
| `web/package.json`, `tsconfig.app.json`, `tsconfig.json` | new dependencies, the service worker's type check |
| `tests/web.rs`, `tests/fixtures/web/` | the service worker and manifest are served uncached |
| `PLAN.md`, `docs/plan/phase-2.md`, `docs/user/deploy.md` | name and icon, PWA details, what phones do after an update |

## How the code works

### The service worker

```ts
precacheAndRoute(self.__WB_MANIFEST)
cleanupOutdatedCaches()
```

At build time the plugin replaces `self.__WB_MANIFEST` with the list of the app's files and a hash for each (16 files, 119 KB). When the service worker installs, it downloads them all into a cache; from then on it answers requests for them from the cache.

```ts
registerRoute(new NavigationRoute(createHandlerBoundToURL('/index.html'), { denylist: [/^\/api\//] }))
```

Opening `/fermata/39` is a *navigation*. The server would answer it with `index.html` (the fallback of chapter 7); offline the service worker does the same, from its cache. `/api/` is left out, so API calls go to the network as if there were no service worker.

> **Frontend concept: service workers.** A service worker is a script the browser runs apart from the page, between the page and the network: it sees every request of the site and can answer from a cache. It is installed once, survives closing the page, and works only in a secure context (HTTPS or `localhost`). The browser checks the server for a new version of the script at every navigation; `sw.js` must therefore never be cached by HTTP, which the server already guarantees with `no-cache` (now covered by a test).

It runs in its own kind of context, with no `window` or `document`: hence its own `tsconfig.sw.json` with the `WebWorker` types, kept apart from the app's.

### New versions

A changed `sw.js` (any change to the app changes its file list) installs in the background and then *waits*: the old one keeps control while the app is open. `registerSW` from the plugin reports it:

```ts
  reloadWithUpdate = registerSW({
    onNeedRefresh() {
      update.available = true
    },
```

`update.available` shows the banner; "Aggiorna" calls `reloadWithUpdate(true)`, which tells the waiting worker to take over (the `SKIP_WAITING` message handled in `sw.ts`) and reloads the page. Since an installed app is usually resumed rather than reopened, the app also asks for an update check when it comes back to the foreground:

```ts
      document.addEventListener('visibilitychange', () => {
        if (document.visibilityState !== 'visible' || Date.now() - lastCheck < CHECK_EVERY_MS) return
        lastCheck = Date.now()
        registration.update()
      })
```

### Last data

```ts
  saveStop<T>(stop: string, data: T) {
    this.write(`stop:${stop}`, data)
    const recent = [stop, ...this.recentStops().filter((other) => other !== stop)]
    for (const forgotten of recent.slice(MAX_STOPS)) this.remove(`stop:${forgotten}`)
    this.write('stops', recent.slice(0, MAX_STOPS), false)
  }
```

Each save puts the stop first in a list of recent stops and forgets those past the tenth. Every access is in `try`/`catch`: storage can be full, disabled or unavailable in private mode, and the app must work anyway.

The screens save every good answer and use it when the server cannot be reached:

```svelte
          if (isUnreachable(error)) {
            // Without network: the data on screen, or else the last saved ones.
            data ??= lastData.loadFavorites<Data>()?.data ?? null
            if (data) {
              staleSince = data.generated_at
              failure = null
              return
            }
          }
```

With `staleSince` set, the banner appears and the rows switch to clock times. `startPolling` now also listens for the browser's `online` event, so the screen reloads as soon as the network is back.

## Tests

- **Rust, `tests/web.rs`**: `service_worker_and_manifest_are_never_cached` checks `text/javascript` and `application/manifest+json` with `no-cache`, against two new fixture files.
- **Vitest** (68 in all, 7 new): `lastData.test.ts` (save and load with time, the ten most recent stops, no storage, a broken entry, a full storage); `isUnreachable`; reloading on `online`.
- **Type check**: the app and, separately, the service worker.
- **By hand**, on the app served by the Rust binary on `localhost:8080` (built with `npm run build`), at phone size:
  - the service worker installs and caches 12 files; Home and stop 39 save their data;
  - with the server stopped, a reload opens the app from the cache: Home shows "Server non raggiungibile · Dati delle 15:38" and the favorite's departures as clock times; the stop screen, opened from the card, shows its saved departures; with `navigator.onLine` forced to false the banner says "Sei offline";
  - with the server back, the stop screen shows live data again;
  - a temporary change to `index.html` and a new build: on reload the new version waits and the banner appears, the page still the old one; "Aggiorna" reloads with the new version and the banner is gone.
- **Docker**: the image builds with the plugin (`precache 16 entries`); the container serves `sw.js`, the manifest and the icons.

Results: 103 Rust tests (1 new), 68 Vitest tests.

## Check it by hand

```bash
cd web && npm run build && cd ..
```

```bash
cargo run
```

Open `http://localhost:8080` (not the Vite server: the service worker is built only by `npm run build`), then in the browser's developer tools look at Application › Service workers and Cache storage. Stop `cargo run` and reload: the app still opens. Installing on a real phone needs HTTPS: Phase 4.

Build size: JavaScript 87.6 kB (32.1 kB compressed) plus `workbox-window` 5.7 kB (2.2 kB) and the service worker; 16 files precached, 119 KB.

## Pitfalls

- **The generator could not load its own config.** Run with `npx`, `@vite-pwa/assets-generator` was not installed in the project, so `import ... from '@vite-pwa/assets-generator/config'` failed. The config is now a plain object.
- **`npm audit`** reports moderate issues in `workbox-build`'s dependencies (`ejs`, `minimatch`, `glob`), used only at build time and not in the browser. The suggested fix is downgrading to `vite-plugin-pwa` 1.1, which does not support Vite 8.
- **A warning at build time**: "inlineDynamicImports option is deprecated", printed by the plugin when it builds the service worker with Vite 8; harmless.
- **The first visit is not controlled.** A service worker takes control from the next load; the offline test therefore reloads once before stopping the server.

## Later changes

- [Chapter 17](17-web-push.md): `sw.ts` also handles `push` and `notificationclick`, reading payloads with `lib/pushPayload.ts`, which `tsconfig.sw.json` now includes.
