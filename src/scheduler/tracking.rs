//! Tracking one run (Phase 3, step 6): which run, when to remind, when it is late or
//! cancelled, when it is over. [`step`] is a pure function of the stored state, the stop's
//! departures and the time; the scheduler stores its result and sends its notices.

use chrono::{DateTime, NaiveDate, TimeDelta, Utc};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};

use crate::{
    departures::{Departure, Source},
    display_name::display_name,
    gtt::trip_updates::TripUpdates,
    notify::Notification,
    rules::{NewRule, Rule, RuleConfig, RuleStore, RulesError, TrackConfig, schedule::Day},
    time::TIME_ZONE,
};

use super::store::{SchedulerStore, SchedulerStoreError, TrackRun};

pub const SEARCHING: &str = "searching";
pub const TRACKING: &str = "tracking";
pub const DONE: &str = "done";
pub const CANCELLED: &str = "cancelled";
pub const NOT_FOUND: &str = "not_found";
pub const STOPPED: &str = "stopped";
pub const SKIPPED: &str = "skipped";
pub const FAILED: &str = "failed";

/// A recurring run still not found this long before its time is given up (decision 11).
const NOT_FOUND_BEFORE: TimeDelta = TimeDelta::minutes(10);
/// A reminder more than this past its threshold is skipped, never sent late (decision 8).
const REMINDER_SLACK: TimeDelta = TimeDelta::minutes(2);
/// A run that vanishes from the list this close to its time has left (decision 12).
const VANISH_GRACE: TimeDelta = TimeDelta::minutes(2);
/// Another delay alert only when the delay grew by this many minutes (section 6).
const DELAY_STEP_MINUTES: u32 = 3;
/// A one-off with no data for this long after its time is given up (decision 11).
pub const GIVE_UP_AFTER: TimeDelta = TimeDelta::minutes(60);
/// A recurring run is not started later than this after its time.
pub const START_GRACE: TimeDelta = TimeDelta::minutes(10);
/// The least a one-off watches before the run (decision 4).
const MIN_WATCH_FROM: u32 = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    Reminder,
    Delay,
    Cancelled,
    NotFound,
}

impl EventKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Reminder => "reminder",
            Self::Delay => "delay",
            Self::Cancelled => "cancelled",
            Self::NotFound => "not_found",
        }
    }
}

/// A notification sent for the run, kept in the state (and shown on the live page).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrackEvent {
    pub kind: EventKind,
    /// The reminder threshold or the delay, in minutes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minutes: Option<u32>,
    /// The title of the notification.
    pub text: String,
    pub sent_at: DateTime<Utc>,
}

/// What is stored in `rule_runs.state` for a tracking run (decision 7). Every field about the
/// run is `None` until it is found.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TrackState {
    /// OpenTripPlanner's id, `gtt:` included.
    pub trip_id: Option<String>,
    pub service_date: Option<NaiveDate>,
    pub stop_sequence: Option<u32>,
    pub headsign: Option<String>,
    pub scheduled: Option<DateTime<Utc>>,
    pub expected: Option<DateTime<Utc>>,
    pub delay_seconds: Option<i64>,
    pub source: Option<Source>,
    /// The last time the run was seen in the departures.
    pub checked_at: Option<DateTime<Utc>>,
    /// Reminder thresholds already handled, sent or skipped.
    #[serde(default)]
    pub reminders_done: Vec<u32>,
    /// Minutes of the last delay alert.
    #[serde(default)]
    pub delay_alerted: Option<u32>,
    #[serde(default)]
    pub events: Vec<TrackEvent>,
}

impl TrackState {
    /// The state of a run just seen in `departure`.
    pub fn seeded(departure: &Departure, now: DateTime<Utc>) -> Self {
        let mut state = Self::default();
        state.refresh(departure, now);
        state
    }

    fn refresh(&mut self, departure: &Departure, now: DateTime<Utc>) {
        self.trip_id = Some(departure.trip_id.clone());
        self.service_date = departure
            .service_date
            .or_else(|| Some(departure.scheduled.date_naive()));
        self.stop_sequence = departure.stop_sequence.or(self.stop_sequence);
        self.headsign = departure.headsign.clone();
        self.scheduled = Some(departure.scheduled.to_utc());
        self.expected = Some(departure.expected.to_utc());
        self.delay_seconds = Some(departure.delay_seconds);
        self.source = Some(departure.source);
        self.checked_at = Some(now);
    }
}

