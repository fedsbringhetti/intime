use axum::{
    Json,
    extract::{
        Path, State,
        rejection::{JsonRejection, PathRejection},
    },
    http::StatusCode,
};
use chrono::{DateTime, NaiveDate, NaiveTime, TimeDelta, Utc, Weekday};
use chrono_tz::Tz;
use serde::{Deserialize, Deserializer, Serialize};

use super::{
    AppState,
    error::ApiError,
    favorites::clean_lines,
    stops::{check_range, resolve_stop_id},
};
use crate::{
    departures::FETCH_WINDOW,
    notify::SendReport,
    rules::{
        Kind, NewRule, Rule, RuleConfig, SummaryConfig, TrackConfig, hh_mm,
        schedule::{Day, local_date},
        weekdays,
    },
    scheduler::{SummaryError, send_and_log, store::RuleRun, summary_notification},
    stops::StopInfo,
};

/// How many runs `GET /api/rules/{id}/runs` returns.
const RUNS_LIMIT: i64 = 30;

const MAX_NAME_LEN: usize = 100;
const MAX_LINE_LEN: usize = 10;

// Defaults and limits of the summary (Phase 3, section 5). Defaults are stored in the rule
// when it is created (decision AG).
const DEFAULT_WINDOW_MINUTES: u32 = 30;
const MIN_WINDOW_MINUTES: u32 = 5;
const DEFAULT_MAX_DEPARTURES: u32 = 6;
const MAX_DEPARTURES: u32 = 20;

// Defaults and limits of tracking.
const DEFAULT_TOLERANCE_MINUTES: u32 = 2;
const MAX_TOLERANCE_MINUTES: u32 = 10;
const DEFAULT_WATCH_FROM_MINUTES: u32 = 30;
/// A run still missing 10 minutes before its time is "not found", so tracking starts earlier.
const MIN_WATCH_FROM_MINUTES: u32 = 10;
const MAX_WATCH_FROM_MINUTES: u32 = 120;
const DEFAULT_ALERT_BEFORE_MINUTES: [u32; 2] = [10, 5];
const MAX_ALERT_BEFORE_MINUTES: u32 = 60;
const MAX_REMINDERS: usize = 4;
const DEFAULT_DELAY_ALERT_MINUTES: u32 = 3;
const MAX_DELAY_ALERT_MINUTES: u32 = 30;

/// How far ahead a pause may last.
const MAX_PAUSE_DAYS: i64 = 365;

#[derive(Debug, Serialize)]
pub struct RulesResponse {
    rules: Vec<RuleView>,
}

#[derive(Debug, Serialize)]
pub struct RuleView {
    #[serde(flatten)]
    rule: Rule,
    /// `None` when the stop is not in the index.
    stop: Option<StopInfo>,
    /// The stop no longer exists after a sync.
    missing: bool,
    /// What the rule does today, in Europe/Rome (decision AJ).
    today: Today,
    /// The next occurrence; `None` when the rule will not run again.
    next: Option<DateTime<Tz>>,
}

/// What the rule does today, and how today's run went (`None`: no run yet).
#[derive(Debug, Serialize)]
struct Today {
    #[serde(flatten)]
    day: Day,
    run: Option<RunView>,
}

/// The run's status as the scheduler wrote it (`running`, `pending`, `sent`, `skipped`,
/// `failed`).
#[derive(Debug, Serialize)]
struct RunView {
    status: String,
    updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct RunsResponse {
    runs: Vec<RuleRun>,
}

#[derive(Debug, Serialize)]
pub struct RuleTestResponse {
    channel: &'static str,
    #[serde(flatten)]
    report: SendReport,
    title: String,
    body: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateRule {
    name: String,
    kind: Kind,
    /// GTT code (`39`) or full gtfsId (`gtt:39`).
    stop: String,
    #[serde(deserialize_with = "weekdays::deserialize")]
    days: Vec<Weekday>,
    #[serde(default = "yes")]
    skip_holidays: bool,
    #[serde(default = "yes")]
    enabled: bool,
    /// The fields of `kind`, read once the kind is known.
    config: serde_json::Value,
}

/// Fields to change; a missing field stays as it is. `config` is replaced whole.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateRule {
    name: Option<String>,
    /// Accepted only when unchanged.
    kind: Option<Kind>,
    stop: Option<String>,
    #[serde(default, deserialize_with = "some_weekdays")]
    days: Option<Vec<Weekday>>,
    skip_holidays: Option<bool>,
    enabled: Option<bool>,
    config: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PauseRule {
    /// The last day skipped, included; missing: the rule is turned off.
    until: Option<NaiveDate>,
}

/// The fields of a summary as sent: missing numbers get their defaults.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SummaryInput {
    #[serde(deserialize_with = "hh_mm::deserialize")]
    time: NaiveTime,
    #[serde(default)]
    lines: Vec<String>,
    window_minutes: Option<u32>,
    max_departures: Option<u32>,
}

/// The fields of a tracking rule as sent: missing numbers get their defaults.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TrackInput {
    line: String,
    #[serde(deserialize_with = "hh_mm::deserialize")]
    scheduled_time: NaiveTime,
    tolerance_minutes: Option<u32>,
    watch_from_minutes: Option<u32>,
    alert_before_minutes: Option<Vec<u32>>,
    delay_alert_minutes: Option<u32>,
}

