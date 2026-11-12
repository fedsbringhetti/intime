# 7. Web app skeleton

> Phase 2, step 1 · PR [#11](https://github.com/fedsbringhetti/intime/pull/11)

## Goal

Lay the ground for the web app: a Svelte + Vite + TypeScript project in `web/`, compiled to static files that the Rust binary embeds and serves next to the API, with a fallback to `index.html` so the app can use normal paths. The Docker image builds the frontend in a Node stage, and CI checks it in its own job. The only screen is a placeholder page that reads `/api/status`, enough to prove the whole chain works.

## Decisions

| Decision | Why |
|----------|-----|
| **Plain Svelte, not SvelteKit** | The app has six screens and no server rendering. SvelteKit would bring file-based routing, `load` functions and adapters to configure for static output; a small router of our own (step 3) is easier to read and keeps the bundle small. |
| **Vite's official `svelte-ts` template**, then trimmed | A standard starting point (type check, TypeScript configs) without writing the build setup by hand. The demo components, images and icons were removed. |
| **Node 24 LTS** everywhere (`web/.nvmrc`, Docker, CI) | The plan said Node 22, which is only in maintenance until April 2027. Node 24 is supported until April 2028. `engines` accepts `>=24`, so a newer local Node works too. |
| **`rust-embed`** with `allow_missing` | One file to deploy, as before. Debug builds read from disk, so a frontend rebuild does not need a Rust rebuild. `allow_missing` keeps `cargo build` and `cargo test` working on a fresh clone without Node. |
| **Rust tests use a fixture folder**, not `web/dist/` | The tests stay independent of the frontend build, and the two CI jobs can run in parallel. The Docker build is where the two meet. |
| **Fallback to `index.html`, except `/api/*` and `/assets/*`** | Any other path is a screen of the app. An unknown API path or an old script name must get a real 404, not HTML that the client would try to read as JSON or JavaScript. |
| **`assets/` cached for a year, the rest `no-cache`** | Vite puts a content hash in those file names, so a name never changes content. `index.html` must be checked at every load to pick up a new version. No `ETag` for now: `index.html` is 450 bytes. |
| **Web app outside the token check** | The files hold no data; every API call the app makes goes through the check. Cloudflare Access protects everything at the edge once deployed (Phase 4). |
| **CI job `check` renamed to `rust`** | With two jobs, "check" no longer said which one. `main` has no branch protection that names it. |

## Files

| File | Role |
|------|------|
| `web/package.json`, `package-lock.json` | dependencies (all for development: only the build output reaches the browser) and the `dev`, `build`, `check`, `test` commands |
| `web/.nvmrc` | Node version, read by CI |
| `web/index.html` | the only HTML page: Italian, iPhone viewport, an empty `<div id="app">` |
| `web/vite.config.ts` | Svelte plugin, development proxy to the backend, Vitest |
| `web/src/main.ts` | entry point: mounts `App` |
| `web/src/App.svelte` | placeholder page |
| `web/src/app.css` | minimal global styles |
| `web/src/lib/status.ts`, `status.test.ts` | `/api/status` type and call, one-line description, tests |
| `web/tsconfig*.json`, `svelte.config.js` | TypeScript and Svelte settings from the template |
| `src/web.rs` | embedded files, fallback, cache headers |
| `src/api/mod.rs` | `AppState.web`, the fallback on the router |
| `src/main.rs` | passes the real files, warns when they are missing |
| `tests/web.rs`, `tests/fixtures/web/` | integration tests and the stand-in web app |
| `tests/common/mod.rs` | `FixtureWeb`, `Options.web`, `get_text` |
| `Dockerfile`, `.dockerignore` | Node stage; local `node_modules` and `dist` kept out |
| `.github/workflows/ci.yml` | `frontend` job |
| `.claude/skills/docs-sync/` | treats `web/` as code; checks `ts`, `svelte` and `html` excerpts |

## How the code works

### The frontend project

`npm run dev` starts Vite's development server; `npm run build` writes the finished files to `web/dist/`:

```
dist/index.html                  0.45 kB │ gzip:  0.28 kB
dist/assets/index-CVzC3CsM.css   0.41 kB │ gzip:  0.28 kB
dist/assets/index-BP-zhLR-.js   31.91 kB │ gzip: 12.56 kB
```

The part after the dash is a hash of the content: change one character and the name changes. That is what lets the backend tell the browser to keep those files for a year.

> **Frontend concept: Vite and Svelte.** Svelte is a *compiler*: `.svelte` components become plain JavaScript at build time, so no framework library has to be downloaded and started in the browser. Vite is the tool around it: in development it serves the files and reloads the page on every save (hot module replacement); for production it bundles and minifies them.

The page is a single `index.html` with an empty `<div id="app">`. `main.ts` fills it:

```ts
mount(App, { target: document.getElementById('app')! })
```

In development the interface is on port 5173 and the API on 8080 (`cargo run`). The proxy in `vite.config.ts` forwards API calls, so the code always calls `/api/...` as it will in production, where both come from the same origin:

```ts
  server: {
    // `npm run dev` serves the interface; API calls go to the backend started with `cargo run`.
    proxy: {
      '/api': 'http://localhost:8080',
    },
  },
```

### The placeholder page

`App.svelte` called `/api/status` once and showed the result. This is the code as it was in this step: since the design base ([chapter 8](08-design-base.md)) `App.svelte` chooses the screen, and the info screen ([chapter 13](13-info-screen.md)) shows the status in full.

```svelte illustrative
<script lang="ts">
  import { describeStatus, fetchStatus } from './lib/status'

  // Started once when the component is created; the `{#await}` block below follows it.
  const status = fetchStatus()
</script>

<main>
  <h1>GTT</h1>
  <p>La web app è in costruzione.</p>

  <p class="status">
    {#await status}
      Controllo il server…
    {:then value}
      {describeStatus(value)}
    {:catch error}
      Server non raggiungibile ({error.message})
    {/await}
  </p>
</main>
```

> **Frontend concept: a Svelte component.** One file holds the logic (`<script>`), the markup and the styles (`<style>`). Styles apply only to the component's own elements: Svelte adds a unique class to them. `{#await}` shows one of three states of a *Promise* (a value that arrives later): waiting, `{:then}` with the result, `{:catch}` with the error.

The response type mirrors `StatusResponse` in `src/api/status.rs` (today in `web/src/lib/api.ts`):

```ts
export interface Status {
  version: string
  time: string
  upstream: {
    ok: boolean | null
    last_success: string | null
    last_failure: string | null
    last_error: string | null
    consecutive_failures: number
  }
  stop_index: { synced_at: string; count: number } | null
}
```

> **Frontend concept: TypeScript types.** `npm run check` (`svelte-check`) fails if the code uses a field that is not in the type, as `rustc` would. Unlike Rust, TypeScript does not check what really arrives from the network at run time: `response.json()` is trusted to match. Keeping these types next to the Rust structs they mirror is how the two stay aligned.

### Embedding the build in the binary

```rust
#[derive(RustEmbed)]
#[folder = "web/dist"]
#[allow_missing = true]
struct Dist;
```

The derive generates a `Dist::get(path)` function. In release builds it looks the file up in data compiled into the binary; in debug builds it reads `web/dist/<path>` from disk. With `allow_missing`, a missing folder is an empty set of files instead of a compile error.

The router does not use `Dist` directly but a `WebAssets` value, so tests can swap in another folder:

```rust
#[derive(Clone, Copy)]
pub struct WebAssets {
    get: fn(&str) -> Option<EmbeddedFile>,
}
```

```rust
    pub fn from_embed<E: RustEmbed>() -> Self {
        Self { get: E::get }
    }
```

> **Rust concept: function pointers.** `fn(&str) -> Option<EmbeddedFile>` is the type of a plain function, here `Dist::get` or `FixtureWeb::get`. The `RustEmbed` trait could not be stored as `Box<dyn RustEmbed>`, because its functions have no `self` (they are "associated functions"), which makes the trait unusable as a trait object. A function pointer is the simplest way around it: it is `Copy`, needs no allocation, and `(self.get)(path)` calls it.

### The fallback

The router registers the web app as its fallback, the handler for every request no route matched:

```rust
    let web = state.web;
    Router::new()
        .route("/health", get(health::health))
        .merge(api)
        .with_state(state)
        .fallback_service(get(web::serve).with_state(web))
```

`get(...)` limits it to GET (and HEAD): a POST to `/fermata/39` gets 405. The token layer is on the API routes only (`route_layer`), so the web app is open.

The handler decides in this order:

```rust
    if path == "api" || path.starts_with("api/") {
        return StatusCode::NOT_FOUND.into_response();
    }
    let path = if path.is_empty() { INDEX } else { path };
    if let Some(file) = (assets.get)(path) {
        return file_response(path, file);
    }
    if path.starts_with(HASHED_DIR) {
        return StatusCode::NOT_FOUND.into_response();
    }

    // Any other path is a screen of the app (`/fermata/39`): the page loads and shows it.
    match (assets.get)(INDEX) {
        Some(file) => file_response(INDEX, file),
        None => StatusCode::NOT_FOUND.into_response(),
    }
```

`file_response` sets `Content-Type` from the file extension (the `mime-guess` feature of `rust-embed`) and `Cache-Control`:

```rust
fn cache_control(path: &str) -> &'static str {
    if path.starts_with(HASHED_DIR) {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    }
}
```

`no-cache` does not mean "do not store": the browser may keep the file but must ask the server before using it. `immutable` tells it not to ask at all.

At startup `main.rs` checks that the build is there:

```rust
    let web = WebAssets::dist();
    if !web.is_built() {
        tracing::warn!("web app not built: run `npm run build` in web/ (the API works anyway)");
    }
```

### Docker

```dockerfile
FROM --platform=$BUILDPLATFORM node:24-slim AS web
WORKDIR /web
COPY web/package.json web/package-lock.json ./
RUN npm ci
COPY web/ ./
RUN npm run build
```

and in the builder, before `cargo build`:

```dockerfile
COPY --from=web /web/dist web/dist
```

- **`--platform=$BUILDPLATFORM`**: the output is the same on every processor, so this stage runs natively on the machine doing the build, never under arm64 emulation. On an Apple Silicon Mac nothing changes; on an x86 machine it saves minutes.
- **Dependencies before sources**: `npm ci` is cached until `package.json` or `package-lock.json` change, like `cargo chef cook` for Rust.
- **`npm ci`** installs exactly what `package-lock.json` says and fails if it does not match `package.json`: the command for automated builds. `npm install` is for working locally, and may update the lockfile.

### CI

```yaml
  frontend:
    runs-on: ubuntu-latest
    defaults:
      run:
        working-directory: web
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-node@v7
        with:
          node-version-file: web/.nvmrc
          cache: npm
          cache-dependency-path: web/package-lock.json
```

then `npm ci`, `npm run check`, `npm test`, `npm run build`. It has no `needs:`, so it runs in parallel with the `rust` job.

## Tests

- **Rust, `tests/web.rs`**, against `tests/fixtures/web/` (an `index.html` and `assets/index-Ab12Cd34.js`):
  - `root_serves_the_page_without_caching`: `/` is `text/html`, `no-cache`.
  - `app_paths_serve_the_page`: `/fermata/39`, `/cerca?q=porta`, `/index.html`, `/health/extra` get the page.
  - `hashed_files_are_cached_for_a_year`: `text/javascript`, `immutable`.
  - `missing_hashed_files_and_api_paths_are_not_found`: `/assets/index-Old00000.js`, `/api`, `/api/nope`.
  - `only_get_is_allowed`: POST is 405.
  - `without_a_build_the_api_still_works`: an `allow_missing` folder that does not exist; `/` is 404, `/api/favorites` 200.
- **Rust, `tests/auth.rs`**: `web_app_stays_open` with a token configured.
- **Rust, unit**: `only_hashed_files_are_cached` in `src/web.rs`.
- The old `unknown_route_returns_not_found` in `tests/health.rs` was removed: an unknown path now correctly gets the page; the 404 cases are in `tests/web.rs`.
- **Frontend, Vitest** (`web/src/lib/status.test.ts`): `describeStatus` with and without a stop index.

Results: 99 Rust tests, 2 Vitest tests, `svelte-check` with 0 errors.

## Check it by hand

```bash
cd web && npm ci && npm run build && cd ..
```

```bash
cargo run
```

Open `http://localhost:8080/fermata/39`: the placeholder shows "Server 0.1.0 · 19.421 fermate". Headers:

```bash
curl -sI http://localhost:8080/ | grep -i cache-control
```

`no-cache`; a file under `/assets/` shows `public, max-age=31536000, immutable`; `/api/nope` and `/assets/index-Old00000.js` are 404.

For instant reloads while working on the interface, keep `cargo run` going and start `npm run dev` in `web/`, then open `http://localhost:5173`.

Docker (a temporary tag, so the local `0.1.0` and `latest` images are left alone):

```bash
docker buildx build --platform linux/arm64 --tag intime:web-skeleton --load .
```

```bash
docker run --rm -p 18080:8080 intime:web-skeleton
```

Measured on 2026-10-30: full build about 1 minute (the Node stage runs in parallel with `cargo chef cook`); rebuild after editing only `App.svelte` 7 seconds, with `npm ci` and the Rust dependencies from cache. Binary 15,344,976 bytes against 15,331,432 for `v0.1.0` (13.5 KB more); download size unchanged at 37.3 MB (`docker save | gzip`); 33 MB of memory after the first sync.

## Pitfalls

- **Italian number grouping.** `(7058).toLocaleString('it-IT')` gives `7058`, not `7.058`: Italian locale data groups digits only from five digits up. Found by the first Vitest run. `useGrouping: 'always'` makes it the same on every browser.
- **A local name hiding a helper.** In a test, `let request = Request::post(...)` then `request(&app, request)` fails with "expected function, found `Request<Body>`": the variable shadows the `request` helper imported from `common`. Renamed the variable.
- **Unknown paths are no longer 404.** With the fallback, `/nope` gets the page with 200, which broke a Phase 1 test. That is the intended behavior for an app with its own routes, so the test moved to `tests/web.rs` with the paths that must stay 404.
- **Checking JSON excerpts.** Extending the docs check to `json` blocks made it compare the API response examples in the docs with `web/package.json`. JSON is not checked.
- **`npm audit`** reports two low-severity issues in `deepmerge`, used by the Svelte Vite plugin at build time, with no fix available. It never reaches the browser bundle.
- **Building the image with `scripts/build-image.sh`** would tag it `0.1.0` and `latest`, replacing the local images of the real release. For trying a branch, build with a temporary tag.

## Later changes

- [Chapter 8](08-design-base.md): `App.svelte` now shows one screen per path; the status line moved to the info screen; `app.css` holds the design tokens.
- [Chapter 13](13-info-screen.md): the status line became the info screen; `status.ts` now describes the status for it, and the call moved to `api.ts`.