/// What [`step`] decided about the run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackStatus {
    Searching,
    Tracking,
    Done,
    Cancelled,
    NotFound,
}

impl TrackStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Searching => SEARCHING,
            Self::Tracking => TRACKING,
            Self::Done => DONE,
            Self::Cancelled => CANCELLED,
            Self::NotFound => NOT_FOUND,
        }
    }
}

/// A notification to send; its event is already in the new state.
#[derive(Debug, Clone, PartialEq)]
pub struct Notice {
    pub event: TrackEvent,
    pub title: String,
    pub body: String,
}

impl Notice {
    pub fn notification(&self, rule_id: i64) -> Notification {
        Notification {
            title: self.title.clone(),
            body: self.body.clone(),
            url: format!("/corsa/{rule_id}"),
            tag: Some(format!("track-{rule_id}")),
        }
    }
}

/// What [`step`] needs besides the state and the departures.
#[derive(Debug, Clone)]
pub struct StepInput<'a> {
    pub config: &'a TrackConfig,
    /// The scheduled time of the occurrence (`Schedule::on`).
    pub at: DateTime<Utc>,
    /// A recurring rule searches its run; a one-off follows `config.trip_id`.
    pub recurring: bool,
    /// How the stop reads in a message: `Porta Nuova (39)`.
    pub stop: &'a str,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Step {
    pub state: TrackState,
    pub status: TrackStatus,
    pub notices: Vec<Notice>,
}

/// Evaluates a run that is `searching` or `tracking`. `departures` is the stop's whole list
/// (not `select`ed: a departure already passed still matters).
pub fn step(
    mut state: TrackState,
    input: &StepInput,
    departures: &[Departure],
    now: DateTime<Utc>,
) -> Step {
    let config = input.config;
    let mut notices = Vec::new();

    if state.trip_id.is_none() {
        match search(config, input.at, departures) {
            Some(found) => state.refresh(found, now),
            None => {
                if now < input.at - NOT_FOUND_BEFORE {
                    state.checked_at = Some(now);
                    return finish(state, TrackStatus::Searching, notices);
                }
                // Too late to be useful after the run's time: silent.
                if now <= input.at {
                    let body = closest_text(config, input.at, departures);
                    let title = format!("❓ {} non si trova", label(config));
                    push(
                        &mut state,
                        &mut notices,
                        EventKind::NotFound,
                        None,
                        title,
                        body,
                        now,
                    );
                }
                return finish(state, TrackStatus::NotFound, notices);
            }
        }
    }

    let found = departures
        .iter()
        .find(|departure| tracked(&state, departure));
    let Some(departure) = found else {
        // Not in the list: gone, or a glitch. Keep the last state unless it is time.
        let expected = state.expected.unwrap_or(input.at);
        let over = now >= expected || expected - now <= VANISH_GRACE;
        let status = if over {
            TrackStatus::Done
        } else {
            TrackStatus::Tracking
        };
        return finish(state, status, notices);
    };
    state.refresh(departure, now);
    let expected = departure.expected.to_utc();

    if departure.canceled {
        let body = match next_after(departure, departures) {
            Some(next) => format!("Il prossimo: {}", hm(next.expected.to_utc())),
            None => format!("Nessun altro {} nelle prossime 2 ore", config.line),
        };
        let title = format!("⚠️ {} è cancellato", label(config));
        push(
            &mut state,
            &mut notices,
            EventKind::Cancelled,
            None,
            title,
            body,
            now,
        );
        return finish(state, TrackStatus::Cancelled, notices);
    }
    if now >= expected {
        return finish(state, TrackStatus::Done, notices);
    }

    reminder(&mut state, &mut notices, input, departure, expected, now);
    delay_alert(&mut state, &mut notices, input, departure, now);
    finish(state, TrackStatus::Tracking, notices)
}

fn finish(state: TrackState, status: TrackStatus, notices: Vec<Notice>) -> Step {
    Step {
        state,
        status,
        notices,
    }
}

