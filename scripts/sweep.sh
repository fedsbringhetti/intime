#!/usr/bin/env bash
# Empties Cargo's target directory after showing its size and asking for confirmation.
#
# Why: Cargo never deletes superseded builds. The hash in artifact names
# (libfoo-53d0b5347d5c3856.rlib) depends on the compiler, Cargo.lock, features and
# profile, not on the sources, so every toolchain change, lock update or new feature
# combination leaves a full set of libraries and test binaries behind. Left alone the
# folder grows without bound (another project reached 132 GB, 46 copies of one library).
#
# Choices to keep when editing this script:
# - The folder comes from `cargo metadata` (`target_directory`), not from guessing
#   CARGO_TARGET_DIR: it also honours `build.target-dir` in .cargo/config.toml, so it is
#   exactly what `cargo clean` deletes. Both run from the project root.
# - `cargo clean`, not `cargo sweep`: nothing to install, and emptying the folder is all
#   this command promises.
# - A hand-written prompt, default no, so it can show how much space it frees. Without a
#   terminal (EOF, e.g. in CI) `read` fails and the answer counts as no.
set -euo pipefail

cd "$(dirname "$0")/.."
root="$(pwd -P)"

target_dir="$(cargo metadata --format-version 1 --no-deps \
  | python3 -c 'import json, sys; print(json.load(sys.stdin)["target_directory"])')"

if [ ! -d "$target_dir" ]; then
  echo "$target_dir does not exist: nothing to sweep"
  exit 0
fi

target_dir="$(cd "$target_dir" && pwd -P)"
size="$(du -sh "$target_dir" | awk '{print $1}')"

echo "Target directory: $target_dir ($size)"
echo "It holds every build (debug and release), test binaries and generated docs."
echo "None of it is source and all of it can be rebuilt, but the next build starts cold."
case "$target_dir/" in
  "$root"/*) ;;
  *)
    echo "WARNING: this folder is outside the project ($root), so it may be shared:"
    echo "cargo clean would also delete the builds of every other project using it."
    ;;
esac

read -r -p "Delete it? [y/N] " answer || answer=""
case "$answer" in
  y | Y | yes | YES) ;;
  *)
    echo "nothing deleted"
    exit 0
    ;;
esac

cargo clean
echo "Deleted $target_dir ($size). The next cargo build, run or test rebuilds from scratch."
