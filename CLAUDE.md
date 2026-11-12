# CLAUDE.md

Personal service for real-time GTT public transport departures in Turin: Rust backend, packaged as a Docker image for an Orange Pi 5 (ARM64); tested locally for now, first deployment planned with Phase 4, after the web app (Phase 2) and rules and notifications (Phase 3). Single user.

## Reference documents

- [PLAN.md](PLAN.md): project phases, cross-cutting decisions, workflow.
- [docs/README.md](docs/README.md): documentation index and layout.
- [docs/plan/](docs/plan/phase-1.md): detailed plan of each phase.
- [docs/plan/steps/](docs/plan/steps/README.md): plan and final report of each medium or large item.
- [docs/backlog.md](docs/backlog.md): work not done yet, with evidence. [docs/adr/](docs/adr/README.md): structural decisions.
- [docs/architecture.md](docs/architecture.md), [docs/user/](docs/user/api.md), [docs/development/](docs/development/README.md): how it works, how to use it, how it was built.

Decisions already made are recorded in these files: follow them, and when a choice is not covered, ask instead of deciding alone.

## Language

- Everything in the repository is in English: file and directory names, document contents, code, identifiers, comments, branch names, commit messages, PR titles and descriptions, release notes.
- Conversation with the user is in Italian, in short messages.

## How we work

Applies to every piece of work: a backlog item, a development step, a fix.

### Before touching anything

- Read the existing code and docs. Measure what is taken as true (run it, grep it) instead of trusting descriptions: plans and issues age.
- Size each item, not the whole PR. **Small** (behavior already clear, few files): done directly by the session. **Medium or large** (several modules, shared contracts, concurrency, persistence, security): three phases, plan → implementation → review.
- For a medium or large item the plan goes in the repository before any code: `docs/plan/steps/<name>.md` (e.g. `phase-3-step-5.md`), at most about 150 lines: verified facts with the file where each was found, decisions and why, pitfalls, done criteria. Questions that cannot be decided alone go to the user all at once, with concrete examples.
- Each phase still has its plan in `docs/plan/phase-N.md`, approved before the phase starts and updated when it changes; no code for a phase until the user says to start. New screens are mocked up on the design canvas and approved before any UI code.

### During

- Implementation agents get an explicit cheap model (`model: "sonnet"`, `effort: "medium"`), otherwise they inherit the session's. One agent per task, briefed with the path of the plan file and its part; in parallel only on tasks with no file in common. Agents never touch git and report gaps or contradictions instead of working around them.
- **One review**, by a separate agent on Opus (`model: "opus"`, `effort: "medium"`), limited to the diff and the plan; it reports problems and never edits code. A second round only if the first finds a defect that can lose data, crash the process or let through what the change must stop, and only on the fixes. Never a third without asking.
- A defect this change did not introduce becomes an item in [docs/backlog.md](docs/backlog.md) with its evidence, not a commit, unless the user says otherwise for that defect.
- No report is proof, neither the implementer's nor the reviewer's: verify independently with build, lint and tests (Commands) and, where it makes sense, with the running app. A new migration is tried on a copy of the data folder (`DATA_DIR`), never on `data/gtt.db`.

### Documentation

- Docs change with the code, in the same commit: `docs/user/` for visible behavior, `docs/architecture.md` for design, the step's chapter in `docs/development/` (template in its README). Use the `docs-sync` skill (`.claude/skills/docs-sync/`): a hook blocks commits whose code changes have no documentation update.
- Structural or hard-to-reverse decisions go in an ADR in [docs/adr/](docs/adr/README.md), append-only: a superseded decision gets a new ADR, the old one is never edited. Prefer a test that checks the docs (`check_docs.py`) to a reminder.
- At the end, append to the item's plan file a report of at most one page: what was built, where the plan was wrong, questions for the next work.

### History and publishing

- Creating a branch needs no permission: `feat/...`, `fix/...`, `docs/...` from an up-to-date `main`, one PR per branch, never stacked on another. `main` always works.
- When the work is ready, propose the commits in a table (type and scope, subject, what it covers) and stop. Commit only after approval. Push only after a separate confirmation: approving one push does not cover the next.
- The user opens the PRs; when asked, use the repository's PR template if there is one. Group work in PRs by area, so CI does not run for a few lines.
- Conventional Commits (`feat:`, `fix:`, `docs:`, `ci:`, `chore:`, ...), with no attribution trailers (no `Co-Authored-By`). Releases are semantic version tags (`v0.1.0`, `-rcN` for pre-releases), only when asked.

