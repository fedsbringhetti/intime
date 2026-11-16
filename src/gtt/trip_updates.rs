//! GTT's trip updates, indexed by run, and the prediction they give for one stop of a run.

use std::collections::HashMap;

use chrono::{DateTime, NaiveDate, TimeDelta, Utc};

use super::proto::{
    FeedMessage, trip_descriptor::ScheduleRelationship as TripRelationship,
    trip_update::stop_time_update::ScheduleRelationship as StopRelationship,
};

/// Predictions older than this are ignored: the run is probably not being tracked any more.
pub const MAX_AGE: TimeDelta = TimeDelta::minutes(3);
/// A prediction further than this from the scheduled time is taken as wrong (merge rule 3).
pub const MAX_DIFFERENCE: TimeDelta = TimeDelta::minutes(60);

/// A run: GTT's trip id (without `gtt:`) and its service date.
type RunKey = (String, NaiveDate);

/// Every run in one read of `trip_update.aspx`.
#[derive(Debug, Default)]
pub struct TripUpdates {
    runs: HashMap<RunKey, Run>,
    /// The most recent update of any run: how fresh the feed is.
    pub newest_update: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq)]
struct Run {
    canceled: bool,
    updated_at: Option<DateTime<Utc>>,
    /// The label of the vehicle assigned (`9648`).
    vehicle: Option<String>,
    /// Sorted by stop sequence.
    stops: Vec<StopUpdate>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct StopUpdate {
    sequence: u32,
    event: Event,
}

/// GTT gives each run either absolute times or delays, never both (checked on 2026-11-04).
#[derive(Debug, Clone, Copy, PartialEq)]
enum Event {
    Time(DateTime<Utc>),
    Delay(TimeDelta),
}

/// The vehicle assigned to a run, from its trip update (decision AW).
#[derive(Debug, Clone, PartialEq)]
pub struct RunVehicle {
    pub label: String,
    /// The first stop sequence of the update: where the vehicle is heading.
    pub next_sequence: Option<u32>,
    pub updated_at: Option<DateTime<Utc>>,
}

impl RunVehicle {
    /// Stops from the vehicle to the stop at `sequence`; `None` when the update is older than
    /// [`MAX_AGE`] or the stop is already behind it. Before a run starts the vehicle is at
    /// its first stop: the number then means "not left yet", not "near".
    pub fn stops_away(&self, sequence: u32, now: DateTime<Utc>) -> Option<u32> {
        if self.updated_at.is_some_and(|at| now - at > MAX_AGE) {
            return None;
        }
        sequence.checked_sub(self.next_sequence?)
    }
}

/// What GTT says about one departure.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Prediction {
    /// The expected time at the stop.
    Expected(DateTime<Utc>),
    /// The whole run is cancelled.
    Canceled,
    /// A time too far from the scheduled one: OpenTripPlanner is used and a mismatch counted.
    Implausible(DateTime<Utc>),
    /// Nothing usable: the run is not in the feed (not urban, or not started), its update is
    /// too old, the stop has already been passed, or only absolute times exist before it.
    None,
}

impl TripUpdates {
    pub fn decode(bytes: &[u8]) -> Result<Self, prost::DecodeError> {
        use prost::Message;
        Ok(Self::from_feed(FeedMessage::decode(bytes)?))
    }

    pub fn from_feed(feed: FeedMessage) -> Self {
        let mut runs = HashMap::new();
        let mut newest_update = None;
        for update in feed
            .entity
            .into_iter()
            .filter_map(|entity| entity.trip_update)
        {
            let (Some(trip_id), Some(date)) = (update.trip.trip_id, update.trip.start_date) else {
                continue;
            };
            let Ok(date) = NaiveDate::parse_from_str(&date, "%Y%m%d") else {
                continue;
            };
            let updated_at = update
                .timestamp
                .and_then(|seconds| DateTime::from_timestamp(i64::try_from(seconds).ok()?, 0));
            newest_update = newest_update.max(updated_at);

            let mut stops: Vec<StopUpdate> = update
                .stop_time_update
                .into_iter()
                .filter(|stop| {
                    stop.schedule_relationship.is_none_or(|relationship| {
                        relationship == StopRelationship::Scheduled as i32
                    })
                })
                .filter_map(|stop| {
                    // GTT sends arrivals only; the departure, when present, is preferred.
                    let event = stop.departure.or(stop.arrival)?;
                    let event = match (event.time, event.delay) {
                        (Some(time), _) => Event::Time(DateTime::from_timestamp(time, 0)?),
                        (None, Some(delay)) => Event::Delay(TimeDelta::seconds(delay.into())),
                        (None, None) => return None,
                    };
                    Some(StopUpdate {
                        sequence: stop.stop_sequence?,
                        event,
                    })
                })
                .collect();
            stops.sort_by_key(|stop| stop.sequence);

            runs.insert(
                (trip_id, date),
                Run {
                    canceled: update.trip.schedule_relationship
                        == Some(TripRelationship::Canceled as i32),
                    updated_at,
                    vehicle: update
                        .vehicle
                        .and_then(|vehicle| vehicle.label)
                        .filter(|label| !label.is_empty()),
                    stops,
                },
            );
        }
        Self {
            runs,
            newest_update,
        }
    }

