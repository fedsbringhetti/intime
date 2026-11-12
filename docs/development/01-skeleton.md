# 1. Project skeleton

> Phase 1, step 1 · PR [#3](https://github.com/fedsbringhetti/intime/pull/3)

## Goal

An empty but complete service: a Rust crate that reads its configuration, logs, answers `GET /health`, shuts down cleanly, and is checked by CI on every PR. Every later step adds to this base.

## Decisions

| Decision | Why |
|----------|-----|
| **Rust** with `axum` and `tokio` | One small binary, low memory on the Orange Pi, no runtime to install. `axum` is the most used web framework on top of `tokio`, the standard async runtime. |
| **Library + binary** in one crate (`src/lib.rs` and `src/main.rs`) | Integration tests in `tests/` can only use a library. `main.rs` stays a thin layer that wires the pieces together. |
| **Configuration from environment variables**, read through a lookup function | Environment variables are the natural way to configure a container. Reading them through a function lets tests pass fake values without touching the real environment, which is shared by all tests running in parallel. |
| **Only `BIND_ADDR` at first** | A setting that nothing uses yet would trigger "field never read" warnings, and CI treats warnings as errors. Each later step adds its own settings. |
| **Graceful shutdown on Ctrl+C and SIGTERM** | `docker stop` sends SIGTERM; requests in progress finish instead of being cut. |
| **CI with `fmt`, `clippy -D warnings`, `test`** | Catches formatting, common mistakes and broken tests before merging, so `main` always works. |

## Files

| File | Role |
|------|------|
| `Cargo.toml` | crate name, edition 2024, dependencies |
| `src/lib.rs` | lists the modules, making them usable from tests |
| `src/main.rs` | startup: logging, configuration, server, shutdown |
| `src/config.rs` | `Config` and its parsing |
| `src/api/mod.rs` | the router: which path goes to which handler |
| `src/api/health.rs` | `GET /health` |
| `tests/health.rs` | integration tests for `/health` |
| `.github/workflows/ci.yml` | CI checks |
| `.gitignore` | `/target`, `/data`, `.env` |

## How the code works

### Startup

`main` is an async function run by `tokio`:

```rust
#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let config = match Config::from_env() {
        Ok(config) => config,
        Err(err) => {
            tracing::error!("configuration error: {err}");
            return ExitCode::FAILURE;
        }
    };
    // ...
```

- `#[tokio::main]` starts the async runtime and runs `main` inside it. Inside an `async fn`, every `.await` is a point where the function can pause while waiting (for the network, a timer...) and let other tasks run.
- Logging uses `tracing`. The level comes from `RUST_LOG` (default `info`). Logs go to stdout, which is what Docker collects.
- `main` returns an `ExitCode`: 0 on success, 1 on a configuration or startup error, so a supervisor (Docker) sees the failure.

> **Rust concept: `match` on `Result`.** A function that can fail returns `Result<T, E>`: either `Ok(value)` or `Err(error)`. `match` handles both cases explicitly. Elsewhere the code uses `?`, which means "if this is an error, return it from the current function; otherwise give me the value".

### Configuration

```rust
pub fn from_env() -> Result<Self, ConfigError> {
    Self::from_lookup(|name| std::env::var(name).ok())
}

pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Result<Self, ConfigError> {
    Ok(Self {
        bind_addr: parse_or_default(&lookup, "BIND_ADDR", DEFAULT_BIND_ADDR)?,
        // ... (later steps add more fields)
    })
}
```

- `from_env` reads the real environment; `from_lookup` receives any function that, given a variable name, returns its value or `None`. Tests pass a closure over a `HashMap`.
- `parse_or_default` takes the value or the default, then converts it with `FromStr` (the trait behind `"...".parse()`). On failure the error names the variable and the bad value:

```rust
fn parse_with<T>(
    lookup: &impl Fn(&str) -> Option<String>,
    var: &'static str,
    default: &str,
    parse: impl Fn(&str) -> Result<T, String>,
) -> Result<T, ConfigError> {
    let value = lookup(var).unwrap_or_else(|| default.to_owned());
    parse(&value).map_err(|reason| ConfigError::Invalid { var, value, reason })
}
```

> **Rust concept: generics and traits.** `parse_or_default<T>` works for any type `T` that implements `FromStr` (socket addresses, URLs, numbers, paths). The compiler generates one version per type used.

`ConfigError` uses `thiserror`, which turns the `#[error("...")]` attribute into a readable message:

```text
configuration error: invalid value "nope" for BIND_ADDR: invalid socket address syntax
```

### Router and the first handler

```rust
Router::new()
    .route("/health", get(health::health))
```

A handler is an async function whose return value can become an HTTP response:

```rust
pub async fn health() -> Json<Health> {
    Json(Health {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
    })
}
```

- `Json(...)` serializes the struct to JSON and sets `Content-Type: application/json`.
- `Health` derives `Serialize` from `serde`, so it does not need hand-written JSON code.
- `env!("CARGO_PKG_VERSION")` is filled in at compile time with the version from `Cargo.toml`.

### Graceful shutdown

```rust
axum::serve(listener, api::router(state))
    .with_graceful_shutdown(shutdown_signal())
    .await
```

`shutdown_signal` waits for whichever comes first, Ctrl+C or SIGTERM:

```rust
tokio::select! {
    () = ctrl_c => {},
    () = terminate => {},
}
```

`tokio::select!` runs several futures and continues with the first one that finishes. When it returns, `axum` stops accepting connections, completes the requests in progress, and `main` exits with code 0.

### CI

`.github/workflows/ci.yml` runs on every PR and every push to `main`: checkout, Rust toolchain with `rustfmt` and `clippy`, a cache of compiled dependencies (`Swatinem/rust-cache`), then the three checks. With `-D warnings`, any Clippy warning fails the build.

## Tests

- **Config** (unit tests in `src/config.rs`): defaults when nothing is set, a custom value, an invalid value with the right variable name in the error.
- **`/health`** (`tests/health.rs`): the router is called directly with `tower::ServiceExt::oneshot`, without opening a port. Checks status 200, `status: ok`, the version, and that an unknown path returns 404.

## Check it by hand

```bash
cargo run
```

```bash
curl -s http://localhost:8080/health
```

```bash
BIND_ADDR=nope cargo run
```

The last command exits with code 1 and a message naming `BIND_ADDR`.

## Pitfalls

- **Tests and the environment.** Setting environment variables inside tests is unsafe when tests run in parallel. Reading configuration through `from_lookup` avoids the problem entirely.
- **Unused settings.** Adding every planned setting up front would have produced warnings, and so CI failures. Settings arrive with the code that uses them.

## Later changes

- [Step 2](02-departures.md) adds the upstream settings and passes an `AppState` to the router.
- [Step 3](03-stop-index.md) adds `DATA_DIR`, `STOPS_SYNC_TIME`, `DEFAULT_FEEDS` and a custom parser (`parse_with`).
- [Step 5](05-robustness.md) adds `API_TOKEN` and splits the router into open and protected routes.
- [Step 7](07-web-skeleton.md) gives the router a fallback that serves the web app, so an unknown path now gets the page instead of 404 (the test moved to `tests/web.rs`), renames the CI job to `rust` and adds a `frontend` job.
- [Step 6](06-docker.md) adds the `healthcheck` subcommand to `main`.
- The docs tooling ([Keeping the docs in sync](README.md#keeping-the-docs-in-sync)) adds a `Docs` step to CI that checks links and code excerpts.
