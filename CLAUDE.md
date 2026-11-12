# CLAUDE.md

Personal service for real-time GTT public transport departures in Turin: Rust backend, packaged as a Docker image for an Orange Pi 5 (ARM64); tested locally for now, first deployment planned with Phase 4, after the web app (Phase 2) and rules and notifications (Phase 3). Single user.

## Reference documents

- [PLAN.md](PLAN.md): project phases, cross-cutting decisions, workflow.
- [docs/README.md](docs/README.md): documentation index and layout.
- [docs/plan/](docs/plan/phase-1.md): detailed plan of each phase.
- [docs/architecture.md](docs/architecture.md), [docs/user/](docs/user/api.md), [docs/development/](docs/development/README.md): how it works, how to use it, how it was built.

Decisions already made are recorded in these files: follow them, and when a choice is not covered, ask instead of deciding alone.

## Language

- Everything in the repository is in English: file and directory names, document contents, code, identifiers, comments, branch names, commit messages, PR titles and descriptions, release notes.
- Conversation with the user is in Italian.

## How we work

- Plan first, then build. Do not write code, project skeletons or configuration files for a phase until the user explicitly says to start.
- Involve the user step by step: before a multi-step process (development step, release, deployment, git or GitHub operations) outline the steps, explain what each one does as it runs, and stop for their OK before changing the repository or anything outside it. They want to understand how things work, not only get them done. Group confirmations to keep turns few (see Token economy).
- Each phase has a detailed plan in `docs/plan/phase-N.md`, written and approved before the phase starts. If the plan changes during development, update the file.
- Every development step adds a chapter in `docs/development/` (`NN-short-name.md`, template in its README) in the same PR as the code: goal, decisions, files, how the code works with short excerpts, tests, how to check it, pitfalls. Changes visible to users also update `docs/user/`; design changes update `docs/architecture.md`. Use the `docs-sync` skill (`.claude/skills/docs-sync/`) for this: a hook blocks commits and PRs whose code changes have no documentation update.

## Multi-agent workflow

Capable models decide, efficient models build. Size every request first and use the lightest workflow that fits:

| Request | Workflow |
|---------|----------|
| Simple (small fix, one edit, docs) | done directly by the session, no planning phase, no subagents |
| Medium (one bounded change) | analyzed and done by the session, or by one Sonnet subagent for a well-bounded part |
| Complex (a development step) | plan (Opus, high) → Sonnet implements and tests → Opus reviews |
| Architectural or high risk | deeper plan (Opus, high) → delegated implementation → independent Opus review |

1. **Plan** (the session, Opus at high effort). Read the plan of the phase, the docs and the code involved; then write the plan the user approves under "How we work": goals and requirements, files to change and create, contracts and interfaces between parts, dependencies between tasks, acceptance criteria, tests, constraints, risks. Split it into tasks that can run independently. Nothing is delegated before the plan is approved.
2. **Implement and test** (Agent tool, `model: "sonnet"`, `effort: "medium"`). One subagent per task, with a short but complete brief: its part of the plan, the files it owns, the contracts it must respect, the tests to write and the checks to run. Subagents explore only what their task needs, follow the existing conventions, write or update tests, change nothing outside their task, never commit, push or touch git, and report any gap or contradiction in the plan instead of working around it. Run subagents in parallel only on independent tasks with no file in common; otherwise one after the other.
3. **Review** (Agent tool, `model: "opus"`, `effort: "medium"`, a fresh agent, not a fork). For development steps, architectural changes and complex features, never after small edits. It receives the approved plan and the diff, not the session's conclusions, and checks requirements, consistency with the architecture, correctness, errors and edge cases, security, tests and regressions. Its report ranks problems by severity with suggested fixes; the session (or a Sonnet subagent) fixes them before the commit table.

The session keeps the conversation with the user, decisions, git and GitHub, the full checks under "Commands" and the commit table. A subagent's report is not proof: check its diff before relying on it. If implementation reveals an architectural problem, stop the subagents and revise the plan with the user.

### Token economy

Use as few tokens as possible: the user asked for it. Every turn re-reads the whole conversation, so a long session is the main cost (step 4 of Phase 3: about 6.8M effective tokens, 61% in the session, mostly re-reading).

- **One session per phase** of a development step, each started fresh from a handoff file in `.claude/handoff/` (not in git): **plan** (Opus) ends with the approved plan, contracts and commit split in `phase-N-step-M.md`; **build** (subagents, checks, review, fixes) appends what changed and what was verified; **ship** (docs, commits, PR) works from both. A new session reads the handoff file and the files it names, not the old conversation.
- **Model per phase**, suggested at the start of each session (the user switches it): Opus for plan; Sonnet for ship and for routine build sessions.
- **Fewer turns**: one approval for plan and commit split; parallel subagents are checked together when all have finished; messages to the user only at decisions and at the end of a phase.
- **Small reads**: only the sections needed (grep, line ranges), never whole long documents to find one fact; a wide exploration goes to a subagent that returns a summary; no reading back a file just edited or verified.
- **Short briefs**: subagents get the path of the handoff file and their part, not a copy of it; the docs subagent gets the sections to change and the facts already verified.
- **Commits**: never move or delete files to get past a hook; fix the hook instead.
- Also: no Opus at high effort for routine work; no delegation of trivial work; no two agents reading the same files for the same purpose; no analysis repeated once done; never trade correctness, security or tests for tokens.

Models and effort: the session's model and effort are the user's choice (model picker in the app, `/model` and `/effort` in the terminal) and are never changed by Claude; planning at high effort needs the session set that way. Subagents get `model` and `effort` on each Agent call, so no `.claude/agents/` files are needed. If a call cannot set them, say so and do not pretend the step ran on that model.

## Git

- `main` always works. No `dev` branch.
- Every change in a short-lived branch from `main` (`feat/...`, `fix/...`, `docs/...`), merged back through a PR once CI passes.
- Releases as semantic version tags (`v0.1.0`, ...); pre-releases with `-rcN`.
- Commit, push, open PRs and tag only when asked.

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
