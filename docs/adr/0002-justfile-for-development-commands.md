# 0002. A justfile for development commands

Date: 2026-11-15 · Status: accepted

## Context

The development commands were spread over the docs: three long check commands (`CLAUDE.md`, `docs/development/README.md`), `cargo run` and `npm run dev`, the code generator (`cargo run --manifest-path tools/gtfs-rt-codegen/Cargo.toml`), and the scripts in `scripts/` (`build-image.sh`, `sweep.sh`). The repository had no `justfile` or `Makefile`. `just` 1.55 was already installed on the development Mac (Homebrew); CI and the Dockerfile do not use it.

## Decision

- A `justfile` at the root is the entry point for development commands: `just` lists them, `just check` runs every check before a PR.
- It stays thin: each recipe runs one command or one script that also works without `just`. Logic with prompts or several steps lives in `scripts/` (e.g. `sweep.sh` and its confirmation), not in recipes.
- CI keeps running the commands directly, so it needs nothing installed for this.

## Consequences

- One place to find and run commands; long commands are no longer copied by hand.
- `just` is one more tool on a development machine (`brew install just`); without it every recipe can still be run by hand, as `just --dry-run <recipe>` shows.
- A recipe and the docs or CI that repeat its command can drift apart; the docs show the recipe and say what it runs.
- Rejected: a `Makefile`, built to rebuild files from their timestamps, which here would need every target marked `.PHONY` and offers no recipe list or natural arguments; Cargo aliases, which can only run Cargo subcommands; an `xtask` crate, too heavy for running a few commands.
