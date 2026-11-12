# 6. Docker

> Phase 1, step 6 · PR [#8](https://github.com/fedsbringhetti/intime/pull/8)

## Goal

Package the service as a Docker image for the Orange Pi 5 (`linux/arm64`) and prepare the files to deploy it on the home server following its conventions. The deployment itself was deferred (now to Phase 4, after the web app and the notifications): for now the image is used for local testing.

## Decisions

| Decision | Why |
|----------|-----|
| **Multi-stage build**, `cargo-chef` for dependency caching | The Rust toolchain stays out of the final image; changing only our code recompiles in seconds instead of minutes. |
| **`debian:bookworm-slim` runtime** | Same glibc as the build stage; has a shell for debugging on the server. A distroless base would be smaller but has no shell. |
| **Non-root user `app` (uid 10001)** | Required by the server conventions; limits damage if the process is compromised. |
| **`healthcheck` subcommand** instead of curl | The image has no curl; the binary can call its own `/health`. |
| **Host port 8100 → container 8080** | 8080 is taken on the server; the code does not change. |
| **Bind mount `./data`, not a named volume** | The server's backup script saves folders next to `docker-compose.yml`. Requires the folder to belong to uid 10001. |
| **Secrets only in `.env`** (`env_file`), `.env.example` in the repository | The image is public; nothing secret can be in it or in the compose file. |
| **Docker Hub** (`federicosbringhetti/intime`) instead of GHCR | Server conventions; decision F updated. |
| **Deployment deferred to the web app phase** | Nothing to use on the server until there is a user interface. Planned with Phase 3 at the time; the phases were swapped on 2026-10-30, so it became Phase 2; on 2026-11-02 it moved to Phase 4, after the whole service. |

## Files

| File | Role |
|------|------|
| `Dockerfile` | build and runtime stages |
| `.dockerignore` | keeps secrets, data, tests, docs and Git history out of the build context |
| `docker-compose.yml` | the service on the home server |
| `.env.example` | settings with placeholder values |
| `scripts/build-image.sh` | builds `linux/arm64`, optionally pushes |
| `src/main.rs` | `healthcheck` subcommand |
| `docs/user/deploy.md` | deployment steps and summary |

## How the code works

### Dockerfile

```dockerfile
FROM rust:1.95-slim-bookworm AS chef
RUN cargo install cargo-chef --locked
WORKDIR /app

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json
COPY . .
RUN cargo build --release --locked

FROM debian:bookworm-slim AS runtime
# ... ca-certificates, tzdata, user app (uid 10001), /data owned by app
COPY --from=builder /app/target/release/intime /usr/local/bin/intime
USER app
ENV BIND_ADDR=0.0.0.0:8080 DATA_DIR=/data TZ=Europe/Rome RUST_LOG=info
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD ["intime", "healthcheck"]
ENTRYPOINT ["intime"]
```

1. **planner** reduces the project to `recipe.json`: the list of dependencies, without our code.
2. **builder** compiles the dependencies from the recipe first. Docker caches that layer, and it is reused as long as `Cargo.toml` and `Cargo.lock` do not change. Then it copies the code and compiles only our crate (about 4 seconds).
3. **runtime** gets only the binary. `ca-certificates` is needed for HTTPS to the upstream; `tzdata` for local times. Migrations are embedded in the binary.

`BIND_ADDR=0.0.0.0:8080` makes the service reachable from outside the container (`127.0.0.1` would not be).

### The `healthcheck` subcommand

```rust
let args: Vec<String> = std::env::args().skip(1).collect();
let args: Vec<&str> = args.iter().map(String::as_str).collect();
match args.as_slice() {
    [] => serve(config).await,
    ["healthcheck"] => healthcheck(config.bind_addr).await,
    _ => {
        eprintln!("usage: intime [healthcheck | vapid-keys]");
        ExitCode::from(2)
    }
}
```

`healthcheck` calls `http://<address>/health` with a 3-second timeout and exits 0 or 1. A wildcard bind address (`0.0.0.0`, `::`) becomes loopback, since nothing listens on `0.0.0.0` as a destination.

> **Rust concept: slice patterns.** Matching on `.as_slice()` with `[]` and `["healthcheck"]` checks the number of arguments and their values in one place.

### docker-compose.yml

```yaml
services:
  intime:
    image: federicosbringhetti/intime:0.2.0
    container_name: intime
    restart: unless-stopped
    ports:
      - "8100:8080"
    volumes:
      - ./data:/data
    env_file: .env
    environment:
      TZ: Europe/Rome
    healthcheck:
      test: ["CMD", "intime", "healthcheck"]
      interval: 30s
      timeout: 5s
      start_period: 10s
      retries: 3
    mem_limit: 128m
```

The image tag is pinned; rolling back means changing it.

### Build script

`scripts/build-image.sh` reads the version from `Cargo.toml` and runs `docker buildx build --platform linux/arm64` with two tags, `<version>` and `latest`. Without arguments it loads the image locally; `--push` publishes it.

## Tests

The Rust tests gained one unit test (`local_addr`). The image was checked by running it:

- built natively for arm64 on the Mac, the same architecture as the Orange Pi;
- started with `docker-compose.yml` from a folder that mimics the server one: `healthy` in 5 seconds, 19,422 stops synced in 1.8 s, `/health` 200 and `/api/*` 401 without token on port 8100, `TZ=Europe/Rome`, uid 10001, database in `./data`;
- 32 MB of memory after sync and requests, 5 MB right after a restart;
- restart in 0.14 s with a clean shutdown and data kept.

| Phase 1 criterion | Result |
|-------------------|--------|
| Image under 40 MB | 38 MB to download (138 MB uncompressed: 108 MB Debian base, 15 MB binary) |
| Under 50 MB of RAM at idle | 32 MB |
| Survives a restart without losing data | yes |
| Runs on the Orange Pi | deferred to Phase 4 |

## Check it by hand

```bash
scripts/build-image.sh
```

```bash
cp .env.example .env
```

```bash
docker compose up -d
```

```bash
docker compose ps
```

The container should be `healthy` within a few seconds; the service is on `http://localhost:8100`.

## Pitfalls

- **Bind mount permissions.** If Docker creates `./data` on Linux, it belongs to root and the service, running as uid 10001, exits with `cannot open the database in /data`. The deployment steps create the folder and `chown 10001` it first. Docker Desktop on the Mac hides this; it was reproduced with a volume mounted with `nocopy`, which behaves like a bind mount. (A plain named volume does not show the problem, because Docker copies the image's `/data` ownership into it.)
- **Image size reporting.** `docker image ls` shows the uncompressed size (176 MB with metadata); the size that matters for downloads is the compressed one, 38 MB.
- **`.dockerignore` and public images.** Only the binary reaches the final image, but the build context is still kept minimal (1.5 kB), so secrets or data never even reach the builder.
- **Release builds on the Mac** fail (see [step 3](03-stop-index.md)); inside Docker the build runs on Linux and works.

## Later changes

- The first real deployment will follow [../user/deploy.md](../user/deploy.md), planned in Phase 4.
- [Chapter 17](17-web-push.md) adds the `vapid-keys` subcommand (handled before the configuration is read, so `docker run --rm federicosbringhetti/intime vapid-keys` works with no environment) and moves the argument parsing above `Config::from_env`; the excerpt above is shown as it is today.
- [Step 7](07-web-skeleton.md) adds a first stage, `web`, that builds the web app with Node; the builder copies its output before `cargo build`.
