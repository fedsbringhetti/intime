use chrono::NaiveTime;

use super::*;
use crate::scheduler::store::TrackRun;

fn utc(text: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(text).unwrap().to_utc()
}

/// 2026-11-12, CET: 08:05 in Rome is 07:05 UTC.
const AT: &str = "2026-11-12T07:05:00Z";

fn config() -> TrackConfig {
    TrackConfig {
        line: "67".to_owned(),
        scheduled_time: NaiveTime::from_hms_opt(8, 5, 0).unwrap(),
        tolerance_minutes: 3,
        watch_from_minutes: 30,
        alert_before_minutes: vec![10, 5],
        delay_alert_minutes: 3,
        trip_id: None,
    }
}

fn departure(
    line: &str,
    trip: &str,
    scheduled: &str,
    delay_seconds: i64,
    source: Source,
) -> Departure {
    let scheduled = utc(scheduled).with_timezone(&TIME_ZONE);
    Departure {
        line: line.to_owned(),
        mode: None,
        headsign: Some("CAPOLINEA".to_owned()),
        scheduled,
        expected: scheduled + TimeDelta::seconds(delay_seconds),
        delay_seconds,
        realtime: source != Source::Scheduled,
        canceled: false,
        trip_id: trip.to_owned(),
        source,
        stop_sequence: Some(12),
        service_date: NaiveDate::from_ymd_opt(2026, 11, 12),
    }
}

fn run(delay_seconds: i64) -> Departure {
    departure("67", "gtt:t1", AT, delay_seconds, Source::Gtt)
}

fn go(state: TrackState, departures: &[Departure], now: &str) -> Step {
    go_with(&config(), state, departures, now)
}

fn go_with(config: &TrackConfig, state: TrackState, departures: &[Departure], now: &str) -> Step {
    let input = StepInput {
        config,
        at: utc(AT),
        recurring: config.trip_id.is_none(),
        stop: "Porta Nuova (39)",
    };
    step(state, &input, departures, utc(now))
}

fn titles(step: &Step) -> Vec<&str> {
    step.notices.iter().map(|n| n.title.as_str()).collect()
}

#[test]
fn searching_finds_the_closest_run_in_the_tolerance() {
    let departures = [
        departure("67", "gtt:far", "2026-11-12T07:09:00Z", 0, Source::Otp),
        departure("67", "gtt:near", "2026-11-12T07:06:00Z", 0, Source::Otp),
        departure("68", "gtt:other", AT, 0, Source::Otp),
    ];
    let out = go(TrackState::default(), &departures, "2026-11-12T06:40:00Z");
    assert_eq!(out.status, TrackStatus::Tracking);
    assert_eq!(out.state.trip_id.as_deref(), Some("gtt:near"));
    assert_eq!(out.state.stop_sequence, Some(12));
    assert!(out.notices.is_empty());
}

#[test]
fn a_tie_goes_to_the_earlier_run() {
    let departures = [
        departure("67", "gtt:late", "2026-11-12T07:07:00Z", 0, Source::Otp),
        departure("67", "gtt:early", "2026-11-12T07:03:00Z", 0, Source::Otp),
    ];
    let out = go(TrackState::default(), &departures, "2026-11-12T06:40:00Z");
    assert_eq!(out.state.trip_id.as_deref(), Some("gtt:early"));
}