## Token economy

Every token counts: keep messages short. Every turn re-reads the whole conversation, so a long session is the main cost (Phase 3, step 4: about 6.8M effective tokens, 61% in the session, mostly re-reading).

- **One session per phase** of a medium or large item, each started fresh from the item's plan file: the plan session (Opus) ends with the approved plan in `docs/plan/steps/`; the implementation session (Sonnet suggested) works from it and adds what changed and what was verified. A new session reads the plan file and the files it names, not the old conversation.
- **Model per phase**, suggested at the start of each session; the session's model and effort are the user's choice (model picker in the app, `/model` and `/effort` in the terminal) and are never changed by Claude. Subagents get `model` and `effort` on each Agent call; if a call cannot set them, say so.
- **Fewer turns**: one approval for the plan; parallel agents are checked together when all have finished; messages to the user only at decisions and at the end of a phase.
- **Small reads**: only the sections needed (grep, line ranges), never whole long documents to find one fact; a wide exploration goes to an agent that returns a summary; no reading back a file just edited or verified.
- **Commits**: never move or delete files to get past a hook; fix the hook instead.
- No Opus at high effort for routine work; no delegation of trivial work; no two agents reading the same files for the same purpose; no analysis repeated once done; never trade correctness, security or tests for tokens.

## Stack

- Rust: `axum`, `tokio`, `reqwest` (rustls), `sqlx` + SQLite (FTS5), `serde`, `chrono` + `chrono-tz`, `moka`, `tracing`, `thiserror`, `rust-embed`, `prost` (GTFS Realtime).
- Web app in `web/`: Svelte 5 (plain, no SvelteKit), Vite, TypeScript, Vitest; Node 24 LTS (`web/.nvmrc`). Embedded in the binary from `web/dist/`.
- Multi-stage Docker for `linux/arm64`; public images on Docker Hub as `federicosbringhetti/intime`.
- Deployment follows the home server conventions in [docs/user/deploy.md](docs/user/deploy.md): host port 8100, bind mount `./data`, secrets only in `.env`.

## Data source

OpenTripPlanner GraphQL API at `https://plan.muoversiatorino.it/otp/routers/mato/index/graphql`. It is unofficial and may change without notice. Things to remember:

- Query a stop with `stop(id: "gtt:<code>")`, which also returns departures: no second query is needed.
- `stops(name: ...)` does not work (always returns an empty list): name search uses the local index.
- Departure time = `serviceDay` (Unix) + seconds, then converted to `Europe/Rome`. Seconds can exceed 86,400. Never use "today + seconds".
- Stop codes repeat across feeds: the key is always the full `gtfsId` (`gtt:39`).
- GTT metro stops have `code: null`: derive the code from the id.
- Lines are not numbers (`9B`, `M1S`, `68+`): always treat them as strings.
- Pass parameters as GraphQL `variables`, never concatenated into the query.
- The position of a stop in a run is `stopSequence` (this schema has no `stopPosition`); it matches GTT's `stop_sequence`.

GTT's own real-time feeds (`GTT_REALTIME_URL`, from Phase 3) are GTFS Realtime over HTTP GET:

- Decoded with the types in `src/gtt/proto.rs`, generated by `tools/gtfs-rt-codegen` (no `protoc` needed). Never edit that file by hand.
- Urban runs only; a run is GTT's trip id (ours without `gtt:`) plus its `start_date`. Stops are given by `stop_sequence` only, with arrivals only, and each run uses either absolute times or delays.
- The header timestamp is the time of the request: freshness comes from each run's own timestamp.
- Alerts (`alerts.aspx`, read every 5 minutes) name stops by GTT's `stop_id`, not our code: `gtt_stops` (from `stops.txt` in `GTT_GTFS_URL`) maps it. A stop alert lists one stop and every line serving it, so alerts with stops are linked by stop, never by line. GTT fills the deprecated `active_period`, not `communication_period`.
- The GTFS static zip answers `If-Modified-Since` with `304`: check it with the stored `Last-Modified`, never download it blindly.

## Commands

Before every PR:

```bash
cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test
```

```bash
cd web && npm run check && npm test && npm run build
```

```bash
python3 .claude/skills/docs-sync/scripts/check_docs.py
```