fn yes() -> bool {
    true
}

fn some_weekdays<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Vec<Weekday>>, D::Error> {
    weekdays::deserialize(deserializer).map(Some)
}

pub async fn list(State(state): State<AppState>) -> Result<Json<RulesResponse>, ApiError> {
    let index_ready = state.stops.last_sync().await?.is_some();
    let mut rules = Vec::new();
    for rule in state.rules.list().await? {
        rules.push(view(&state, rule, index_ready).await?);
    }
    Ok(Json(RulesResponse { rules }))
}

pub async fn get(
    State(state): State<AppState>,
    id: Result<Path<i64>, PathRejection>,
) -> Result<Json<RuleView>, ApiError> {
    let Path(id) = id.map_err(|err| ApiError::InvalidRequest(err.body_text()))?;
    let rule = find(&state, id).await?;
    let index_ready = state.stops.last_sync().await?.is_some();
    Ok(Json(view(&state, rule, index_ready).await?))
}

pub async fn create(
    State(state): State<AppState>,
    body: Result<Json<CreateRule>, JsonRejection>,
) -> Result<(StatusCode, Json<RuleView>), ApiError> {
    let Json(body) = body.map_err(|err| ApiError::InvalidRequest(err.body_text()))?;
    let name = clean_name(body.name)?;
    let days = clean_days(body.days)?;
    let mut config = parse_config(body.kind, body.config)?;
    let stop = find_stop(&state, &body.stop).await?;
    check_lines(&mut config, &stop)?;

    let rule = state
        .rules
        .create(
            NewRule {
                name,
                enabled: body.enabled,
                stop_id: stop.id.clone(),
                days,
                date: None,
                skip_holidays: body.skip_holidays,
                paused_until: None,
                config,
            },
            state.clock.now(),
        )
        .await?;
    Ok((StatusCode::CREATED, Json(view(&state, rule, true).await?)))
}

pub async fn update(
    State(state): State<AppState>,
    id: Result<Path<i64>, PathRejection>,
    body: Result<Json<UpdateRule>, JsonRejection>,
) -> Result<Json<RuleView>, ApiError> {
    let Path(id) = id.map_err(|err| ApiError::InvalidRequest(err.body_text()))?;
    let Json(body) = body.map_err(|err| ApiError::InvalidRequest(err.body_text()))?;
    let mut rule = find(&state, id).await?;
    let kind = rule.config.kind();
    if body.kind.is_some_and(|other| other != kind) {
        return Err(ApiError::InvalidRequest(
            "kind cannot change: create a new rule".to_owned(),
        ));
    }

    if let Some(name) = body.name {
        rule.name = clean_name(name)?;
    }
    if let Some(days) = body.days {
        rule.days = clean_days(days)?;
    }
    if let Some(skip_holidays) = body.skip_holidays {
        rule.skip_holidays = skip_holidays;
    }
    if let Some(enabled) = body.enabled {
        rule.enabled = enabled;
    }
    // The lines are checked against the stop only when one of the two changes: a rule whose
    // stop lost a line can still be renamed.
    let check_stop = body.stop.is_some() || body.config.is_some();
    if let Some(config) = body.config {
        rule.config = parse_config(kind, config)?;
    }
    if check_stop {
        let stop = find_stop(&state, body.stop.as_deref().unwrap_or(&rule.stop_id)).await?;
        check_lines(&mut rule.config, &stop)?;
        rule.stop_id = stop.id;
    }

    save(&state, rule).await
}

