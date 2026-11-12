# intime

Personal service for real-time GTT public transport departures in Turin: stop search by name and position, real-time departures with line filters, favorite stops, and a web app for the phone, installable as "InTime". A Rust backend with SQLite, packaged as a Docker image for an Orange Pi 5.
Inspired by [madbob/gtt-pirate-api](https://github.com/madbob/gtt-pirate-api).

**Status**: Phase 1 (core backend) and Phase 2 (web app, installable as "InTime") done, released as `v0.1.0` and `v0.2.0`. Phase 3 adds GTT open data, rules, trip tracking and Web Push notifications; Phase 4 adds Cloudflare Access and the first deployment. See [PLAN.md](PLAN.md).

## Quick start

From source (the first command builds the web app; without it `cargo run` serves the API only):

```bash
cd web && npm ci && npm run build && cd ..
```

```bash
cargo run
```

The web app is then on `http://localhost:8080`; the API examples:

```bash
curl -s 'http://localhost:8080/api/stops?q=porta%20nuova&limit=3'
```

```bash
curl -s 'http://localhost:8080/api/stops/39/departures?limit=5'
```

With Docker (local testing):

```bash
scripts/build-image.sh
```

```bash
cp .env.example .env
```

```bash
docker compose up -d
```

The service is then on `http://localhost:8100`; requests need `Authorization: Bearer <API_TOKEN from .env>`.

## Documentation

Start at [docs/README.md](docs/README.md).

| Topic | Document |
|-------|----------|
| Every endpoint, with examples | [docs/user/api.md](docs/user/api.md) |
| Environment variables | [docs/user/configuration.md](docs/user/configuration.md) |
| Docker image and deployment | [docs/user/deploy.md](docs/user/deploy.md) |
| How it works, code map, limits | [docs/architecture.md](docs/architecture.md) |
| How it was built, step by step | [docs/development/](docs/development/README.md) |

## Checks

```bash
cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test
```

```bash
cd web && npm run check && npm test && npm run build
```
