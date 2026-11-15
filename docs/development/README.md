# Development

How the service was built, one chapter per development step, in order. Each chapter explains the goal, the decisions and why they were made, the code with short excerpts, the tests, how to check it by hand, and the pitfalls met along the way.

Read the chapters in order the first time: each builds on the previous ones. Code excerpts show the code as it is today; when a later step changed something, the chapter says so.

## Chapters

| # | Chapter | Phase | PR |
|---|---------|-------|----|
| 1 | [Project skeleton](01-skeleton.md): crate, configuration, logging, `/health`, CI | 1 | [#3](https://github.com/fedsbringhetti/intime/pull/3) |
| 2 | [Departures](02-departures.md): upstream client, times, cache, first real endpoint | 1 | [#4](https://github.com/fedsbringhetti/intime/pull/4) |
| 3 | [Stop index](03-stop-index.md): SQLite, nightly sync, name normalization, search, nearby | 1 | [#5](https://github.com/fedsbringhetti/intime/pull/5) |
| 4 | [Favorites](04-favorites.md): ordered favorites, JSON bodies, partial updates | 1 | [#6](https://github.com/fedsbringhetti/intime/pull/6) |
| 5 | [Robustness](05-robustness.md): retry, stale data, status, optional token | 1 | [#7](https://github.com/fedsbringhetti/intime/pull/7) |
| 6 | [Docker](06-docker.md): image, health check, compose, deployment files | 1 | [#8](https://github.com/fedsbringhetti/intime/pull/8) |
| 7 | [Web app skeleton](07-web-skeleton.md): Svelte + Vite project, embedding in the binary, Node stage, frontend CI | 2 | [#11](https://github.com/fedsbringhetti/intime/pull/11) |
| 8 | [Design base](08-design-base.md): tokens, light and dark, router, bottom bar, badges, departure rows, loaders | 2 | [#13](https://github.com/fedsbringhetti/intime/pull/13) |
| 9 | [Search and stop](09-search-and-stop.md): API client, names as titles, search while typing, live departures, line filters, error states | 2 | [#14](https://github.com/fedsbringhetti/intime/pull/14) |
| 10 | [Home and favorites](10-home-and-favorites.md): favorites with departures in one call, Home cards, the star, managing favorites | 2 | [#15](https://github.com/fedsbringhetti/intime/pull/15) |
| 11 | [Near me](11-near-me.md): the phone's position, nearby stops, a wider radius, denied permission | 2 | [#16](https://github.com/fedsbringhetti/intime/pull/16) |
| 12 | [Installable and offline](12-pwa-offline.md): InTime's icon and manifest, service worker, last data offline, new version prompt | 2 | [#17](https://github.com/fedsbringhetti/intime/pull/17) |
| 13 | [Info screen](13-info-screen.md): version, real-time health, stop list, installing InTime | 2 | [#18](https://github.com/fedsbringhetti/intime/pull/18) |
| 14 | [GTT predictions](14-gtt-predictions.md): GTFS Realtime without `protoc`, trip updates, merge rules, `source`, comparison with OpenTripPlanner, GTT rows on the info screen | 3 | [#21](https://github.com/fedsbringhetti/intime/pull/21) |
| 15 | [GTT alerts](15-gtt-alerts.md): GTT's static data with conditional requests, alerts linked by stop or by line, `/api/alerts`, the alerts banner, the static data row on the info screen | 3 | [#22](https://github.com/fedsbringhetti/intime/pull/22) |
| 16 | [Rules](16-rules.md): summaries and trackings with typed config, holidays and Easter, occurrences across daylight saving and midnight, CRUD API with pause and resume | 3 | [#23](https://github.com/fedsbringhetti/intime/pull/23) |
| 17 | [Web Push](17-web-push.md): VAPID keys, encrypted and signed pushes, subscriptions, the `Notifier` interface, the Notifiche screen, the service worker's `push` and `notificationclick` | 3 | [#24](https://github.com/fedsbringhetti/intime/pull/24) |
| 18 | [Scheduler and summaries](18-scheduler.md): the tick, `rule_runs` and idempotency, missed runs, the summary message, new alerts once, the notifications log, `POST /api/rules/{id}/test` | 3 | this PR |

The planning that came before the code is in [../plan/](../plan/phase-1.md); the overall picture is in [../architecture.md](../architecture.md).

## Workflow

### Branches and PRs

- `main` always works. Every step is a short-lived branch from `main` (`feat/...`, `fix/...`, `docs/...`) merged through a PR once CI passes.
- Everything in the repository is in English: code, comments, docs, branch names, commits, PR titles and descriptions.
- Releases are semantic version tags on `main` (`v0.1.0`, ...).

### Checks

```bash
cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test
```

```bash
cd web && npm run check && npm test && npm run build
```

```bash
python3 .claude/skills/docs-sync/scripts/check_docs.py
```

The second command checks the web app: types (`svelte-check` and `tsc`), unit tests (Vitest) and the build. The third checks the documentation: every relative link and anchor, and that the code excerpts in `docs/` still match the code (see [Keeping the docs in sync](#keeping-the-docs-in-sync)). All these checks run in GitHub Actions (`.github/workflows/ci.yml`) on every PR and on every push to `main`, in two parallel jobs: `rust` (docs, format, clippy, tests) and `frontend` (install, type check, tests, build).

### Working on the web app

```bash
cargo run
```

```bash
cd web && npm run dev
```

Open `http://localhost:5173`: Vite reloads the page on every save and forwards `/api` to the backend on 8080. To see the web app as the binary serves it, run `npm run build` and open `http://localhost:8080` (debug builds read `web/dist/` from disk, so no Rust rebuild is needed). Node 24 is the reference version (`web/.nvmrc`), the same as Docker and CI.

### Tests

- **Unit tests** sit at the bottom of each source file in a `#[cfg(test)] mod tests` block. They test pure functions: times, normalization, ranking helpers, config parsing, sync scheduling, token parsing.
- **Integration tests** live in `tests/`, one file per area (`departures.rs`, `gtt.rs`, `alerts.rs`, `gtt_static.rs`, `stops.rs`, `favorites.rs`, `rules.rs`, `push.rs`, `status.rs`, `auth.rs`, `health.rs`, `web.rs`). They build the real router with an in-memory database, a fixed clock and a fake upstream server (`wiremock`), then send HTTP requests to it. GTT's feeds are off unless a test sets `Options::gtt_url` to a fake GTT server; GTT's static data also needs `Options::gtt_gtfs_url`. Web Push is off unless a test sets `Options::vapid_key`; subscriptions then point to a fake push service (`wiremock`) allowed with `Options::push_endpoints` (an `EndpointPolicy` that accepts `127.0.0.1` over `http`; the default is the real list of push services). Background tasks are not started: tests call `TestApp::alerts` (`refresh`) and `TestApp::gtt_static` (`check`) themselves. `TestApp::subscriptions` is the subscription store and `TestApp::vapid_public_key` the key the browser would receive.
- **Shared helpers** are in `tests/common/mod.rs`: `test_app` / `test_app_with` (app with options), `synced_app` (app with the stop index already loaded; `synced_app_at` with the clock at a chosen time), `get`, `send`, `request`, `get_text` (for responses that are not JSON).
- **Fixtures** in `tests/fixtures/` are real upstream responses saved to files: `stop_39.json` (departures at Porta Nuova) and `all_stops.json` (44 real stops). `tests/fixtures/gtt/` holds GTT's trip updates (`trip_update.pb`, binary) and OpenTripPlanner's departures at stop 39 (`stop_39.json`), read at the same moment on 2026-11-04 at 10:46:30, so the merge can be tested on real data; GTT's alerts (`alerts.pb`, 119 alerts, read the same day at 16:03:58); and the real rows of GTT's `stops.txt` for 26 of the stops in `all_stops.json` plus 2 stations (`stops.txt`), zipped by the tests. `tests/fixtures/web/` is a tiny stand-in for the built web app, so the Rust tests never need Node.
- **Web app tests** are Vitest files next to the code they test (`web/src/lib/status.test.ts`), run with `npm test` in `web/`.

### Common changes

| Change | Where |
|--------|-------|
| New endpoint | handler in `src/api/`, route in `src/api/mod.rs` (it is behind the token automatically), error codes in `src/api/error.rs`, integration test in `tests/` |
| Database change | new `migrations/000N_description.sql`, applied at the next startup; never edit a migration that already ran |
| New upstream field | query in `src/upstream/queries.rs`, shape in `src/upstream/types.rs`, refresh fixtures if tests need it |
| New setting | `Config` in `src/config.rs` with a default and a test, then [../user/configuration.md](../user/configuration.md) and `.env.example` |
| Web app code | `web/src/`: components in `.svelte` files, logic in `.ts` modules under `lib/` with a `.test.ts` next to them |
| Node version | `web/.nvmrc` (CI reads it) and the `FROM node:` line in the `Dockerfile` |
| GTFS Realtime types | replace `tools/gtfs-rt-codegen/gtfs-realtime.proto` with the new official file, run `cargo run --manifest-path tools/gtfs-rt-codegen/Cargo.toml`, commit the regenerated `src/gtt/proto.rs` |

## Writing a chapter

Every development step adds a chapter here, in the same PR as the code. Name it `NN-short-name.md` and add it to the table above. Use this outline:

1. **Goal**: what the step delivers, in two or three sentences.
2. **Decisions**: each choice made and why, including alternatives rejected.
3. **Files**: table of files added or changed, with their role.
4. **How the code works**: walkthrough in the order a request or process runs, with short excerpts of the real code and the Rust concepts they use.
5. **Tests**: what is tested and how.
6. **Check it by hand**: commands to see it working.
7. **Pitfalls**: problems met and how they were solved.
8. **Later changes**: links to later chapters that changed this code.

When behavior visible to users changes, also update [../user/api.md](../user/api.md), [../user/configuration.md](../user/configuration.md) or [../architecture.md](../architecture.md).

When a chapter excerpt simplifies the real code, put `...` in the shortened line or mark the block as illustrative (```` ```rust illustrative ````), so the excerpt check skips it.

## Keeping the docs in sync

Three pieces work together:

| Piece | Where | What it does |
|-------|-------|--------------|
| **docs-sync skill** | `.claude/skills/docs-sync/SKILL.md` | The procedure Claude follows after a code change: find the affected documents, update the reference docs and the development chapter, verify, report. |
| **Hook** | `.claude/settings.json` | Before `git commit` and `gh pr create`, blocks when code changed on the branch but no documentation did, or when the docs check fails. It checks what will be committed (a temporary copy of the index; with `-a`, plus tracked edits) or, for a PR, the committed branch, so unrelated edits in the working folder never matter and docs never need moving aside. If the same command stages first (`git add ... && git commit`), it checks the working folder instead. Only real commands are matched, not their words inside a heredoc or a string. Add `[skip docs]` to the commit message for a change with no documentation impact. |
| **CI check** | `.github/workflows/ci.yml` | Runs the docs check on every PR. |

The skill's scripts can also be run by hand:

```bash
python3 .claude/skills/docs-sync/scripts/docs_impact.py
```

Lists the code changed on the branch, the documents to review for each change, the chapters that mention the changed files, and the next chapter number.

```bash
python3 .claude/skills/docs-sync/scripts/check_docs.py
```

Checks links, anchors and code excerpts; exits with 1 and a list of problems if anything is off.