#[test]
fn on_time_sends_the_two_reminders_once() {
    let departures = [run(0)];
    let mut state = TrackState::default();
    let mut sent = Vec::new();
    for now in [
        "2026-11-12T06:40:00Z",
        "2026-11-12T06:55:00Z", // 10 minutes
        "2026-11-12T06:55:15Z",
        "2026-11-12T07:00:00Z", // 5 minutes
        "2026-11-12T07:00:15Z",
    ] {
        let out = go(state, &departures, now);
        assert_eq!(out.status, TrackStatus::Tracking);
        sent.extend(
            out.notices
                .iter()
                .map(|n| (n.title.clone(), n.body.clone())),
        );
        state = out.state;
    }
    assert_eq!(
        sent,
        vec![
            (
                "⏰ Il 67 delle 08:05 passa tra 10 minuti".to_owned(),
                "08:05 · Porta Nuova (39)".to_owned()
            ),
            (
                "⏰ Il 67 delle 08:05 passa tra 5 minuti".to_owned(),
                "08:05 · Porta Nuova (39)".to_owned()
            ),
        ]
    );
    assert_eq!(state.reminders_done, vec![10, 5]);
    assert_eq!(state.events.len(), 2);
    assert_eq!(state.events[1].kind, EventKind::Reminder);
    assert_eq!(state.events[1].minutes, Some(5));
    assert_eq!(state.delay_alerted, None);
    // Past its time the run is done.
    let out = go(state, &departures, "2026-11-12T07:05:00Z");
    assert_eq!(out.status, TrackStatus::Done);
}

#[test]
fn reminder_body_shows_the_delay_or_the_timetable() {
    let late = departure("67", "gtt:t1", AT, 120, Source::Gtt);
    let out = go(TrackState::default(), &[late], "2026-11-12T06:58:30Z");
    assert_eq!(out.notices[0].body, "08:07, +2 · Porta Nuova (39)");
    let timetable = departure("67", "gtt:t1", AT, 0, Source::Scheduled);
    let out = go(TrackState::default(), &[timetable], "2026-11-12T06:55:30Z");
    assert_eq!(out.notices[0].body, "08:05 (orario) · Porta Nuova (39)");
}

#[test]
fn getting_later_alerts_at_three_and_at_six_minutes() {
    let mut state = TrackState::default();
    let mut delays = Vec::new();
    for (now, delay) in [
        ("2026-11-12T06:40:00Z", 60),
        ("2026-11-12T06:41:00Z", 180), // alert at 3
        ("2026-11-12T06:42:00Z", 240),
        ("2026-11-12T06:43:00Z", 300),
        ("2026-11-12T06:44:00Z", 360), // alert at 6
        ("2026-11-12T06:45:00Z", 360),
    ] {
        let out = go(state, &[run(delay)], now);
        for notice in &out.notices {
            assert_eq!(notice.event.kind, EventKind::Delay);
            delays.push((notice.title.clone(), notice.body.clone()));
        }
        state = out.state;
    }
    assert_eq!(
        delays,
        vec![
            (
                "🐢 Il 67 delle 08:05 è in ritardo di 3 minuti".to_owned(),
                "Previsto alle 08:08 · Porta Nuova (39)".to_owned()
            ),
            (
                "🐢 Il 67 delle 08:05 è in ritardo di 6 minuti".to_owned(),
                "Previsto alle 08:11 · Porta Nuova (39)".to_owned()
            ),
        ]
    );
    assert_eq!(state.delay_alerted, Some(6));
}

#[test]
fn a_timetable_only_run_never_alerts_a_delay() {
    let departure = departure("67", "gtt:t1", AT, 600, Source::Scheduled);
    let out = go(TrackState::default(), &[departure], "2026-11-12T06:40:00Z");
    assert!(out.notices.is_empty());
}

#[test]
fn cancelled_run_alerts_once_with_the_next_departure() {
    let mut cancelled = run(0);
    cancelled.canceled = true;
    let next = departure("67", "gtt:t2", "2026-11-12T07:17:00Z", 0, Source::Otp);
    let other_line = departure("68", "gtt:t3", "2026-11-12T07:08:00Z", 0, Source::Otp);
    let out = go(
        TrackState::default(),
        &[cancelled.clone(), other_line, next],
        "2026-11-12T06:50:00Z",
    );
    assert_eq!(out.status, TrackStatus::Cancelled);
    assert_eq!(titles(&out), vec!["⚠️ Il 67 delle 08:05 è cancellato"]);
    assert_eq!(out.notices[0].body, "Il prossimo: 08:17");
    assert_eq!(out.notices[0].event.kind, EventKind::Cancelled);

    let out = go(TrackState::default(), &[cancelled], "2026-11-12T06:50:00Z");
    assert_eq!(out.notices[0].body, "Nessun altro 67 nelle prossime 2 ore");
}