/// The smallest reminder threshold that is reached is handled together with every larger one;
/// it is sent only while the time is still near it (decision 8).
fn reminder(
    state: &mut TrackState,
    notices: &mut Vec<Notice>,
    input: &StepInput,
    departure: &Departure,
    expected: DateTime<Utc>,
    now: DateTime<Utc>,
) {
    let remaining = expected - now;
    let reached: Vec<u32> = input
        .config
        .alert_before_minutes
        .iter()
        .copied()
        .filter(|threshold| !state.reminders_done.contains(threshold))
        .filter(|threshold| remaining <= TimeDelta::minutes(i64::from(*threshold)))
        .collect();
    let Some(&smallest) = reached.iter().min() else {
        return;
    };
    state.reminders_done.extend(reached);
    if remaining <= TimeDelta::minutes(i64::from(smallest)) - REMINDER_SLACK {
        return;
    }
    let title = format!(
        "⏰ {} passa tra {}",
        label(input.config),
        minutes_text(smallest)
    );
    let body = format!(
        "{}{} · {}",
        hm(expected),
        if departure.source == Source::Scheduled {
            " (orario)".to_owned()
        } else if departure.delay_seconds >= 60 {
            format!(", +{}", departure.delay_seconds / 60)
        } else {
            String::new()
        },
        input.stop
    );
    push(
        state,
        notices,
        EventKind::Reminder,
        Some(smallest),
        title,
        body,
        now,
    );
}

/// A late run, from a real-time source, is told once and again every 3 more minutes.
fn delay_alert(
    state: &mut TrackState,
    notices: &mut Vec<Notice>,
    input: &StepInput,
    departure: &Departure,
    now: DateTime<Utc>,
) {
    if departure.source == Source::Scheduled || departure.delay_seconds < 0 {
        return;
    }
    let minutes = u32::try_from(departure.delay_seconds / 60).unwrap_or(u32::MAX);
    let wanted = match state.delay_alerted {
        None => minutes >= input.config.delay_alert_minutes,
        Some(last) => minutes >= last.saturating_add(DELAY_STEP_MINUTES),
    };
    if !wanted {
        return;
    }
    state.delay_alerted = Some(minutes);
    let title = format!(
        "🐢 {} è in ritardo di {}",
        label(input.config),
        minutes_text(minutes)
    );
    let body = format!(
        "Previsto alle {} · {}",
        hm(departure.expected.to_utc()),
        input.stop
    );
    push(
        state,
        notices,
        EventKind::Delay,
        Some(minutes),
        title,
        body,
        now,
    );
}

/// Records the event in the state and queues its notification.
fn push(
    state: &mut TrackState,
    notices: &mut Vec<Notice>,
    kind: EventKind,
    minutes: Option<u32>,
    title: String,
    body: String,
    now: DateTime<Utc>,
) {
    let event = TrackEvent {
        kind,
        minutes,
        text: title.clone(),
        sent_at: now,
    };
    state.events.push(event.clone());
    notices.push(Notice { event, title, body });
}

/// The departure the rule means today: same line, within the tolerance of `at`, the closest,
/// the earlier on a tie (decision 6).
fn search<'a>(
    config: &TrackConfig,
    at: DateTime<Utc>,
    departures: &'a [Departure],
) -> Option<&'a Departure> {
    if let Some(trip_id) = &config.trip_id {
        return departures
            .iter()
            .find(|departure| &departure.trip_id == trip_id);
    }
    let tolerance = TimeDelta::minutes(i64::from(config.tolerance_minutes));
    departures
        .iter()
        .filter(|departure| departure.line.eq_ignore_ascii_case(&config.line))
        .map(|departure| (departure, (departure.scheduled.to_utc() - at).abs()))
        .filter(|(_, difference)| *difference <= tolerance)
        .min_by_key(|(departure, difference)| (*difference, departure.scheduled))
        .map(|(departure, _)| departure)
}

fn tracked(state: &TrackState, departure: &Departure) -> bool {
    state.trip_id.as_deref() == Some(departure.trip_id.as_str())
        && match (state.service_date, departure.service_date) {
            (Some(tracked), Some(seen)) => tracked == seen,
            _ => true,
        }
}

/// The next run of the same line after `departure`, not cancelled.
fn next_after<'a>(departure: &Departure, departures: &'a [Departure]) -> Option<&'a Departure> {
    departures
        .iter()
        .filter(|other| other.line.eq_ignore_ascii_case(&departure.line))
        .filter(|other| !other.canceled && other.trip_id != departure.trip_id)
        .filter(|other| other.scheduled > departure.scheduled)
        .min_by_key(|other| other.expected)
}

