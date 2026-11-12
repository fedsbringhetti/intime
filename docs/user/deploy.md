# Deployment on the home server

> **Not deployed yet.** Until Phase 4 (after the web app, rules and notifications) the image is only used for local testing: build it with `scripts/build-image.sh` (no `--push`), `cp .env.example .env`, `docker compose up -d`, then `http://localhost:8100`. The steps below are for the first real deployment.

The service runs as a single container on the home server (Orange Pi 5, ARM64, Debian 12, Docker 29, Compose v5). The image is built on the Mac for `linux/arm64` (native on Apple Silicon), pushed to Docker Hub as a public image, and pulled by the server.

## Summary

| Item | Value |
|------|-------|
| Container name | `intime` |
| Image | `federicosbringhetti/intime:<version>` (also `latest`) |
| Ports | host `8100` → container `8080` (host 8080 is taken) |
| Folder on the server | `/home/federico/intime/` |
| Bind mount | `./data` → `/data` (SQLite: `gtt.db`, `gtt.db-wal`, `gtt.db-shm`) |
| Environment | from `.env`: `API_TOKEN`; optional `RUST_LOG`, `STOPS_SYNC_TIME`, `DEFAULT_FEEDS`, `UPSTREAM_URL`, `UPSTREAM_TIMEOUT_SECS`, `DEPARTURES_CACHE_SECS`, `GTT_REALTIME_URL`, `GTT_TRIP_UPDATES_CACHE_SECS`, `GTT_ALERTS_POLL_SECS`, `GTT_GTFS_URL`, `SCHEDULER_TICK_SECS`, `MISSED_RUN_GRACE_MINUTES`; for Web Push, `VAPID_PRIVATE_KEY` and `VAPID_SUBJECT` (generate the key with `docker run --rm federicosbringhetti/intime vapid-keys`, see [configuration.md](configuration.md#web-push-keys)). Set by the image or compose: `BIND_ADDR`, `DATA_DIR`, `TZ` |
| Health check | `intime healthcheck` (calls `/health`), every 30 s |
| User in the container | `app`, uid 10001 |

## 1. Build and push (on the Mac)

The image version comes from `Cargo.toml`; releases are tagged on `main` (`v0.1.0`, ...).

```bash
docker login -u federicosbringhetti
```

```bash
scripts/build-image.sh --push
```

Equivalent to `docker buildx build --platform linux/arm64 -t federicosbringhetti/intime:<version> -t federicosbringhetti/intime:latest --push .`. Without `--push` the script only builds the image locally, to try it first.

The image is public. `.dockerignore` keeps secrets, local data, tests, docs, Git history and the local `web/node_modules` and `web/dist` out of the build context. The build compiles the web app in a Node stage and embeds it in the binary; the final image contains only that binary on top of `debian:bookworm-slim`.

## 2. Prepare the server (first time)

Create the folder and the data directory. The data directory must belong to uid 10001: if Docker creates it, it belongs to root and the service exits with `cannot open the database in /data`.

```bash
ssh -t federico@192.168.1.13 'mkdir -p /home/federico/intime/data && sudo chown 10001 /home/federico/intime/data'
```

Copy the compose file:

```bash
scp docker-compose.yml federico@192.168.1.13:/home/federico/intime/
```

Create `.env` directly on the server, so the token never leaves it (see `.env.example` for the optional variables):

```bash
ssh federico@192.168.1.13 'cd /home/federico/intime && printf "API_TOKEN=%s\n" "$(openssl rand -hex 32)" > .env && chmod 600 .env'
```

The server clock must be synchronized via NTP (`timedatectl` shows `System clock synchronized: yes`): the nightly sync and, later, scheduled notifications depend on it.

## 3. Start

```bash
ssh federico@192.168.1.13 'cd /home/federico/intime && docker compose up -d'
```

On first start the service downloads all stops (a few seconds).

## 4. Check

```bash
ssh federico@192.168.1.13 'docker inspect --format "{{.State.Health.Status}}" intime'
```

Should print `healthy` within about 30 seconds.

```bash
ssh federico@192.168.1.13 'cd /home/federico/intime && docker compose logs --tail 50'
```

Expect `listening on 0.0.0.0:8080` and `stop index synced: ... stops`.

```bash
curl -s http://192.168.1.13:8100/health
```

```bash
ssh federico@192.168.1.13 'cd /home/federico/intime && curl -s -H "Authorization: Bearer $(sed -n "s/^API_TOKEN=//p" .env)" http://localhost:8100/api/status'
```

`stop_index.count` should be about 19,000 and `upstream.ok` should be `true`.

```bash
ssh federico@192.168.1.13 'docker stats --no-stream intime'
```

Memory at idle should stay under 50 MB.

## 5. Update and roll back

1. On the Mac: bump `version` in `Cargo.toml`, tag the release, run `scripts/build-image.sh --push`.
2. On the server: change the image tag in `docker-compose.yml`, then run `docker compose pull && docker compose up -d`.

To roll back, put the previous tag back and run the same commands.

Phones that have the web app open, or installed on the Home Screen, get the new version the next time they open it or bring it back from the background: the app downloads it and shows "Nuova versione disponibile · Aggiorna". Until that tap they keep the version they have, which works offline too.

## Notes

- **Backup**: `./data` is what needs saving; copy the whole folder, since the database is split across `gtt.db`, `gtt.db-wal` and `gtt.db-shm`. Pausing the container for a few seconds is safe. If the pause overlaps the nightly stop sync (`STOPS_SYNC_TIME`, default 04:00), that sync may fail and is retried after 5 minutes. The stop index is rebuilt automatically anyway; the valuable data are favorites (and, from Phase 3, rules).
- **Logs** go to stdout only (`docker compose logs`); nothing is written to log files.
- **Remote access**: through the existing Cloudflare Tunnel and Nginx Proxy Manager, protected by Cloudflare Access (decided 2026-10-30, see [PLAN.md](../../PLAN.md)); the setup is part of the Phase 4 plan ([../plan/phase-4.md](../plan/phase-4.md)). Keep `API_TOKEN` set.
- **Limits**: non-root user, 128 MB memory limit, `restart: unless-stopped`.
- **Measured** on 2026-10-29 (arm64 under Docker on the Mac): download size about 38 MB (138 MB uncompressed: 108 MB Debian base, 15 MB binary); 32 MB of memory after sync and requests; first sync of 19,422 stops in 1.7 s; restart in 0.14 s. With the placeholder web app embedded (2026-10-30): binary 13.5 KB larger, download size unchanged (37.3 MB measured with `docker save | gzip`), 33 MB of memory after the first sync.