#[test]
fn never_found_gives_up_ten_minutes_before_with_the_closest_runs() {
    let departures = [
        departure("67", "gtt:a", "2026-11-12T06:58:00Z", 0, Source::Otp),
        departure("67", "gtt:b", "2026-11-12T07:12:00Z", 0, Source::Otp),
        departure("67", "gtt:c", "2026-11-12T07:40:00Z", 0, Source::Otp),
    ];
    let out = go(TrackState::default(), &departures, "2026-11-12T06:54:59Z");
    assert_eq!(out.status, TrackStatus::Searching);
    assert!(out.notices.is_empty());
    assert_eq!(out.state.checked_at, Some(utc("2026-11-12T06:54:59Z")));

    let out = go(TrackState::default(), &departures, "2026-11-12T06:55:00Z");
    assert_eq!(out.status, TrackStatus::NotFound);
    assert_eq!(titles(&out), vec!["❓ Il 67 delle 08:05 non si trova"]);
    assert_eq!(out.notices[0].body, "Più vicine: 07:58, 08:12");
    assert_eq!(out.notices[0].event.kind, EventKind::NotFound);
}

#[test]
fn not_found_after_the_time_is_silent() {
    let out = go(TrackState::default(), &[], "2026-11-12T07:06:00Z");
    assert_eq!(out.status, TrackStatus::NotFound);
    assert!(out.notices.is_empty());
    assert!(out.state.events.is_empty());
}

#[test]
fn found_then_vanished_keeps_the_state_until_the_time() {
    let found = go(TrackState::default(), &[run(120)], "2026-11-12T06:40:00Z");
    // Gone from the list 20 minutes before its time: a glitch, the run is still followed.
    let out = go(found.state.clone(), &[], "2026-11-12T06:45:00Z");
    assert_eq!(out.status, TrackStatus::Tracking);
    assert_eq!(out.state, found.state);
    // Within two minutes of its expected time (08:07): it has left.
    let out = go(found.state.clone(), &[], "2026-11-12T07:05:30Z");
    assert_eq!(out.status, TrackStatus::Done);
    assert_eq!(out.state.delay_seconds, Some(120));
    // And it is never searched again once found.
    let other = departure("67", "gtt:t9", AT, 0, Source::Otp);
    let out = go(found.state, &[other], "2026-11-12T06:50:00Z");
    assert_eq!(out.state.trip_id.as_deref(), Some("gtt:t1"));
}

#[test]
fn a_late_start_sends_no_stale_reminder() {
    // Tracking starts at 07:01 with 4 minutes left: the 10 is long past, the 5 is current.
    let out = go(TrackState::default(), &[run(0)], "2026-11-12T07:01:00Z");
    assert_eq!(
        titles(&out),
        vec!["⏰ Il 67 delle 08:05 passa tra 5 minuti"]
    );
    assert_eq!(out.state.reminders_done, vec![10, 5]);

    // Starts at 07:03:30, 90 seconds left: the 5 is more than 2 minutes past: nothing.
    let out = go(TrackState::default(), &[run(0)], "2026-11-12T07:03:30Z");
    assert!(out.notices.is_empty());
    assert_eq!(out.state.reminders_done, vec![10, 5]);
    assert_eq!(out.status, TrackStatus::Tracking);
}

#[test]
fn a_restart_mid_run_does_not_repeat_what_was_sent() {
    let out = go(TrackState::default(), &[run(0)], "2026-11-12T06:55:10Z");
    assert_eq!(out.notices.len(), 1);
    // The state is what a restarted scheduler reads back from the database.
    let stored = serde_json::to_string(&out.state).unwrap();
    let restored: TrackState = serde_json::from_str(&stored).unwrap();
    assert_eq!(restored, out.state);
    let out = go(restored, &[run(0)], "2026-11-12T06:55:25Z");
    assert!(out.notices.is_empty());
    let out = go(out.state, &[run(0)], "2026-11-12T07:00:00Z");
    assert_eq!(
        titles(&out),
        vec!["⏰ Il 67 delle 08:05 passa tra 5 minuti"]
    );
}