/// `Più vicine: 07:58, 08:12`: the two runs of the line closest to `at`.
fn closest_text(config: &TrackConfig, at: DateTime<Utc>, departures: &[Departure]) -> String {
    let mut closest: Vec<&Departure> = departures
        .iter()
        .filter(|departure| departure.line.eq_ignore_ascii_case(&config.line))
        .filter(|departure| !departure.canceled)
        .collect();
    closest.sort_by_key(|departure| (departure.scheduled.to_utc() - at).abs());
    closest.truncate(2);
    if closest.is_empty() {
        return format!("Nessun {} nelle prossime 2 ore", config.line);
    }
    closest.sort_by_key(|departure| departure.expected);
    let times: Vec<String> = closest
        .iter()
        .map(|departure| hm(departure.expected.to_utc()))
        .collect();
    format!("Più vicine: {}", times.join(", "))
}

/// `Il 67 delle 08:05`.
fn label(config: &TrackConfig) -> String {
    format!(
        "Il {} delle {}",
        config.line,
        config.scheduled_time.format("%H:%M")
    )
}

fn minutes_text(minutes: u32) -> String {
    if minutes == 1 {
        "1 minuto".to_owned()
    } else {
        format!("{minutes} minuti")
    }
}

/// `08:05`, local time.
fn hm(time: DateTime<Utc>) -> String {
    time.with_timezone(&TIME_ZONE).format("%H:%M").to_string()
}

// One-off trackings (POST /api/trackings)

#[derive(Debug, thiserror::Error)]
pub enum TrackError {
    #[error(transparent)]
    Rules(#[from] RulesError),
    #[error(transparent)]
    Store(#[from] SchedulerStoreError),
}

/// A one-off tracking rule to create for `departure` at `stop_id`.
pub struct OneOff<'a> {
    pub stop_id: &'a str,
    pub departure: &'a Departure,
    /// Reminder thresholds, longest first (checked by the caller).
    pub alert_before_minutes: Vec<u32>,
    pub delay_alert_minutes: u32,
}

/// Creates the one-off `track` rule for a departure and today's run as `tracking`, or, when
/// the same stop and trip are already tracked, replaces that rule's reminders and delay
/// threshold and puts its run back to `tracking`. Returns the rule and whether it is new
/// (decision 4). The caller has checked that the departure is not cancelled nor departed.
pub async fn create_one_off(
    rules: &RuleStore,
    store: &SchedulerStore,
    one_off: OneOff<'_>,
    now: DateTime<Utc>,
) -> Result<(Rule, bool), TrackError> {
    let departure = one_off.departure;
    let scheduled = departure.scheduled;
    let watch_from = one_off
        .alert_before_minutes
        .iter()
        .copied()
        .max()
        .unwrap_or(0)
        .max(MIN_WATCH_FROM);
    let config = TrackConfig {
        line: departure.line.clone(),
        scheduled_time: scheduled.time(),
        tolerance_minutes: 0,
        watch_from_minutes: watch_from,
        alert_before_minutes: one_off.alert_before_minutes,
        delay_alert_minutes: one_off.delay_alert_minutes,
        trip_id: Some(departure.trip_id.clone()),
    };
    let date = scheduled.date_naive();
    let state = TrackState::seeded(departure, now);

    let existing = rules.list().await?.into_iter().find(|rule| {
        rule.stop_id == one_off.stop_id
            && rule.date == Some(date)
            && matches!(&rule.config, RuleConfig::Track(track)
                if track.trip_id.as_deref() == Some(departure.trip_id.as_str()))
    });
    if let Some(mut rule) = existing {
        rule.config = RuleConfig::Track(config);
        let rule = rules.update(&rule, now).await?.unwrap_or(rule);
        let run_date = rule.date.unwrap_or(date);
        let reset = store
            .get_track_run(rule.id, run_date)
            .await?
            .is_some_and(|run| run.status != TRACKING && run.status != SEARCHING);
        if reset {
            store
                .reset_track_run(rule.id, run_date, TRACKING, &state, now)
                .await?;
        } else {
            store
                .insert_track_run(rule.id, run_date, TRACKING, scheduled.to_utc(), &state, now)
                .await?;
        }
        return Ok((rule, false));
    }

    let rule = rules
        .create(
            NewRule {
                name: format!("{} delle {}", departure.line, scheduled.format("%H:%M")),
                enabled: true,
                stop_id: one_off.stop_id.to_owned(),
                days: vec![],
                date: Some(date),
                skip_holidays: false,
                paused_until: None,
                config: RuleConfig::Track(config),
            },
            now,
        )
        .await?;
    store
        .insert_track_run(rule.id, date, TRACKING, scheduled.to_utc(), &state, now)
        .await?;
    Ok((rule, true))
}

/// The departure of `trip_id` in a stop's list, cancelled or departed ones included.
pub fn find_departure<'a>(departures: &'a [Departure], trip_id: &str) -> Option<&'a Departure> {
    departures
        .iter()
        .find(|departure| departure.trip_id == trip_id)
}