pub async fn delete(
    State(state): State<AppState>,
    id: Result<Path<i64>, PathRejection>,
) -> Result<StatusCode, ApiError> {
    let Path(id) = id.map_err(|err| ApiError::InvalidRequest(err.body_text()))?;
    if state.rules.delete(id).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::RuleNotFound(id))
    }
}

/// Pauses until a date, included ("salta oggi" is today); without one, turns the rule off
/// (decision AH). The body is optional.
pub async fn pause(
    State(state): State<AppState>,
    id: Result<Path<i64>, PathRejection>,
    body: Result<Option<Json<PauseRule>>, JsonRejection>,
) -> Result<Json<RuleView>, ApiError> {
    let Path(id) = id.map_err(|err| ApiError::InvalidRequest(err.body_text()))?;
    let body = body.map_err(|err| ApiError::InvalidRequest(err.body_text()))?;
    let mut rule = find(&state, id).await?;

    match body.and_then(|Json(body)| body.until) {
        Some(until) => {
            let today = local_date(state.clock.now());
            let last = today + TimeDelta::days(MAX_PAUSE_DAYS);
            if until < today || until > last {
                return Err(ApiError::InvalidRequest(format!(
                    "until must be between {today} and {last}"
                )));
            }
            rule.enabled = true;
            rule.paused_until = Some(until);
        }
        None => {
            rule.enabled = false;
            rule.paused_until = None;
        }
    }
    save(&state, rule).await
}

/// Turns the rule on and ends its pause.
pub async fn resume(
    State(state): State<AppState>,
    id: Result<Path<i64>, PathRejection>,
) -> Result<Json<RuleView>, ApiError> {
    let Path(id) = id.map_err(|err| ApiError::InvalidRequest(err.body_text()))?;
    let mut rule = find(&state, id).await?;
    rule.enabled = true;
    rule.paused_until = None;
    save(&state, rule).await
}

/// Sends the summary now, whatever its days, pause or `enabled` (decision 11): it is logged in
/// the notifications but writes no run. Tracking rules answer 422.
pub async fn test(
    State(state): State<AppState>,
    id: Result<Path<i64>, PathRejection>,
) -> Result<Json<RuleTestResponse>, ApiError> {
    let Path(id) = id.map_err(|err| ApiError::InvalidRequest(err.body_text()))?;
    let rule = find(&state, id).await?;
    let now = state.clock.now();
    let notification = summary_notification(state.departures.as_ref(), &state.stops, &rule, now)
        .await
        .map_err(|err| match err {
            SummaryError::NotSummary => ApiError::NotSummary(id),
            SummaryError::Upstream(err) => err.into(),
        })?;
    let (notification, report) = send_and_log(
        state.notifier.as_ref(),
        &state.scheduler,
        Some(id),
        "summary",
        notification,
        now,
    )
    .await?;
    Ok(Json(RuleTestResponse {
        channel: state.notifier.channel(),
        report,
        title: notification.title,
        body: notification.body,
    }))
}

/// The last runs of the rule, newest day first.
pub async fn runs(
    State(state): State<AppState>,
    id: Result<Path<i64>, PathRejection>,
) -> Result<Json<RunsResponse>, ApiError> {
    let Path(id) = id.map_err(|err| ApiError::InvalidRequest(err.body_text()))?;
    find(&state, id).await?;
    Ok(Json(RunsResponse {
        runs: state.scheduler.recent_runs(id, RUNS_LIMIT).await?,
    }))
}

async fn find(state: &AppState, id: i64) -> Result<Rule, ApiError> {
    state.rules.get(id).await?.ok_or(ApiError::RuleNotFound(id))
}

async fn save(state: &AppState, rule: Rule) -> Result<Json<RuleView>, ApiError> {
    let id = rule.id;
    let rule = state
        .rules
        .update(&rule, state.clock.now())
        .await?
        .ok_or(ApiError::RuleNotFound(id))?;
    let index_ready = state.stops.last_sync().await?.is_some();
    Ok(Json(view(state, rule, index_ready).await?))
}

async fn view(state: &AppState, rule: Rule, index_ready: bool) -> Result<RuleView, ApiError> {
    let stop = state.stops.get(&rule.stop_id).await?;
    let now = state.clock.now();
    let schedule = rule.schedule();
    let date = local_date(now);
    let run = state
        .scheduler
        .get_run(rule.id, date)
        .await?
        .map(|run| RunView {
            status: run.status,
            updated_at: run.updated_at,
        });
    Ok(RuleView {
        today: Today {
            day: schedule.on(date),
            run,
        },
        next: schedule.next(now),
        // Before the first sync every stop would look missing.
        missing: index_ready && stop.is_none(),
        stop,
        rule,
    })
}