#[test]
fn a_reminder_passed_while_delayed_is_skipped_not_sent_late() {
    // Run 8 minutes late (expected 08:13), checked at 08:08 Rome... 5 minutes left: the 10
    // reminder is long past and is marked, the 5 is current.
    let out = go(TrackState::default(), &[run(480)], "2026-11-12T07:08:00Z");
    assert_eq!(
        titles(&out),
        vec![
            "⏰ Il 67 delle 08:05 passa tra 5 minuti",
            "🐢 Il 67 delle 08:05 è in ritardo di 8 minuti"
        ]
    );
    // Now it recovers to on time by 07:04:30 (expected 07:05): both are done, nothing more.
    let out = go(out.state, &[run(0)], "2026-11-12T07:04:30Z");
    assert!(out.notices.is_empty());
}

#[test]
fn a_one_off_follows_its_trip_without_searching() {
    let mut one_off = config();
    one_off.trip_id = Some("gtt:t7".to_owned());
    one_off.tolerance_minutes = 0;
    let departures = [
        departure("67", "gtt:t1", AT, 0, Source::Otp),
        departure("67", "gtt:t7", "2026-11-12T07:30:00Z", 0, Source::Otp),
    ];
    let out = go_with(
        &one_off,
        TrackState::default(),
        &departures,
        "2026-11-12T07:25:00Z",
    );
    assert_eq!(out.state.trip_id.as_deref(), Some("gtt:t7"));
    assert_eq!(out.status, TrackStatus::Tracking);
    assert_eq!(out.notices.len(), 1);
}

fn track_rule() -> Rule {
    Rule {
        id: 7,
        name: "67 delle 08:05".to_owned(),
        enabled: true,
        stop_id: "gtt:39".to_owned(),
        days: vec![chrono::Weekday::Thu],
        date: None,
        skip_holidays: false,
        paused_until: None,
        config: RuleConfig::Track(config()),
        created_at: utc("2026-11-01T00:00:00Z"),
        updated_at: utc("2026-11-01T00:00:00Z"),
    }
}

#[test]
fn view_before_the_run_is_waiting_with_every_reminder_planned() {
    let rule = track_rule();
    let view = view(&rule, None, None, utc("2026-11-12T05:00:00Z")).unwrap();
    assert_eq!(view.phase, "waiting");
    assert!(view.recurring);
    assert_eq!(view.planned_reminders, vec![10, 5]);
    assert_eq!(
        view.scheduled.unwrap().to_rfc3339(),
        "2026-11-12T08:05:00+01:00"
    );
    assert!(view.vehicle.is_none());
    assert!(view.events.is_empty());
}

#[test]
fn view_of_a_run_in_progress() {
    let rule = track_rule();
    let out = go(TrackState::default(), &[run(120)], "2026-11-12T06:58:00Z");
    let run = TrackRun {
        rule_id: 7,
        date: NaiveDate::from_ymd_opt(2026, 11, 12).unwrap(),
        status: "tracking".to_owned(),
        due_at: utc(AT),
        state: out.state,
        updated_at: utc("2026-11-12T06:58:00Z"),
    };
    let view = view(&rule, Some(&run), None, utc("2026-11-12T06:58:00Z")).unwrap();
    assert_eq!(view.phase, "tracking");
    assert_eq!(view.planned_reminders, vec![5]);
    assert_eq!(view.headsign.as_deref(), Some("Capolinea"));
    assert_eq!(view.delay_seconds, Some(120));
    assert_eq!(view.source, Some(Source::Gtt));
    assert_eq!(view.events.len(), 1);
    assert_eq!(view.events[0].kind, EventKind::Reminder);
    let json = serde_json::to_value(&view).unwrap();
    assert_eq!(json["source"], "gtt");
    assert_eq!(json["events"][0]["kind"], "reminder");
}
