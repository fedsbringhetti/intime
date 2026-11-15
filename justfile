# Development commands. A thin layer: each recipe runs a command or script that also
# works without `just` (CI runs the commands directly). See docs/adr/0002-justfile-for-development-commands.md.

set shell := ["bash", "-euo", "pipefail", "-c"]

# List the recipes.
default:
    @just --list

# Every check before a PR: Rust, web app, docs.
check: check-rust check-web check-docs

# Format, lint and test the Rust code.
check-rust:
    cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test

# Type check, test and build the web app.
check-web:
    cd web && npm run check && npm test && npm run build

# Check docs links, anchors and code excerpts.
check-docs:
    python3 .claude/skills/docs-sync/scripts/check_docs.py

# Run the service on :8080.
dev:
    cargo run

# Run the web app dev server on :5173 (needs `just dev` for /api).
web:
    cd web && npm run dev

# Build the linux/arm64 image; `just image --push` also publishes it.
image *args:
    scripts/build-image.sh {{args}}

# Empty Cargo's target directory, after showing its size and asking.
sweep:
    scripts/sweep.sh

# Regenerate src/gtt/proto.rs from tools/gtfs-rt-codegen/gtfs-realtime.proto.
codegen:
    cargo run --manifest-path tools/gtfs-rt-codegen/Cargo.toml
