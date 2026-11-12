# syntax=docker/dockerfile:1

# The web app: plain HTML, CSS and JavaScript, the same on every platform, so this stage runs on
# the machine doing the build ($BUILDPLATFORM) instead of under arm64 emulation. Dependencies are
# installed before the sources are copied, so editing a component does not reinstall them.
FROM --platform=$BUILDPLATFORM node:24-slim AS web
WORKDIR /web
COPY web/package.json web/package-lock.json ./
RUN npm ci
COPY web/ ./
RUN npm run build

# Build stages share the Rust toolchain and cargo-chef, which caches dependencies in their own
# layer: changing only our code does not recompile them.
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
# rust-embed includes web/dist/ in the release binary, so it must be there before the build.
COPY --from=web /web/dist web/dist
RUN cargo build --release --locked

FROM debian:bookworm-slim AS runtime
LABEL org.opencontainers.image.source="https://github.com/fedsbringhetti/intime" \
      org.opencontainers.image.description="Personal API for real-time GTT departures in Turin"
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates tzdata \
    && rm -rf /var/lib/apt/lists/*
RUN useradd --system --uid 10001 --no-create-home --shell /usr/sbin/nologin app \
    && mkdir /data \
    && chown app /data
COPY --from=builder /app/target/release/intime /usr/local/bin/intime

USER app
ENV BIND_ADDR=0.0.0.0:8080 \
    DATA_DIR=/data \
    TZ=Europe/Rome \
    RUST_LOG=info
EXPOSE 8080
VOLUME /data

HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD ["intime", "healthcheck"]
ENTRYPOINT ["intime"]
