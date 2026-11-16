use std::{sync::Arc, time::Duration};

use chrono::{DateTime, NaiveDate, TimeDelta, Utc};
use chrono_tz::Tz;
use moka::future::Cache;
use serde::Serialize;

use crate::{
    gtt::{
        GttRealtime,
        comparison::{Comparisons, Outcome},
        trip_updates::{Prediction, TripUpdates},
    },
    stops::display_code,
    time::{Clock, TIME_ZONE, service_date, service_time},
    upstream::{UpstreamClient, UpstreamError, types},
};

/// Departures are fetched for this window and cached; each request filters a part of it.
pub const FETCH_WINDOW: Duration = Duration::from_secs(2 * 60 * 60);
const FETCH_MAX_DEPARTURES: u32 = 200;
const CACHE_CAPACITY: u64 = 1_000;
/// How long the last good response for a stop can stand in when the upstream fails.
const STALE_MAX_AGE: Duration = Duration::from_secs(2 * 60);

#[derive(Debug, Clone, Serialize)]
pub struct StopRef {
    pub id: String,
    pub code: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Departure {
    pub line: String,
    pub mode: Option<String>,
    pub headsign: Option<String>,
    pub scheduled: DateTime<Tz>,
    pub expected: DateTime<Tz>,
    /// Negative when early.
    pub delay_seconds: i64,
    pub realtime: bool,
    pub canceled: bool,
    pub trip_id: String,
    /// Where `expected` comes from.
    pub source: Source,
    /// Position of the stop in the run, to find GTT's prediction for it.
    #[serde(skip)]
    pub stop_sequence: Option<u32>,
    /// Date the run belongs to (a run after midnight belongs to the day before).
    #[serde(skip)]
    pub service_date: Option<NaiveDate>,
}

/// Where a departure's expected time comes from (decision N of Phase 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    /// GTT's own trip updates (urban runs).
    Gtt,
    /// OpenTripPlanner's real time.
    Otp,
    /// No real-time data: the timetable.
    Scheduled,
}

#[derive(Debug)]
pub struct StopDepartures {
    pub stop: StopRef,
    /// When the data was received from the upstream.
    pub fetched_at: DateTime<Utc>,
    /// Sorted by expected time.
    pub departures: Vec<Departure>,
}

#[derive(Debug, Clone)]
pub struct Filter {
    /// Empty means all lines.
    pub lines: Vec<String>,
    pub window: TimeDelta,
    pub limit: usize,
}

impl StopDepartures {
    pub fn from_upstream(stop: types::Stop, fetched_at: DateTime<Utc>) -> Self {
        let code = display_code(stop.code, &stop.gtfs_id);
        let mut departures: Vec<_> = stop
            .stoptimes_without_patterns
            .into_iter()
            .filter_map(Departure::from_upstream)
            .collect();
        departures.sort_by_key(|departure| departure.expected);

        Self {
            stop: StopRef {
                id: stop.gtfs_id,
                code,
                name: stop.name,
            },
            fetched_at,
            departures,
        }
    }

    /// Departures expected from `now` to `now + filter.window`, on the requested lines.
    pub fn select(&self, filter: &Filter, now: DateTime<Utc>) -> Vec<&Departure> {
        let now = now.with_timezone(&TIME_ZONE);
        let until = now + filter.window;
        self.departures
            .iter()
            .filter(|departure| departure.expected >= now && departure.expected <= until)
            .filter(|departure| {
                filter.lines.is_empty()
                    || filter
                        .lines
                        .iter()
                        .any(|line| line.eq_ignore_ascii_case(&departure.line))
            })
            .take(filter.limit)
            .collect()
    }

    /// A copy with GTT's predictions applied (merge rules, Phase 3 plan section 4), sorted
    /// again by expected time. Each outcome is recorded in `comparisons`.
    pub fn with_gtt(
        &self,
        gtt: &TripUpdates,
        comparisons: &Comparisons,
        now: DateTime<Utc>,
    ) -> Self {
        let today = now.with_timezone(&TIME_ZONE).date_naive();
        let mut departures: Vec<_> = self
            .departures
            .iter()
            .map(|departure| {
                let (merged, outcome) = departure.with_gtt(gtt, now);
                if let Some(outcome) = outcome {
                    comparisons.record(today, &departure.trip_id, &self.stop.id, outcome);
                }
                merged
            })
            .collect();
        departures.sort_by_key(|departure| departure.expected);
        Self {
            stop: self.stop.clone(),
            fetched_at: self.fetched_at,
            departures,
        }
    }
}

