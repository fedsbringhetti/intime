# 16. Rules

> Phase 3, step 3 · PR [#23](https://github.com/fedsbringhetti/intime/pull/23)

## Goal

The service now stores rules: a **summary** of the next departures from a stop at a fixed time, or the **tracking** of one run (line and scheduled time at a stop), on chosen days of the week. Each rule has a calendar that knows Italian public holidays, pauses, daylight saving and midnight, and the API shows what a rule does today and when it runs next. Nothing is sent yet: the scheduler comes with step 5, tracking with step 6, the screens with step 7.

## Decisions

| Decision | Why |
|----------|-----|
| **`kind` plus a `config` object** (decision AG) | The two kinds have different fields. The API mirrors the database (`kind` and `config` columns), and each kind's fields are a struct with `deny_unknown_fields`, so a typo (`"windows_minutes"`) is refused instead of ignored. All fields at one level would read better, but serde cannot refuse unknown fields through `#[serde(flatten)]`. |
| **Defaults stored in the rule** (decision AG) | A rule created with "reminders 10 and 5" keeps them even if the default changes later; what the database holds is what the rule does. |
| **Lines checked against the stop** (decision AG) | The editor (step 7) offers the lines of the stop; the API refuses others, so a rule cannot wait every day for a line that never passes. Lines take the index's spelling (`9b` → `9B`), which is what departures carry. |
| **`paused_until` is the last day skipped, included** (decision AH, changed from the plan) | The approved mockup says "In pausa fino al 16 novembre": in Italian that includes the 16th. "Salta oggi" becomes `until` today. A pause without a date turns the rule off, so `resume` covers both ("Riprendi" in the mockup). |
| **`rule_runs` waits for the scheduler** (decision AF) | Nothing writes it before step 5, and the scheduler decides what its state holds. A merged migration can never change. |
| **Days are calendar days in Rome** (decision AI) | "Saturday 00:30" is what the user sees on the clock: the night between Friday and Saturday, whatever GTT's service date of that run. |
| **`today` and `next` computed at each request** (decision AJ) | The Notifiche screen shows today's state of every rule ("Oggi alle 18:10", "Non oggi", "In pausa"). Until step 5 records what happened, the calendar alone gives it, and it makes the calendar visible with `curl`. |
| **4 October from 2026** (decision K) | San Francesco is a national holiday again from 1 January 2026 (law 151/2025); found while planning this step. |
| **Easter computed, no crate** | The anonymous Gregorian algorithm is a dozen integer operations, tested against known dates; a crate for it would be a dependency for one function. |
| **One-off trackings refused by `/api/rules`** (decision AJ) | They are started from a departure (`/api/trackings`, step 6) with a known run. The `date` column and the calendar already handle them. |

## Files

| File | Role |
|------|------|
| `migrations/0004_rules.sql` | the `rules` table |
| `src/rules/mod.rs` | `Rule`, `RuleConfig` with `SummaryConfig` and `TrackConfig`, `RuleStore`, days and times in JSON |
| `src/rules/schedule.rs` | `Schedule`: what a rule does on a day (`Day`), its next occurrence, local times |
| `src/rules/holidays.rs` | Italian public holidays and Easter |
| `src/api/rules.rs` | handlers: checks, defaults, lines against the stop, pause and resume, `today` and `next` |
| `src/api/mod.rs`, `src/api/error.rs`, `src/main.rs` | routes, `RuleStore` in the state, `rule_not_found` |
| `src/api/favorites.rs` | `clean_lines` shared with summaries |
| `tests/rules.rs`, `tests/common/mod.rs` | integration tests; `synced_app_at` for a chosen clock |

## How the code works

### Stored as JSON, read as types

A rule's kind-specific fields live in one JSON column. In Rust they are an enum with one struct per kind; serde writes it as `kind` plus `config`:

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "config", rename_all = "snake_case")]
pub enum RuleConfig {
    Summary(SummaryConfig),
    Track(TrackConfig),
}
```

> **Rust concept: adjacently tagged enums.** By default serde writes an enum as `{"Summary": {...}}`. `tag` and `content` put the variant's name and its data in two fields side by side, `{"kind": "summary", "config": {...}}`, which is what the API and the two database columns look like. `#[serde(flatten)]` on the `config` field of `Rule` lifts both fields into the rule's own JSON.