/// The stop of a rule, which the index must know: a typo would make a rule that never works.
async fn find_stop(state: &AppState, input: &str) -> Result<StopInfo, ApiError> {
    let stop_id = resolve_stop_id(input)
        .ok_or_else(|| ApiError::InvalidRequest(format!("Invalid stop {input:?}")))?;
    if state.stops.last_sync().await?.is_none() {
        return Err(ApiError::IndexNotReady);
    }
    state
        .stops
        .get(&stop_id)
        .await?
        .ok_or(ApiError::StopNotFound(stop_id))
}

/// Trimmed, not empty.
fn clean_name(name: String) -> Result<String, ApiError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(ApiError::InvalidRequest(
            "name must not be empty".to_owned(),
        ));
    }
    if name.chars().count() > MAX_NAME_LEN {
        return Err(ApiError::InvalidRequest(format!(
            "name must be at most {MAX_NAME_LEN} characters"
        )));
    }
    Ok(name.to_owned())
}

/// Monday first, each once, at least one.
fn clean_days(mut days: Vec<Weekday>) -> Result<Vec<Weekday>, ApiError> {
    days.sort_by_key(Weekday::num_days_from_monday);
    days.dedup();
    if days.is_empty() {
        return Err(ApiError::InvalidRequest(
            "days must list at least one day".to_owned(),
        ));
    }
    Ok(days)
}

/// The fields of `kind`, checked, with defaults for the missing numbers.
fn parse_config(kind: Kind, config: serde_json::Value) -> Result<RuleConfig, ApiError> {
    let invalid = |err: serde_json::Error| ApiError::InvalidRequest(format!("config: {err}"));
    match kind {
        Kind::Summary => {
            let input: SummaryInput = serde_json::from_value(config).map_err(invalid)?;
            let max_window = (FETCH_WINDOW.as_secs() / 60) as u32;
            Ok(RuleConfig::Summary(SummaryConfig {
                time: input.time,
                lines: clean_lines(input.lines)?,
                window_minutes: check_range(
                    "config.window_minutes",
                    input.window_minutes.unwrap_or(DEFAULT_WINDOW_MINUTES),
                    MIN_WINDOW_MINUTES,
                    max_window,
                )?,
                max_departures: check_range(
                    "config.max_departures",
                    input.max_departures.unwrap_or(DEFAULT_MAX_DEPARTURES),
                    1,
                    MAX_DEPARTURES,
                )?,
            }))
        }
        Kind::Track => {
            let input: TrackInput = serde_json::from_value(config).map_err(invalid)?;
            let line = input.line.trim().to_owned();
            if line.is_empty() || line.chars().count() > MAX_LINE_LEN {
                return Err(ApiError::InvalidRequest(format!(
                    "config.line must have 1 to {MAX_LINE_LEN} characters"
                )));
            }
            let reminders = clean_reminders(
                input
                    .alert_before_minutes
                    .unwrap_or(DEFAULT_ALERT_BEFORE_MINUTES.to_vec()),
            )?;
            let watch_from_minutes = check_range(
                "config.watch_from_minutes",
                input
                    .watch_from_minutes
                    .unwrap_or(DEFAULT_WATCH_FROM_MINUTES),
                MIN_WATCH_FROM_MINUTES,
                MAX_WATCH_FROM_MINUTES,
            )?;
            // A reminder before tracking starts could never be sent.
            if reminders
                .first()
                .is_some_and(|longest| *longest > watch_from_minutes)
            {
                return Err(ApiError::InvalidRequest(
                    "config.watch_from_minutes must be at least the longest reminder".to_owned(),
                ));
            }
            Ok(RuleConfig::Track(TrackConfig {
                line,
                scheduled_time: input.scheduled_time,
                tolerance_minutes: check_range(
                    "config.tolerance_minutes",
                    input.tolerance_minutes.unwrap_or(DEFAULT_TOLERANCE_MINUTES),
                    0,
                    MAX_TOLERANCE_MINUTES,
                )?,
                watch_from_minutes,
                alert_before_minutes: reminders,
                delay_alert_minutes: check_range(
                    "config.delay_alert_minutes",
                    input
                        .delay_alert_minutes
                        .unwrap_or(DEFAULT_DELAY_ALERT_MINUTES),
                    1,
                    MAX_DELAY_ALERT_MINUTES,
                )?,
            }))
        }
    }
}

