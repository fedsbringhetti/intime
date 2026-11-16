//! Trip tracking (Phase 3, step 6): one-off trackings of a single departure, and the live view
//! of every tracking rule, one-off or recurring.

use std::collections::HashMap;

use axum::{
    Json,
    extract::{
        Path, State,
        rejection::{JsonRejection, PathRejection},
    },
    http::StatusCode,
};
use chrono::{DateTime, TimeDelta};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};

use super::{
    AppState,
    error::ApiError,
    rules::{
        DEFAULT_ALERT_BEFORE_MINUTES, DEFAULT_DELAY_ALERT_MINUTES, MAX_DELAY_ALERT_MINUTES,
        clean_reminders, find_stop,
    },
    stops::check_range,
};
use crate::{
    departures::Source,
    display_name::display_name,
    rules::{
        Rule, RuleConfig,
        schedule::{Day, local_date},
    },
    scheduler::{
        store::TrackRun,
        tracking::{
            OneOff, SEARCHING, TRACKING, TrackingView, create_one_off, find_departure, view,
        },
    },
};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateTracking {
    /// GTT code (`39`) or full gtfsId (`gtt:39`).
    stop: String,
    /// The run's gtfsId (`gtt:28459721U`), as in the departures of the stop.
    trip_id: String,
    alert_before_minutes: Option<Vec<u32>>,
    delay_alert_minutes: Option<u32>,
}

#[derive(Debug, Serialize)]
pub struct TrackingsResponse {
    trackings: Vec<Summary>,
}

#[derive(Debug, Serialize)]
struct StopView {
    id: String,
    code: String,
    name: String,
}

/// A tracking being searched for or followed, for the list.
#[derive(Debug, Serialize)]
struct Summary {
    id: i64,
    line: String,
    headsign: Option<String>,
    stop: StopView,
    scheduled: Option<DateTime<Tz>>,
    expected: Option<DateTime<Tz>>,
    delay_seconds: Option<i64>,
    source: Option<Source>,
    phase: String,
    recurring: bool,
}

#[derive(Debug, Serialize)]
pub struct Detail {
    #[serde(flatten)]
    view: TrackingView,
    stop: StopView,
}