Reading a row picks the struct from the `kind` column, so a stored config that does not match its kind is an error, not a silent default:

```rust
    fn from_json(kind: &str, json: &str) -> Result<Self, RulesError> {
        match kind {
            "summary" => Ok(Self::Summary(serde_json::from_str(json)?)),
            "track" => Ok(Self::Track(serde_json::from_str(json)?)),
            other => Err(RulesError::Kind(other.to_owned())),
        }
    }
```

Days are written `["mon", ...]` and times `"07:40"` by two small serde modules (`weekdays`, `hh_mm`), used both by the API and by the database columns. `hh_mm` accepts exactly two digits each, so `7:40` and `07:40:00` are refused alike.

### Checks on the way in

The API reads `config` as plain JSON first, then as the struct of the rule's kind, with every number optional; the missing ones get their defaults and every value its range. For a tracking rule, a reminder longer than the watch window could never be sent, so it is refused:

```rust
            // A reminder before tracking starts could never be sent.
            if reminders.first().is_some_and(|longest| *longest > watch_from_minutes) {
```

Then the lines are matched against those of the stop in the index, and take its spelling:

```rust
    for line in lines {
        let served = stop
            .lines
            .iter()
            .find(|served| served.name.eq_ignore_ascii_case(line))
            .ok_or_else(|| {
                ApiError::InvalidRequest(format!("line {line:?} does not serve stop {}", stop.id))
            })?;
        *line = served.name.clone();
    }
```

`PATCH` loads the rule, applies the fields given and saves the whole row. The lines are checked again only when the stop or the config changes, so a rule whose stop lost a line after a sync can still be renamed or paused:

```rust
    let check_stop = body.stop.is_some() || body.config.is_some();
    if let Some(config) = body.config {
        rule.config = parse_config(kind, config)?;
    }
```

### What a rule does on a day

`Schedule` holds the calendar of a rule; `Rule::schedule` builds it. `on(date)` checks, in order, whether the rule is off, paused, on one of its days, on a holiday it skips:

```rust
        if let Some(until) = self.paused_until.filter(|until| date <= *until) {
            return Day::Paused { until };
        }
```

The pause comes before "not one of its days", so a paused rule says "In pausa" every day, as in the mockup. `next(now)` walks the days from today (in Rome) and takes the first occurrence not yet passed:

```rust
        today
            .iter_days()
            .take(NEXT_HORIZON_DAYS)
            .take_while(|date| self.date.is_none_or(|only| *date <= only))
            .find_map(|date| match self.on(date) {
                Day::Due { at } if at >= now => Some(at),
                _ => None,
            })
```

400 days are enough for a pause of up to a year plus holidays. A one-off rule stops looking after its date.

### Local times and daylight saving

A rule says "07:40"; the instant depends on the day. `local_time` asks chrono-tz which instants that local time is on that date:

```rust
        match TIME_ZONE.from_local_datetime(&local) {
            LocalResult::Single(time) => return time,
            LocalResult::Ambiguous(first, _) => return first,
            LocalResult::None => local += TimeDelta::minutes(1),
        }
```

On 29 March 2026 clocks jump from 02:00 to 03:00, so 02:30 does not exist (`None`): it moves forward a minute at a time until 03:00. On 25 October 2026 they go back from 03:00 to 02:00, so 02:30 happens twice (`Ambiguous`): the first one counts, and a summary is sent once. A rule at 07:40 is at 07:40 local on both sides of the change, whatever the UTC offset.

"Today" is computed in Rome too (`local_date`): at 23:30 UTC on a Thursday it is already Friday in Rome, and `Utc::now().date_naive()` would be a day behind for an hour or two every night.

### Holidays

`holiday(date)` matches the fixed dates, then Easter and the day after:

```rust
        (10, 4) if date.year() >= SAN_FRANCESCO_FROM => Some("San Francesco"),
```

Easter comes from the anonymous Gregorian algorithm (also called Meeus/Jones/Butcher): integer divisions and remainders of the year, no tables.

### Pause and resume

`POST /api/rules/{id}/pause` takes an optional body. axum's `Option<Json<T>>` gives `None` when the request has no `Content-Type`, so `curl -X POST .../pause` turns the rule off, and a date pauses it:

```rust
    match body.and_then(|Json(body)| body.until) {
        Some(until) => {
            let today = local_date(state.clock.now());
            let last = today + TimeDelta::days(MAX_PAUSE_DAYS);
```

A pause with a date also turns the rule on: choosing "Metti in pausa fino a…" on a rule that was off means it should come back after that date.

## Tests

- **Unit, Rust** (24 new): `holidays.rs` (5): Easter of nine years, including the earliest (22 March 2285) and latest (25 April 1943) dates; all the holidays of 2026; San Francesco from 2026; Easter Monday moving with Easter, and 25 April 2011, when it fell on Liberation Day; ordinary days. `schedule.rs` (10), all with fixed dates and times instead of a clock: days of the week; holidays skipped or not; a pause including its last day and shown on the other days; disabled rules; one-off dates; the next occurrence (same day, the minute itself, the day after, the weekend, Christmas, the new year); midnight in Rome; the start of daylight saving (01:59, 02:00, 02:30, 03:00 on 29 March 2026) and its end (02:30 twice, 03:00, the next one after the repeated hour); the same local time across the change. `rules/mod.rs` (3): `kind` and `config` in JSON, `HH:MM`, day names. `api/rules.rs` (6): defaults of each kind, ranges and fields of the wrong kind, reminders, days, names.
- **Integration** (`tests/rules.rs`, 9), on the stops of `all_stops.json` and a fixed clock: a summary created with its defaults and the index's spelling of lines, read back alone and in the list; a tracking rule on Saturdays; invalid bodies (kind, name, days, stop, `date`, times, a line not serving the stop, fields of the other kind, unknown fields, no config) and an unknown stop; the index not ready; updates (fields, `config` replaced whole, a new stop with its lines checked, `kind` refused, unknown rule); "salta oggi", a pause until the 16th, off without a date, on again, resume, dates out of range; a holiday (Immacolata) skipped or not; today in Rome at 23:30 UTC; deletion.

Results: 181 Rust tests (33 new); the web app did not change.

## Check it by hand

```bash
DATA_DIR=/tmp/gtt-copy cargo run
```

On a copy of `data/`, as for chapter 15 (see Pitfalls).

```bash
curl -s -X POST localhost:8080/api/rules -H 'Content-Type: application/json' -d '{"name": "Casa → lavoro", "kind": "summary", "stop": "39", "days": ["mon", "tue", "wed", "thu", "fri"], "config": {"time": "07:40", "lines": ["9b", "67"]}}' | jq '{config, today, next}'
```

```bash
curl -s -X POST localhost:8080/api/rules/1/pause -H 'Content-Type: application/json' -d '{"until": "2026-11-06"}' | jq '{paused_until, today, next}'
```

Done on Thursday 5 November 2026 at 11:12, on a copy of the data folder: migration 4 applied; the summary stored lines `["9B", "67"]` with `window_minutes` 30 and `max_departures` 6, `today` due at 07:40 (already passed) and `next` Friday at 07:40. A tracking rule on Saturdays got its defaults and `next` Saturday 7 at 08:05; line `4` at stop 39 was refused. A pause until Friday 6 gave `next` Monday 9; a pause without a date turned the rule off (`next: null`); resume turned it on. Changing the summary to Sundays at 02:30 gave `next` Sunday 8 at 02:30. Deleting answered 204.

## Pitfalls

- **`flatten` and unknown fields.** The first idea for the API was one flat object with the fields of both kinds. serde's `deny_unknown_fields` does not work through `#[serde(flatten)]`, so mistyped fields would have been ignored silently; `config` as its own object fixed it, and matches the database.
- **"Fino al" in the plan and in the mockup.** The plan's "skip today = until tomorrow" made `paused_until` the day the rule starts again, while the approved mockup says "In pausa fino al 16 novembre". Reading it as Italian speakers do, the date became the last day skipped (decision AH).
- **A new national holiday.** The plan listed the national holidays as known; checking them found 4 October, a holiday again from 2026. It fell on a Sunday this year, so it would have shown up only in 2027.
- **A migration and the usual database.** As in chapter 15, a new migration would make `data/gtt.db` unusable by `main`; the check by hand ran on a copy of the folder, with a temporary entry in `.claude/launch.json` removed at the end.

## Later changes

None yet.