    /// Number of runs in the feed.
    pub fn len(&self) -> usize {
        self.runs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.runs.is_empty()
    }

    /// The vehicle of a run (`trip_id` without `gtt:`), when its update names one.
    pub fn vehicle(&self, trip_id: &str, date: NaiveDate) -> Option<RunVehicle> {
        let run = self.runs.get(&(trip_id.to_owned(), date))?;
        Some(RunVehicle {
            label: run.vehicle.clone()?,
            next_sequence: run.stops.first().map(|stop| stop.sequence),
            updated_at: run.updated_at,
        })
    }

    /// GTT's prediction for the stop at `sequence` of a run scheduled there at `scheduled`.
    /// `trip_id` is GTT's, without the `gtt:` prefix.
    pub fn prediction(
        &self,
        trip_id: &str,
        date: NaiveDate,
        sequence: u32,
        scheduled: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> Prediction {
        let Some(run) = self.runs.get(&(trip_id.to_owned(), date)) else {
            return Prediction::None;
        };
        // A cancellation stays true however old it is.
        if run.canceled {
            return Prediction::Canceled;
        }
        if run.updated_at.is_some_and(|at| now - at > MAX_AGE) {
            return Prediction::None;
        }

        // The update for this stop, or the last one before it, whose delay carries over to the
        // following stops (the GTFS Realtime rule). Updates only after it: already passed.
        let Some(update) = run
            .stops
            .iter()
            .rev()
            .find(|stop| stop.sequence <= sequence)
        else {
            return Prediction::None;
        };
        let expected = match update.event {
            Event::Time(time) if update.sequence == sequence => time,
            // An absolute time at an earlier stop says nothing about the delay here.
            Event::Time(_) => return Prediction::None,
            Event::Delay(delay) => scheduled + delay,
        };

        if (expected - scheduled).abs() > MAX_DIFFERENCE {
            Prediction::Implausible(expected)
        } else {
            Prediction::Expected(expected)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gtt::proto::{
        FeedEntity, FeedHeader, TripDescriptor, TripUpdate,
        trip_update::{StopTimeEvent, StopTimeUpdate},
    };

    const NOW: i64 = 1_793_785_590; // 2026-11-04 10:46:30 +01:00

    fn at(offset_seconds: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(NOW + offset_seconds, 0).unwrap()
    }

    fn date() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, 4).unwrap()
    }

    fn stop(sequence: u32, time: Option<i64>, delay: Option<i32>) -> StopTimeUpdate {
        StopTimeUpdate {
            stop_sequence: Some(sequence),
            arrival: Some(StopTimeEvent {
                time,
                delay,
                ..Default::default()
            }),
            schedule_relationship: Some(StopRelationship::Scheduled as i32),
            ..Default::default()
        }
    }

    fn run(trip: &str, age: i64, canceled: bool, stops: Vec<StopTimeUpdate>) -> FeedEntity {
        FeedEntity {
            id: trip.to_owned(),
            trip_update: Some(TripUpdate {
                trip: TripDescriptor {
                    trip_id: Some(trip.to_owned()),
                    start_date: Some("20261104".to_owned()),
                    schedule_relationship: Some(if canceled {
                        TripRelationship::Canceled as i32
                    } else {
                        TripRelationship::Scheduled as i32
                    }),
                    ..Default::default()
                },
                stop_time_update: stops,
                timestamp: Some((NOW - age) as u64),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    fn feed(entities: Vec<FeedEntity>) -> TripUpdates {
        TripUpdates::from_feed(FeedMessage {
            header: FeedHeader {
                gtfs_realtime_version: "2.0".to_owned(),
                timestamp: Some(NOW as u64),
                ..Default::default()
            },
            entity: entities,
        })
    }

    /// The prediction for stop `sequence` of `trip`, scheduled 10 minutes from now.
    fn predict(updates: &TripUpdates, trip: &str, sequence: u32) -> Prediction {
        updates.prediction(trip, date(), sequence, at(600), at(0))
    }

    #[test]
    fn delay_at_the_stop() {
        let updates = feed(vec![run("1U", 30, false, vec![stop(12, None, Some(120))])]);
        assert_eq!(predict(&updates, "1U", 12), Prediction::Expected(at(720)));
    }

    #[test]
    fn absolute_time_at_the_stop() {
        let updates = feed(vec![run(
            "1U",
            30,
            false,
            vec![stop(12, Some(NOW + 650), None)],
        )]);
        assert_eq!(predict(&updates, "1U", 12), Prediction::Expected(at(650)));
    }

    #[test]
    fn delay_carries_over_from_an_earlier_stop() {
        let updates = feed(vec![run(
            "1U",
            30,
            false,
            vec![stop(8, None, Some(60)), stop(10, None, Some(-30))],
        )]);
        assert_eq!(predict(&updates, "1U", 14), Prediction::Expected(at(570)));
    }

    #[test]
    fn absolute_time_does_not_carry_over() {
        let updates = feed(vec![run(
            "1U",
            30,
            false,
            vec![stop(10, Some(NOW + 300), None)],
        )]);
        assert_eq!(predict(&updates, "1U", 14), Prediction::None);
    }

    #[test]
    fn stop_already_passed() {
        let updates = feed(vec![run("1U", 30, false, vec![stop(20, None, Some(60))])]);
        assert_eq!(predict(&updates, "1U", 14), Prediction::None);
    }

    #[test]
    fn stale_update_is_ignored() {
        let updates = feed(vec![run("1U", 181, false, vec![stop(12, None, Some(60))])]);
        assert_eq!(predict(&updates, "1U", 12), Prediction::None);
        let updates = feed(vec![run("1U", 180, false, vec![stop(12, None, Some(60))])]);
        assert_eq!(predict(&updates, "1U", 12), Prediction::Expected(at(660)));
    }

    #[test]
    fn implausible_prediction() {
        let updates = feed(vec![run(
            "1U",
            30,
            false,
            vec![stop(12, None, Some(3_601))],
        )]);
        assert_eq!(
            predict(&updates, "1U", 12),
            Prediction::Implausible(at(600 + 3_601))
        );
    }

    #[test]
    fn cancelled_run_even_if_old() {
        let updates = feed(vec![run("1U", 3_600, true, vec![])]);
        assert_eq!(predict(&updates, "1U", 12), Prediction::Canceled);
    }

    #[test]
    fn unknown_run_or_other_date() {
        let updates = feed(vec![run("1U", 30, false, vec![stop(12, None, Some(60))])]);
        assert_eq!(predict(&updates, "2U", 12), Prediction::None);
        let tomorrow = date().succ_opt().unwrap();
        assert_eq!(
            updates.prediction("1U", tomorrow, 12, at(600), at(0)),
            Prediction::None
        );
    }

    #[test]
    fn skipped_stops_are_ignored() {
        let mut skipped = stop(12, None, Some(600));
        skipped.schedule_relationship = Some(StopRelationship::Skipped as i32);
        let updates = feed(vec![run(
            "1U",
            30,
            false,
            vec![stop(10, None, Some(60)), skipped],
        )]);
        assert_eq!(predict(&updates, "1U", 12), Prediction::Expected(at(660)));
    }

    #[test]
    fn vehicle_and_stops_away() {
        let mut entity = run(
            "1U",
            30,
            false,
            vec![stop(5, None, Some(0)), stop(9, None, Some(0))],
        );
        entity.trip_update.as_mut().unwrap().vehicle = Some(crate::gtt::proto::VehicleDescriptor {
            id: Some("9648U".to_owned()),
            label: Some("9648".to_owned()),
            ..Default::default()
        });
        let updates = feed(vec![entity, run("2U", 30, false, vec![])]);
        let vehicle = updates.vehicle("1U", date()).unwrap();
        assert_eq!(vehicle.label, "9648");
        assert_eq!(vehicle.next_sequence, Some(5));
        assert_eq!(vehicle.stops_away(12, at(0)), Some(7));
        assert_eq!(vehicle.stops_away(5, at(0)), Some(0));
        assert_eq!(vehicle.stops_away(4, at(0)), None);
        // An update older than three minutes says nothing about where the vehicle is.
        assert_eq!(vehicle.stops_away(12, at(151)), None);
        assert!(updates.vehicle("2U", date()).is_none());
        assert!(updates.vehicle("3U", date()).is_none());
    }

    #[test]
    fn newest_update_and_size() {
        let updates = feed(vec![
            run("1U", 90, false, vec![]),
            run("2U", 10, false, vec![]),
        ]);
        assert_eq!(updates.len(), 2);
        assert_eq!(updates.newest_update, Some(at(-10)));
    }

    #[test]
    fn decodes_the_recorded_feed() {
        let bytes = std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/gtt/trip_update.pb"
        ))
        .unwrap();
        let updates = TripUpdates::decode(&bytes).unwrap();
        assert_eq!(updates.len(), 383);
        assert_eq!(updates.newest_update, Some(at(-1)));
        assert!(TripUpdates::decode(b"not protobuf").is_err());
    }
}
