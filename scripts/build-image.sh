#!/usr/bin/env bash
# Builds the linux/arm64 image for the Orange Pi, tagged with the Cargo version and `latest`.
# With --push it also publishes both tags to Docker Hub (requires `docker login`).
set -euo pipefail

cd "$(dirname "$0")/.."
image="federicosbringhetti/intime"
version="$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n 1)"

output="--load"
case "${1:-}" in
  "") ;;
  --push) output="--push" ;;
  *) echo "usage: $0 [--push]" >&2; exit 2 ;;
esac

echo "Building $image:$version ($output)"
docker buildx build \
  --platform linux/arm64 \
  --tag "$image:$version" \
  --tag "$image:latest" \
  "$output" \
  .