pub async fn create(
    State(state): State<AppState>,
    body: Result<Json<CreateTracking>, JsonRejection>,
) -> Result<(StatusCode, Json<Detail>), ApiError> {
    let Json(body) = body.map_err(|err| ApiError::InvalidRequest(err.body_text()))?;
    let reminders = clean_reminders(
        body.alert_before_minutes
            .unwrap_or(DEFAULT_ALERT_BEFORE_MINUTES.to_vec()),
    )?;
    let delay_alert_minutes = check_range(
        "delay_alert_minutes",
        body.delay_alert_minutes
            .unwrap_or(DEFAULT_DELAY_ALERT_MINUTES),
        1,
        MAX_DELAY_ALERT_MINUTES,
    )?;
    let stop = find_stop(&state, &body.stop).await?;

    let fetched = state.departures.get(&stop.id).await?;
    let departure = find_departure(&fetched.data.departures, &body.trip_id)
        .ok_or_else(|| ApiError::TripNotFound(body.trip_id.clone()))?;
    let now = state.clock.now();
    if departure.canceled {
        return Err(ApiError::TripNotTrackable(
            "the run is cancelled".to_owned(),
        ));
    }
    if departure.expected <= now {
        return Err(ApiError::TripNotTrackable(
            "the run has already departed".to_owned(),
        ));
    }

    let (rule, created) = create_one_off(
        &state.rules,
        &state.scheduler,
        OneOff {
            stop_id: &stop.id,
            departure,
            alert_before_minutes: reminders,
            delay_alert_minutes,
        },
        now,
    )
    .await?;
    let status = if created {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    Ok((status, Json(detail(&state, &rule).await?)))
}

/// Trackings running now (`searching` or `tracking`, today's and yesterday's runs: one after
/// midnight belongs to the day before), soonest first.
pub async fn list(State(state): State<AppState>) -> Result<Json<TrackingsResponse>, ApiError> {
    let now = state.clock.now();
    let today = local_date(now);
    let runs = state
        .scheduler
        .active_track_runs(today - TimeDelta::days(1), today + TimeDelta::days(1))
        .await?;
    let rules: HashMap<i64, Rule> = state
        .rules
        .list()
        .await?
        .into_iter()
        .map(|rule| (rule.id, rule))
        .collect();

    let mut trackings = Vec::new();
    for run in &runs {
        let Some(rule) = rules.get(&run.rule_id) else {
            continue;
        };
        let Some(tracking) = view(rule, Some(run), None, now) else {
            continue;
        };
        if tracking.phase != SEARCHING && tracking.phase != TRACKING {
            continue;
        }
        trackings.push(Summary {
            id: tracking.id,
            line: tracking.line,
            headsign: tracking.headsign,
            stop: stop_view(&state, &rule.stop_id).await?,
            scheduled: tracking.scheduled,
            expected: tracking.expected,
            delay_seconds: tracking.delay_seconds,
            source: tracking.source,
            phase: tracking.phase,
            recurring: tracking.recurring,
        });
    }
    trackings.sort_by_key(|tracking| tracking.expected.or(tracking.scheduled));
    Ok(Json(TrackingsResponse { trackings }))
}

pub async fn get(
    State(state): State<AppState>,
    id: Result<Path<i64>, PathRejection>,
) -> Result<Json<Detail>, ApiError> {
    let Path(id) = id.map_err(|err| ApiError::InvalidRequest(err.body_text()))?;
    let rule = find_tracking(&state, id).await?;
    Ok(Json(detail(&state, &rule).await?))
}

/// One-off: the rule is deleted. Recurring: only today's run is stopped (decision BA), the
/// rule goes on tomorrow.
pub async fn delete(
    State(state): State<AppState>,
    id: Result<Path<i64>, PathRejection>,
) -> Result<StatusCode, ApiError> {
    let Path(id) = id.map_err(|err| ApiError::InvalidRequest(err.body_text()))?;
    let rule = find_tracking(&state, id).await?;
    let now = state.clock.now();
    if rule.date.is_some() {
        state.rules.delete(id).await?;
    } else {
        match active_run(&state, &rule, now).await? {
            // A run still going (yesterday's after midnight, or tomorrow's started early).
            Some(run) => {
                state
                    .scheduler
                    .stop_track_run(id, run.date, run.due_at, now)
                    .await?;
            }
            None => {
                let today = local_date(now);
                let due_at = match rule.schedule().on(today) {
                    Day::Due { at } => at.to_utc(),
                    _ => now,
                };
                state
                    .scheduler
                    .stop_track_run(id, today, due_at, now)
                    .await?;
            }
        }
    }
    Ok(StatusCode::NO_CONTENT)
}

/// A rule that exists and tracks.
async fn find_tracking(state: &AppState, id: i64) -> Result<Rule, ApiError> {
    state
        .rules
        .get(id)
        .await?
        .filter(|rule| matches!(rule.config, RuleConfig::Track(_)))
        .ok_or(ApiError::TrackingNotFound(id))
}

/// The run of a recurring rule that is going now (`searching` or `tracking`), looking at
/// yesterday, today and tomorrow in this order.
async fn active_run(
    state: &AppState,
    rule: &Rule,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<Option<TrackRun>, ApiError> {
    let today = local_date(now);
    let dates = [
        today - TimeDelta::days(1),
        today,
        today + TimeDelta::days(1),
    ];
    for date in dates {
        if let Some(run) = state.scheduler.get_track_run(rule.id, date).await?
            && (run.status == SEARCHING || run.status == TRACKING)
        {
            return Ok(Some(run));
        }
    }
    Ok(None)
}

async fn detail(state: &AppState, rule: &Rule) -> Result<Detail, ApiError> {
    let now = state.clock.now();
    // A one-off has its own day; a recurring rule is shown with its active run (yesterday's,
    // today's or tomorrow's), else as of today.
    let run: Option<TrackRun> = match rule.date {
        Some(date) => state.scheduler.get_track_run(rule.id, date).await?,
        None => match active_run(state, rule, now).await? {
            Some(run) => Some(run),
            None => {
                state
                    .scheduler
                    .get_track_run(rule.id, local_date(now))
                    .await?
            }
        },
    };
    let updates = match &state.gtt {
        Some(gtt) => gtt.trip_updates().await,
        None => None,
    };
    let view = view(rule, run.as_ref(), updates.as_deref(), now).ok_or(ApiError::Internal)?;
    Ok(Detail {
        view,
        stop: stop_view(state, &rule.stop_id).await?,
    })
}

async fn stop_view(state: &AppState, stop_id: &str) -> Result<StopView, ApiError> {
    Ok(match state.stops.get(stop_id).await? {
        Some(stop) => StopView {
            id: stop.id,
            code: stop.code,
            name: display_name(&stop.name),
        },
        None => StopView {
            id: stop_id.to_owned(),
            code: stop_id.rsplit(':').next().unwrap_or(stop_id).to_owned(),
            name: stop_id.to_owned(),
        },
    })
}
