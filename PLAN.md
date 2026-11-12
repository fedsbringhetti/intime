# Development plan — intime

Personal service for real-time GTT public transport departures in Turin, hosted in Docker on an Orange Pi 5 (ARM64).
Inspired by [madbob/gtt-pirate-api](https://github.com/madbob/gtt-pirate-api).

This document defines the phases. The detailed plan for each phase is written together, one phase at a time, before development starts.

Renamed from `gtt-personal-api` to `intime`, the name of the app, on 2026-11-05: the GitHub repository, the crate and binary, the Docker image and container, and the server folder.

## Context

- **Data source**: OpenTripPlanner GraphQL API at `https://plan.muoversiatorino.it/otp/routers/mato/index/graphql` (unofficial, may break without notice).
- **What works upstream** (verified on 2026-10-29):
  - `stop(id: "gtt:<code>")` returns stop details and departures in a single query.
  - `stopsByRadius(lat, lon, radius)` for nearby stops.
  - `stops { ... }` returns the full list (about 7,058 GTT stops).
  - Available feeds: `gtt` (GTT urban, suburban, SFM rail, tourist), `2` (Trenitalia), `3` (Arriva), `4` (regional bus companies).
- **What does not work upstream**: `stops(name: ...)` returns an empty list, so name search must use a local index.
- **Useful data available**: line, destination (`headsign`), vehicle mode (bus/tram/metro), scheduled and real-time times, `serviceDay`, trip id, coordinates, lines serving a stop.
- **GTT open data** (verified on 2026-11-02), used from Phase 3:
  - GTFS static: `https://www.gtt.to.it/open_data/gtt_gtfs.zip` (about 20 MB, rebuilt about weekly, GTT only). Its `stop_id` is not our stop code: `gtt:39` is `stop_code` 39, `stop_id` 2843. `stop_times.txt` alone is 116 MB (1.83 million rows).
  - GTFS Realtime, Protocol Buffers over HTTP, polled: `https://percorsieorari.gtt.to.it/das_gtfsrt/` + `trip_update.aspx`, `vehicle_position.aspx`, `alerts.aspx`.
  - Trip and route ids match OpenTripPlanner (`28378913U` ↔ `gtt:28378913U`, `54U` ↔ `gtt:54U`); stop sequences and scheduled times match too (30 trips out of 30 sampled).
  - Trip updates and vehicle positions cover the **urban network only**. Trip updates have `stop_sequence` but no `stop_id`, and predictions for the next ~6 stops of a run only (median 15% of its stops).
  - The header `timestamp` is the time of the request (the feed is generated on demand), so freshness comes from each entity's timestamp, and every response differs byte by byte even when nothing changed.
  - Measured on a Monday morning: each vehicle updates every ~14 s (median; 10th–90th percentile 8–28 s), each trip update every ~38 s (30–100 s), alerts did not change in 2 minutes. Responses of 37, 77 and 137 KB, in 0.2–0.6 s. To be confirmed at other times.
  - Alerts are not available in OpenTripPlanner (its `alerts` query returns nothing). Predictions differ from OpenTripPlanner's by about 2 minutes (median of 41 comparisons), and at 10:18 OpenTripPlanner had no real-time data at all while the GTT feed did.

## Stack

- Rust: axum, tokio, reqwest, sqlx + SQLite (FTS5), serde, chrono-tz
- Multi-stage Docker image for `linux/arm64`, SQLite database on a volume

## Cross-cutting decisions

| # | Decision | Options | Choice |
|---|----------|---------|--------|
| 1 | Transport feeds | GTT only / all feeds | All indexed, search defaults to `gtt` only |
| 2 | Notification channel | Telegram / ntfy / Web Push (PWA) | Web Push from the web app (decided 2026-10-30): no third-party app; works on iPhone (web app added to the Home Screen, iOS 16.4+) and Android |
| 3 | Remote access | LAN only / Tailscale / Cloudflare Tunnel + login | The existing Cloudflare Tunnel → Nginx Proxy Manager on the server (HTTPS), protected by **Cloudflare Access** (decided 2026-10-30); `API_TOKEN` stays as a second layer |
| 4 | Users | just me / multiple people | Just me. Shared data (delay history) is stored per run, not per person, so more users can be added later without reshaping it |
| 5 | Rule types | scheduled summary / trip tracking / "leave now" | Scheduled summary and trip tracking, both on the fly and as recurring rules; no "leave now" (no walking time), reminders "N minutes before" instead (decided 2026-10-30) |
| 6 | Delay history | yes / no | Yes, for tracked runs only, stored per run (date, trip, stop) and shared by every rule (decided 2026-10-30) |
| 7 | Phase order | rules first / web app first | Web app first (decided 2026-10-30): Phase 2 is the web app, Phase 3 rules, tracking and notifications, which need the web app for Web Push. Authorization and the first deployment are Phase 4, after the whole service has been built and tested locally (decided 2026-11-02) |
| 8 | Phones | iPhone / Android | Both: the web app is designed for the stricter case (iPhone, installable PWA) |
| 9 | Language of user-facing text | Italian / English / both | Italian (web app, notifications); code and docs stay in English |
| 10 | Real-time source | OpenTripPlanner / GTT GTFS Realtime | GTT's own feed for urban runs, since GTT produces the data; then OpenTripPlanner's real time; then the scheduled time. OpenTripPlanner stays the source of stops and of which runs serve a stop, for every company. If GTT's predictions turn out clearly wrong, OpenTripPlanner goes back first (decided 2026-11-02) |
| 11 | GTFS static | full import / partial / none | Only `stops.txt` (GTT `stop_id` ↔ stop code); no timetables (decided 2026-11-02). Checked every night with a conditional request and downloaded only when GTT rebuilt it (changed from weekly on 2026-11-04). `routes.txt` dropped on 2026-11-04: its colors are per category, not per line, and below 4.5:1 contrast, so badges keep the colors by vehicle type |
| 12 | Testing on real phones | local HTTPS / temporary tunnel / at deployment | At deployment (Phase 4). Until then the service is tested on the Mac (browser and iOS Simulator), where `localhost` counts as secure for location, service worker and Web Push (decided 2026-11-02) |
| 13 | Name and icon of the app | GTT / a name of our own | **InTime** (provisional, may change), not the company's brand; icon: a white train in profile with a dish on its roof sending radio waves, on red (decided 2026-11-02) |

## Workflow

- **Language**: everything in the repository is in English: file and directory names, document contents, code, identifiers, branch names, commit messages, PR titles and descriptions.
- **Branches**: `main` always works. Every change starts from `main` in a short-lived branch (`feat/...`, `fix/...`, `docs/...`) and merges back through a PR once CI (`cargo fmt`, `cargo clippy`, `cargo test`) passes. No `dev` branch.
- **Releases**: semantic version tags on `main` (`v0.1.0` at the end of Phase 1, `v0.2.0` at the end of Phase 2, ...), each with a GitHub Release and notes. To try a version on the Pi before releasing it, use a pre-release tag (`v0.4.0-rc1`).
- **Docker images**: built on the Mac for `linux/arm64` and pushed to Docker Hub as `federicosbringhetti/intime` (public), tagged with the version and `latest`.
- **Deployment**: first deployment in Phase 4, once the web app, rules and notifications work locally (decided on 2026-10-29 with the web app; moved to Phase 4 on 2026-11-02). Until then the Docker image is only used for local testing. Then: on the home server in `/home/federico/intime/`, host port 8100, data in the `./data` bind mount. The compose file pins an exact version (e.g. `:0.1.0`), never `latest`; rolling back means changing the tag. Steps in [docs/user/deploy.md](docs/user/deploy.md).

---

## Phase 1 — Core backend

**Goal**: a reliable API to search stops and read departures, packaged as a Docker image for the Orange Pi (first deployment in Phase 4).

**Features**
- Client for the upstream GraphQL API (a single query per stop)
- Local copy of all stops, refreshed nightly, with name search
- Nearby stops search
- Stop details with the lines serving it
- Stop departures: line, destination, scheduled and real-time times, delay, vehicle mode, line filter
- Short in-memory cache to avoid overloading the upstream
- Favorite stops
- Health check, including upstream reachability
- Dockerfile and `docker-compose.yml` for arm64

**Detailed plan**: [docs/plan/phase-1.md](docs/plan/phase-1.md) (done; development story in [docs/development/](docs/development/README.md))

---

## Phase 2 — Web app

**Goal**: a modern, simple interface on the phone over the existing API, tested locally.

**Features**
- Installable web app (PWA) for iPhone and Android, in Italian, built with Svelte and served by the same binary as the backend
- Stop search by name and by location, departures with automatic refresh, favorites

**Detailed plan**: [docs/plan/phase-2.md](docs/plan/phase-2.md) (done, released as `v0.2.0` on 2026-11-02; authorization and deployment moved to Phase 4; development story in [docs/development/](docs/development/README.md), chapters 7–13)

---

## Phase 3 — GTT data, rules, trip tracking and notifications

**Goal**: automatically receive the departures I need, when I need them, as Web Push notifications from the web app.

**Features**
- Real-time predictions from GTT's own feed for urban runs, with OpenTripPlanner as fallback (decision 10)
- GTT service alerts in the departures and in the web app, and notified for the lines of a rule
- Vehicle positions during trip tracking: whether a vehicle is assigned to the run, and how many stops away it is
- Scheduled summary (e.g. "Mon–Fri at 7:40, lines 4 and 15 from stop X")
- Trip tracking, on the fly from the departures list or as a recurring rule: reminders N minutes before, delay and cancellation alerts, a live page
- Delay history of tracked runs
- Rule management in the web app
- Alert when the upstream stops working

**Detailed plan**: [docs/plan/phase-3.md](docs/plan/phase-3.md) (approved on 2026-11-02)

---

## Phase 4 — Authorization and first deployment

**Goal**: the complete service running on the home server, reachable from anywhere and protected.

**Features**
- Authorization with a Cloudflare Access signature or `API_TOKEN`
- First deployment on the home server, behind the Cloudflare Tunnel and Nginx Proxy Manager, protected by Cloudflare Access
- Checks on real phones: installation, location, offline, Web Push notifications on iPhone and Android

**Detailed plan**: [docs/plan/phase-4.md](docs/plan/phase-4.md) (moved from Phase 2 on 2026-11-02; hostname and Access policy still open)

---

## Future ideas

Not planned in any phase yet.

### Custom dates for rules

Rules already skip Italian public holidays and 24 June. Personal dates (holidays, days off) could be added as a list of extra dates to skip, per rule or for all rules. Proposed on 2026-10-30.

### Walking time ("leave now")

A "leave now" alert needs the walking time to the stop: entered by hand, estimated from the phone's position and the stop's coordinates, or computed with OpenTripPlanner's walking routes. Set aside on 2026-10-30 in favor of reminders "N minutes before" chosen by the user.

### Multiple users

Separate accounts with their own favorites and rules. Cloudflare Access can already let several email addresses in, but they would share the same data today. The delay history is stored per run, so it would be shared by all users and more precise with more of them.

### Vehicle map

A map in the web app with the vehicles of a line or near a stop, moving in real time. Proposed on 2026-11-02, after choosing GTT's vehicle positions feed for trip tracking in Phase 3.

- **Source**: `vehicle_position.aspx` from GTT open data (see Context): position, heading, run, line and vehicle number for about 450 urban vehicles; no speed and no current stop.
- **Needs**: a map library and tiles in the web app; line shapes from OpenTripPlanner (pattern geometry) to draw the route, so `shapes.txt` is not needed.
- **Limits**: urban network only; about a quarter of the vehicles have no run assigned; a few positions are minutes old and must be hidden or marked.

Replaces the earlier idea of reading positions from the 5T MQTT broker at `wss://mapi.5t.torino.it/scre` (found on 2026-10-29 through InfomezziMap), dropped on 2026-11-02: it needs the `Origin` of muoversiatorino.it, while GTT publishes the same data openly.

### Lighter requests

Fewer bytes between the phone and the server, which matters on a weak mobile connection. Proposed on 2026-11-02.

- **Today** (measured on 2026-11-02): the stop screen downloads its departures every 30 seconds, 7.5 KB each time for 30 rows (about 900 KB per hour with the screen open); a search is 6.7 KB. Nothing is compressed and there is no `ETag`. The app's own files are not downloaded again: from Phase 2 the service worker keeps them on the phone, so only API data travels.
- **Compression** (`gzip`/`br` with `tower-http`'s compression layer): the same departures become about 1 KB. The cheapest change and the largest gain.
- **Only what changed**: an `ETag` per response with `304 Not Modified` when nothing changed; it pays off only if the response does not change every minute, so `minutes` would be computed in the app from `expected`. A further step would send only the departures that changed since the last response (a delta).
- **Push instead of polling**: the server sends updates when they happen (Server-Sent Events or WebSocket) instead of the app asking every 30 seconds; it fits trip tracking in Phase 3.
- To decide with measurements on a real phone after the first deployment (Phase 4).

### Full GTFS import

A fallback if OpenTripPlanner disappears: import GTT's whole GTFS (calendars, 61,000 runs, 1.83 million stop times) and compute departures locally, merged with GTT's trip updates. A large piece of work, that would replace what OpenTripPlanner does for free; only for GTT, other companies would be lost. Proposed on 2026-11-02, not planned.