// The live view (GET /api/trackings/{id})

/// The vehicle of a run as the live page shows it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct VehicleView {
    pub label: String,
    pub stops_away: Option<u32>,
    pub updated_at: Option<DateTime<Tz>>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EventView {
    pub kind: EventKind,
    pub minutes: Option<u32>,
    pub text: String,
    pub sent_at: DateTime<Tz>,
}

/// Everything of the detail JSON but the stop (the API adds it): the contract of step 6.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TrackingView {
    pub id: i64,
    pub recurring: bool,
    /// `waiting` (no run yet), or the run's status.
    pub phase: String,
    pub line: String,
    pub headsign: Option<String>,
    pub scheduled: Option<DateTime<Tz>>,
    pub expected: Option<DateTime<Tz>>,
    pub delay_seconds: Option<i64>,
    pub source: Option<Source>,
    pub checked_at: Option<DateTime<Tz>>,
    pub vehicle: Option<VehicleView>,
    pub events: Vec<EventView>,
    /// Thresholds not handled yet, longest first.
    pub planned_reminders: Vec<u32>,
    pub delay_alert_minutes: u32,
}

/// Builds the view of a track rule from its run (`None` before it starts). `gtt` gives the
/// vehicle; without it the vehicle is always `None`. `None` when `rule` is not a track rule.
pub fn view(
    rule: &Rule,
    run: Option<&TrackRun>,
    gtt: Option<&TripUpdates>,
    now: DateTime<Utc>,
) -> Option<TrackingView> {
    let RuleConfig::Track(config) = &rule.config else {
        return None;
    };
    let state = run.map(|run| &run.state);
    let phase = run.map_or("waiting", |run| run.status.as_str()).to_owned();
    let active = matches!(phase.as_str(), "waiting" | SEARCHING | TRACKING);

    let scheduled = state.and_then(|state| state.scheduled).or_else(|| {
        let schedule = rule.schedule();
        match run {
            Some(run) => match schedule.on(run.date) {
                Day::Due { at } => Some(at.to_utc()),
                _ => Some(run.due_at),
            },
            None => schedule.next(now).map(|at| at.to_utc()),
        }
    });

    let vehicle = state.filter(|_| phase == TRACKING).and_then(|state| {
        let trip = state.trip_id.as_deref()?.strip_prefix("gtt:")?;
        let vehicle = gtt?.vehicle(trip, state.service_date?)?;
        let stops_away = state
            .stop_sequence
            .and_then(|sequence| vehicle.stops_away(sequence, now));
        Some(VehicleView {
            label: vehicle.label,
            stops_away,
            updated_at: vehicle.updated_at.map(|at| at.with_timezone(&TIME_ZONE)),
        })
    });

    let mut planned: Vec<u32> = if active {
        config
            .alert_before_minutes
            .iter()
            .copied()
            .filter(|threshold| state.is_none_or(|state| !state.reminders_done.contains(threshold)))
            .collect()
    } else {
        Vec::new()
    };
    planned.sort_unstable_by(|a, b| b.cmp(a));

    Some(TrackingView {
        id: rule.id,
        recurring: rule.date.is_none(),
        phase,
        line: config.line.clone(),
        headsign: state
            .and_then(|state| state.headsign.as_deref())
            .map(display_name),
        scheduled: scheduled.map(|at| at.with_timezone(&TIME_ZONE)),
        expected: state
            .and_then(|state| state.expected)
            .map(|at| at.with_timezone(&TIME_ZONE)),
        delay_seconds: state.and_then(|state| state.delay_seconds),
        source: state.and_then(|state| state.source),
        checked_at: state
            .and_then(|state| state.checked_at)
            .map(|at| at.with_timezone(&TIME_ZONE)),
        vehicle,
        events: state
            .map(|state| {
                state
                    .events
                    .iter()
                    .map(|event| EventView {
                        kind: event.kind,
                        minutes: event.minutes,
                        text: event.text.clone(),
                        sent_at: event.sent_at.with_timezone(&TIME_ZONE),
                    })
                    .collect()
            })
            .unwrap_or_default(),
        planned_reminders: planned,
        delay_alert_minutes: config.delay_alert_minutes,
    })
}

#[cfg(test)]
mod tests;
