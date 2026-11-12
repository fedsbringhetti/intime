# Configuration

Environment variables, all optional. In Docker they go in `.env` (see `.env.example`).

| Variable | Default | Purpose |
|----------|---------|---------|
| `API_TOKEN` | unset | bearer token for `/api/*`; unset means open (a warning is logged) |
| `BIND_ADDR` | `0.0.0.0:8080` | listen address |
| `DATA_DIR` | `data` (`/data` in Docker) | SQLite database folder |
| `STOPS_SYNC_TIME` | `04:00` | nightly stop sync, Europe/Rome |
| `DEFAULT_FEEDS` | `gtt` | feeds searched when a request does not say |
| `UPSTREAM_URL` | muoversiatorino endpoint | upstream GraphQL API |
| `UPSTREAM_TIMEOUT_SECS` | `10` | timeout for departures calls (the full stop list has 60 s) |
| `DEPARTURES_CACHE_SECS` | `20` | how long departures are reused |
| `GTT_REALTIME_URL` | `https://percorsieorari.gtt.to.it/das_gtfsrt/` | folder of GTT's real-time feeds; empty disables GTT's predictions and alerts (OpenTripPlanner only) |
| `GTT_TRIP_UPDATES_CACHE_SECS` | `20` | how long GTT's trip updates are reused |
| `GTT_ALERTS_POLL_SECS` | `300` | how often GTT's alerts are read, at least 1 |
| `GTT_GTFS_URL` | `https://www.gtt.to.it/open_data/gtt_gtfs.zip` | GTT's static data, for its `stops.txt`: checked every night 10 minutes after `STOPS_SYNC_TIME`, downloaded only when it changed. Empty disables it: alerts are then linked to stops by their lines only |
| `VAPID_PRIVATE_KEY` | unset | the server's Web Push key (see below); unset or empty disables Web Push: notifications are only written to the log |
| `VAPID_SUBJECT` | unset | contact of the Web Push identity, `mailto:you@example.com` or an `https:` URL; required when `VAPID_PRIVATE_KEY` is set, ignored otherwise |
| `SCHEDULER_TICK_SECS` | `15` | how often the scheduler looks for rules that are due, at least 1 |
| `MISSED_RUN_GRACE_MINUTES` | `5` | a summary found due up to this many minutes late is still sent (server restarting, upstream down); later it is skipped. At least 1 |
| `RUST_LOG` | `info` | log level, e.g. `debug` or `info,intime::upstream=debug` |

### Web Push keys

Web Push needs a key pair that identifies the server to the browsers' push services (VAPID). Generate it once:

```bash
cargo run -- vapid-keys
```

or, with the Docker image (the entry point is the binary, and the command needs no other configuration):

```bash
docker run --rm federicosbringhetti/intime vapid-keys
```

It prints `VAPID_PRIVATE_KEY=...` (to paste into `.env`) and, as comments, the public key and a reminder to set `VAPID_SUBJECT`. Only the private key is configured: the service derives the public one and gives it to the browsers.

- `VAPID_PRIVATE_KEY` is the 32-byte P-256 private key in base64url without padding (43 characters). An invalid value stops the service at startup, without printing it. Keep it secret like `API_TOKEN`: it never appears in the logs.
- `VAPID_SUBJECT` must be `mailto:` or `https:` followed by an address (`mailto:` alone is refused); without a valid one, a key stops the service at startup.
- **Do not change the key once phones have subscribed**: a subscription belongs to the key it was made with. The web app notices at the next opening of the Notifiche screen and subscribes again, but until then pushes to the old subscriptions fail.

An invalid value stops the service at startup with a message naming the variable.

In Docker, `BIND_ADDR`, `DATA_DIR` and `TZ` are set by the image or `docker-compose.yml`; the rest goes in `.env` ([deploy.md](deploy.md)).