impl Departure {
    fn from_upstream(stoptime: types::Stoptime) -> Option<Self> {
        Some(Self {
            scheduled: service_time(stoptime.service_day, stoptime.scheduled_departure)?,
            expected: service_time(stoptime.service_day, stoptime.realtime_departure)?,
            delay_seconds: stoptime.realtime_departure - stoptime.scheduled_departure,
            realtime: stoptime.realtime,
            source: if stoptime.realtime {
                Source::Otp
            } else {
                Source::Scheduled
            },
            stop_sequence: stoptime.stop_sequence,
            service_date: service_date(stoptime.service_day),
            canceled: stoptime.realtime_state.as_deref() == Some("CANCELED"),
            line: stoptime.trip.route.short_name.unwrap_or_default(),
            mode: stoptime.trip.route.mode,
            headsign: stoptime.headsign.or(stoptime.trip.trip_headsign),
            trip_id: stoptime.trip.gtfs_id,
        })
    }

    /// This departure with GTT's prediction, when it has a usable one, and what happened to
    /// that prediction (`None` when GTT had nothing to say). A cancellation from either
    /// source stays.
    fn with_gtt(&self, gtt: &TripUpdates, now: DateTime<Utc>) -> (Self, Option<Outcome>) {
        let (Some(trip), Some(sequence), Some(date)) = (
            self.trip_id.strip_prefix("gtt:"),
            self.stop_sequence,
            self.service_date,
        ) else {
            return (self.clone(), None);
        };
        match gtt.prediction(trip, date, sequence, self.scheduled.to_utc(), now) {
            Prediction::Expected(expected) => {
                let expected = expected.with_timezone(&TIME_ZONE);
                let difference = (self.source == Source::Otp).then(|| expected - self.expected);
                if let Some(difference) = difference {
                    tracing::debug!(
                        "{} at sequence {sequence}: GTT and OpenTripPlanner differ by {}s",
                        self.trip_id,
                        difference.num_seconds()
                    );
                }
                let merged = Self {
                    expected,
                    delay_seconds: (expected - self.scheduled).num_seconds(),
                    realtime: true,
                    source: Source::Gtt,
                    ..self.clone()
                };
                (merged, Some(Outcome::Used { difference }))
            }
            Prediction::Canceled => {
                let merged = Self {
                    canceled: true,
                    realtime: true,
                    source: Source::Gtt,
                    ..self.clone()
                };
                (merged, None)
            }
            Prediction::Implausible(expected) => {
                tracing::debug!(
                    "{} at sequence {sequence}: implausible GTT time {expected}",
                    self.trip_id
                );
                (self.clone(), Some(Outcome::Mismatch))
            }
            Prediction::None => (self.clone(), None),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum DeparturesError {
    #[error("stop {0} not found")]
    StopNotFound(String),
    #[error(transparent)]
    Upstream(#[from] UpstreamError),
}

/// Departures as served to a request.
#[derive(Debug, Clone)]
pub struct Fetched {
    pub data: Arc<StopDepartures>,
    /// The upstream failed and `data` is the last good response, at most `STALE_MAX_AGE` old.
    pub stale: bool,
}

pub struct DeparturesService {
    upstream: UpstreamClient,
    /// GTT's predictions for urban runs; `None` when disabled.
    gtt: Option<Arc<GttRealtime>>,
    clock: Arc<dyn Clock>,
    /// Fresh responses, reused for `cache_ttl`.
    cache: Cache<String, Arc<StopDepartures>>,
    /// Last good response per stop, used only when the upstream fails.
    last_good: Cache<String, Arc<StopDepartures>>,
}

impl DeparturesService {
    pub fn new(
        upstream: UpstreamClient,
        gtt: Option<Arc<GttRealtime>>,
        cache_ttl: Duration,
        clock: Arc<dyn Clock>,
    ) -> Self {
        let cache = Cache::builder()
            .max_capacity(CACHE_CAPACITY)
            .time_to_live(cache_ttl)
            .build();
        let last_good = Cache::builder()
            .max_capacity(CACHE_CAPACITY)
            .time_to_live(STALE_MAX_AGE)
            .build();
        Self {
            upstream,
            gtt,
            clock,
            cache,
            last_good,
        }
    }

    /// Departures for a stop, with GTT's predictions merged in when GTT can be read. The two
    /// sources are read in parallel, each from its own cache, and merged at every request
    /// (decision AB), so a source that fails does not spoil the other's data.
    pub async fn get(&self, stop_id: &str) -> Result<Fetched, Arc<DeparturesError>> {
        let gtt = async {
            match &self.gtt {
                Some(gtt) => gtt.trip_updates().await.map(|updates| (gtt, updates)),
                None => None,
            }
        };
        let (fetched, gtt) = tokio::join!(self.get_otp(stop_id), gtt);
        let fetched = fetched?;
        Ok(match gtt {
            Some((gtt, updates)) => Fetched {
                data: Arc::new(
                    fetched
                        .data
                        .with_gtt(&updates, &gtt.comparisons, self.clock.now()),
                ),
                stale: fetched.stale,
            },
            None => fetched,
        })
    }

    /// OpenTripPlanner's departures for a stop, from the cache when fresh. Concurrent requests
    /// for the same stop share a single upstream call; errors are not cached. When the upstream
    /// fails, the last good response is returned as stale if it is recent enough.
    async fn get_otp(&self, stop_id: &str) -> Result<Fetched, Arc<DeparturesError>> {
        let result = self
            .cache
            .try_get_with(stop_id.to_owned(), async {
                let stop = self
                    .upstream
                    .stop_departures(stop_id, FETCH_WINDOW, FETCH_MAX_DEPARTURES)
                    .await?
                    .ok_or_else(|| DeparturesError::StopNotFound(stop_id.to_owned()))?;
                let data = Arc::new(StopDepartures::from_upstream(stop, self.clock.now()));
                self.last_good
                    .insert(stop_id.to_owned(), data.clone())
                    .await;
                Ok::<_, DeparturesError>(data)
            })
            .await;

        match result {
            Ok(data) => Ok(Fetched { data, stale: false }),
            Err(err) if matches!(*err, DeparturesError::Upstream(_)) => {
                match self.last_good.get(stop_id).await {
                    Some(data) => {
                        tracing::warn!("serving stale departures for {stop_id}: {err}");
                        Ok(Fetched { data, stale: true })
                    }
                    None => Err(err),
                }
            }
            Err(err) => Err(err),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const SERVICE_DAY: i64 = 1_793_228_400; // 2026-10-29

    fn stoptime(line: &str, scheduled: i64, expected: i64, state: &str) -> serde_json::Value {
        json!({
            "scheduledDeparture": scheduled,
            "realtimeDeparture": expected,
            "realtime": state == "UPDATED",
            "realtimeState": state,
            "serviceDay": SERVICE_DAY,
            "stopSequence": 12,
            "headsign": null,
            "trip": {
                "gtfsId": format!("gtt:{line}-{scheduled}"),
                "tripHeadsign": "CAPOLINEA",
                "route": { "shortName": line, "mode": "BUS" }
            }
        })
    }

    fn stop(code: Option<&str>, stoptimes: Vec<serde_json::Value>) -> StopDepartures {
        let stop: types::Stop = serde_json::from_value(json!({
            "gtfsId": "gtt:8224",
            "code": code,
            "name": "METRO PORTA NUOVA",
            "stoptimesWithoutPatterns": stoptimes,
        }))
        .unwrap();
        StopDepartures::from_upstream(stop, at(8, 0))
    }

    fn at(hours: i64, minutes: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(SERVICE_DAY + hours * 3600 + minutes * 60, 0).unwrap()
    }

    fn filter(lines: &[&str], window_minutes: i64, limit: usize) -> Filter {
        Filter {
            lines: lines.iter().map(|line| line.to_string()).collect(),
            window: TimeDelta::minutes(window_minutes),
            limit,
        }
    }

    #[test]
    fn falls_back_to_code_from_id() {
        assert_eq!(stop(None, vec![]).stop.code, "8224");
        assert_eq!(stop(Some(""), vec![]).stop.code, "8224");
        assert_eq!(stop(Some("39"), vec![]).stop.code, "39");
    }

    #[test]
    fn maps_departure_fields() {
        let data = stop(None, vec![stoptime("9B", 28_800, 28_980, "UPDATED")]);
        let departure = &data.departures[0];
        assert_eq!(departure.line, "9B");
        assert_eq!(departure.headsign.as_deref(), Some("CAPOLINEA"));
        assert_eq!(
            departure.scheduled.to_rfc3339(),
            "2026-10-29T08:00:00+01:00"
        );
        assert_eq!(departure.expected.to_rfc3339(), "2026-10-29T08:03:00+01:00");
        assert_eq!(departure.delay_seconds, 180);
        assert!(departure.realtime);
        assert!(!departure.canceled);
        assert_eq!(departure.source, Source::Otp);
        assert_eq!(departure.stop_sequence, Some(12));
        assert_eq!(
            departure.service_date,
            NaiveDate::from_ymd_opt(2026, 10, 29)
        );
    }

    #[test]
    fn departures_without_real_time_are_scheduled() {
        let data = stop(None, vec![stoptime("4", 28_800, 28_800, "SCHEDULED")]);
        assert_eq!(data.departures[0].source, Source::Scheduled);
    }

    #[test]
    fn flags_canceled_departures() {
        let data = stop(None, vec![stoptime("4", 28_800, 28_800, "CANCELED")]);
        assert!(data.departures[0].canceled);
    }

    #[test]
    fn sorts_by_expected_time() {
        let data = stop(
            None,
            vec![
                stoptime("4", 28_800, 29_400, "UPDATED"),
                stoptime("15", 29_100, 29_100, "SCHEDULED"),
            ],
        );
        let lines: Vec<_> = data.departures.iter().map(|d| d.line.as_str()).collect();
        assert_eq!(lines, ["15", "4"]);
    }

    #[test]
    fn select_applies_window_lines_and_limit() {
        let data = stop(
            None,
            vec![
                stoptime("4", 28_500, 28_500, "SCHEDULED"), // 07:55, already gone
                stoptime("4", 28_800, 28_800, "SCHEDULED"), // 08:00
                stoptime("15", 29_400, 29_400, "SCHEDULED"), // 08:10
                stoptime("4", 30_600, 30_600, "SCHEDULED"), // 08:30
                stoptime("4", 32_400, 32_400, "SCHEDULED"), // 09:00, outside 45 minutes
            ],
        );
        let lines_at = |filter: &Filter| -> Vec<String> {
            data.select(filter, at(8, 0))
                .iter()
                .map(|d| format!("{} {}", d.line, d.expected.format("%H:%M")))
                .collect()
        };

        assert_eq!(
            lines_at(&filter(&[], 45, 10)),
            ["4 08:00", "15 08:10", "4 08:30"]
        );
        assert_eq!(lines_at(&filter(&["4"], 45, 10)), ["4 08:00", "4 08:30"]);
        assert_eq!(lines_at(&filter(&[], 45, 1)), ["4 08:00"]);
    }

    #[test]
    fn line_filter_ignores_case() {
        let data = stop(None, vec![stoptime("9B", 28_800, 28_800, "SCHEDULED")]);
        assert_eq!(data.select(&filter(&["9b"], 60, 10), at(8, 0)).len(), 1);
    }

    /// GTT's trip updates for runs of 2026-10-29, updated at `updated_at`: for each trip
    /// (without `gtt:`), whether it is cancelled and its delay at stop sequence 12.
    fn gtt(runs: &[(&str, bool, Option<i32>)], updated_at: DateTime<Utc>) -> TripUpdates {
        use crate::gtt::proto::{
            FeedEntity, FeedMessage, TripDescriptor, TripUpdate,
            trip_descriptor::ScheduleRelationship,
            trip_update::{StopTimeEvent, StopTimeUpdate},
        };
        let entity = runs
            .iter()
            .map(|(trip, canceled, delay)| FeedEntity {
                id: trip.to_string(),
                trip_update: Some(TripUpdate {
                    trip: TripDescriptor {
                        trip_id: Some(trip.to_string()),
                        start_date: Some("20261029".to_owned()),
                        schedule_relationship: Some(if *canceled {
                            ScheduleRelationship::Canceled as i32
                        } else {
                            ScheduleRelationship::Scheduled as i32
                        }),
                        ..Default::default()
                    },
                    stop_time_update: delay
                        .iter()
                        .map(|delay| StopTimeUpdate {
                            stop_sequence: Some(12),
                            arrival: Some(StopTimeEvent {
                                delay: Some(*delay),
                                ..Default::default()
                            }),
                            ..Default::default()
                        })
                        .collect(),
                    timestamp: Some(updated_at.timestamp() as u64),
                    ..Default::default()
                }),
                ..Default::default()
            })
            .collect();
        TripUpdates::from_feed(FeedMessage {
            entity,
            ..Default::default()
        })
    }

    fn day() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, 29).unwrap()
    }

    #[test]
    fn merges_gtt_predictions_and_sorts_again() {
        let data = stop(
            None,
            vec![
                stoptime("4", 28_800, 28_860, "UPDATED"), // 08:00, OpenTripPlanner +1
                stoptime("15", 29_100, 29_100, "SCHEDULED"), // 08:05
            ],
        );
        let updates = gtt(
            &[("4-28800", false, Some(600)), ("15-29100", false, Some(0))],
            at(7, 59),
        );
        let comparisons = Comparisons::default();
        let merged = data.with_gtt(&updates, &comparisons, at(8, 0));

        let lines: Vec<_> = merged.departures.iter().map(|d| d.line.as_str()).collect();
        assert_eq!(lines, ["15", "4"], "the 4 is now 10 minutes late");
        let four = &merged.departures[1];
        assert_eq!(four.source, Source::Gtt);
        assert_eq!(four.expected.to_rfc3339(), "2026-10-29T08:10:00+01:00");
        assert_eq!(four.delay_seconds, 600);
        assert!(four.realtime);
        assert_eq!(merged.departures[0].source, Source::Gtt);
        assert!(merged.departures[0].realtime);

        let summary = comparisons.summary(day());
        assert_eq!((summary.used, summary.compared), (2, 1));
        assert_eq!(summary.median_difference_seconds, Some(540));
        // The original data, cached for the next requests, is untouched.
        assert_eq!(data.departures[0].source, Source::Otp);
    }

    #[test]
    fn cancellations_from_either_source() {
        let data = stop(
            None,
            vec![
                stoptime("4", 28_800, 28_800, "SCHEDULED"),
                stoptime("15", 29_100, 29_100, "CANCELED"),
            ],
        );
        let updates = gtt(
            &[("4-28800", true, None), ("15-29100", false, Some(60))],
            at(7, 59),
        );
        let merged = data.with_gtt(&updates, &Comparisons::default(), at(8, 0));
        assert!(merged.departures.iter().all(|d| d.canceled));
        assert!(merged.departures.iter().all(|d| d.source == Source::Gtt));
    }

    #[test]
    fn implausible_or_stale_predictions_keep_opentripplanner() {
        let data = stop(None, vec![stoptime("4", 28_800, 28_860, "UPDATED")]);
        let comparisons = Comparisons::default();

        let implausible = gtt(&[("4-28800", false, Some(4_000))], at(7, 59));
        let merged = data.with_gtt(&implausible, &comparisons, at(8, 0));
        assert_eq!(merged.departures[0].source, Source::Otp);
        assert_eq!(merged.departures[0].delay_seconds, 60);
        assert_eq!(comparisons.summary(day()).mismatches, 1);

        let stale = gtt(&[("4-28800", false, Some(120))], at(7, 50));
        let merged = data.with_gtt(&stale, &comparisons, at(8, 0));
        assert_eq!(merged.departures[0].source, Source::Otp);
    }

    #[test]
    fn other_feeds_are_left_alone() {
        let mut data = stop(None, vec![stoptime("4", 28_800, 28_800, "SCHEDULED")]);
        data.departures[0].trip_id = "4:4-28800".to_owned();
        let updates = gtt(&[("4-28800", false, Some(120))], at(7, 59));
        let merged = data.with_gtt(&updates, &Comparisons::default(), at(8, 0));
        assert_eq!(merged.departures[0].source, Source::Scheduled);
    }
}