/// Longest first, each once; none is allowed (only delay and cancellation alerts).
fn clean_reminders(mut minutes: Vec<u32>) -> Result<Vec<u32>, ApiError> {
    for minute in &minutes {
        check_range(
            "config.alert_before_minutes",
            *minute,
            1,
            MAX_ALERT_BEFORE_MINUTES,
        )?;
    }
    minutes.sort_unstable_by(|a, b| b.cmp(a));
    minutes.dedup();
    if minutes.len() > MAX_REMINDERS {
        return Err(ApiError::InvalidRequest(format!(
            "config.alert_before_minutes: at most {MAX_REMINDERS} reminders"
        )));
    }
    Ok(minutes)
}

/// Every line of the rule must serve the stop; each takes the index's spelling (`9b` → `9B`).
fn check_lines(config: &mut RuleConfig, stop: &StopInfo) -> Result<(), ApiError> {
    let lines: Vec<&mut String> = match config {
        RuleConfig::Summary(summary) => summary.lines.iter_mut().collect(),
        RuleConfig::Track(track) => vec![&mut track.line],
    };
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
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn summary_defaults_are_filled_in() {
        let config = parse_config(Kind::Summary, json!({"time": "07:40"})).unwrap();
        assert_eq!(
            serde_json::to_value(config).unwrap()["config"],
            json!({"time": "07:40", "lines": [], "window_minutes": 30, "max_departures": 6})
        );
    }

    #[test]
    fn track_defaults_are_filled_in() {
        let config = parse_config(
            Kind::Track,
            json!({"line": " 67 ", "scheduled_time": "08:05"}),
        )
        .unwrap();
        assert_eq!(
            serde_json::to_value(config).unwrap()["config"],
            json!({
                "line": "67", "scheduled_time": "08:05", "tolerance_minutes": 2,
                "watch_from_minutes": 30, "alert_before_minutes": [10, 5], "delay_alert_minutes": 3
            })
        );
    }

    #[test]
    fn config_is_checked() {
        let summary = |config| parse_config(Kind::Summary, config);
        let track = |config| parse_config(Kind::Track, config);
        // Fields of the other kind, unknown fields, bad times.
        assert!(summary(json!({"line": "4", "scheduled_time": "08:05"})).is_err());
        assert!(summary(json!({"time": "07:40", "color": "red"})).is_err());
        assert!(summary(json!({"time": "7:40"})).is_err());
        assert!(summary(json!({"time": "07:40", "window_minutes": 4})).is_err());
        assert!(summary(json!({"time": "07:40", "window_minutes": 121})).is_err());
        assert!(summary(json!({"time": "07:40", "max_departures": 0})).is_err());
        let base = json!({"line": "4", "scheduled_time": "08:05"});
        let with = |field: &str, value| {
            let mut config = base.clone();
            config[field] = value;
            track(config)
        };
        assert!(with("line", json!(" ")).is_err());
        assert!(with("tolerance_minutes", json!(11)).is_err());
        assert!(with("watch_from_minutes", json!(9)).is_err());
        assert!(with("alert_before_minutes", json!([0])).is_err());
        assert!(with("alert_before_minutes", json!([1, 2, 3, 4, 5])).is_err());
        assert!(with("delay_alert_minutes", json!(0)).is_err());
        // A reminder longer than the watch window could never be sent.
        assert!(with("alert_before_minutes", json!([45])).is_err());
        assert!(with("alert_before_minutes", json!([])).is_ok());
    }

    #[test]
    fn reminders_longest_first() {
        assert_eq!(clean_reminders(vec![5, 15, 10, 5]).unwrap(), [15, 10, 5]);
    }

    #[test]
    fn days_monday_first() {
        use Weekday::*;
        assert_eq!(
            clean_days(vec![Sun, Mon, Sun, Wed]).unwrap(),
            [Mon, Wed, Sun]
        );
        assert!(clean_days(Vec::new()).is_err());
    }

    #[test]
    fn names_are_trimmed() {
        assert_eq!(
            clean_name(" Casa → lavoro ".into()).unwrap(),
            "Casa → lavoro"
        );
        assert!(clean_name("  ".into()).is_err());
        assert!(clean_name("x".repeat(101)).is_err());
    }
}
