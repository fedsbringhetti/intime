use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use chrono::{DateTime, NaiveDate, NaiveTime, Utc, Weekday};
use sqlx::SqlitePool;

use super::*;
use crate::{
    db,
    departures::{Departure, DeparturesError, Fetched, Source, StopDepartures, StopRef},
    gtt::proto,
    notify::{BoxFuture, Notification, Notifier, NotifyError, SendReport},
    rules::{NewRule, Rule, RuleConfig, RuleStore, SummaryConfig},
    time::TIME_ZONE,
};

fn utc(text: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(text).unwrap().to_utc()
}

fn date(year: i32, month: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(year, month, day).unwrap()
}

const ALL_DAYS: [Weekday; 7] = [
    Weekday::Mon,
    Weekday::Tue,
    Weekday::Wed,
    Weekday::Thu,
    Weekday::Fri,
    Weekday::Sat,
    Weekday::Sun,
];

/// Thursday 12 November 2026, 07:40 in Rome (CET, UTC+1) is 06:40 UTC.
const DUE: &str = "2026-11-12T06:40:00Z";

#[derive(Default)]
struct Recorder {
    sent: Mutex<Vec<Notification>>,
}

impl Recorder {
    fn all(&self) -> Vec<Notification> {
        self.sent.lock().unwrap().clone()
    }
    fn count(&self) -> usize {
        self.sent.lock().unwrap().len()
    }
}

impl Notifier for Recorder {
    fn channel(&self) -> &'static str {
        "test"
    }
    fn send<'a>(
        &'a self,
        notification: &'a Notification,
    ) -> BoxFuture<'a, Result<SendReport, NotifyError>> {
        self.sent.lock().unwrap().push(notification.clone());
        Box::pin(async {
            Ok(SendReport {
                sent: 1,
                removed: 0,
                failed: 0,
            })
        })
    }
}

/// `None` makes the upstream fail.
struct FakeDepartures(Mutex<Option<Vec<Departure>>>);

impl DepartureSource for FakeDepartures {
    fn departures<'a>(
        &'a self,
        stop_id: &'a str,
    ) -> BoxFuture<'a, Result<Fetched, Arc<DeparturesError>>> {
        let result = match self.0.lock().unwrap().clone() {
            Some(departures) => Ok(Fetched {
                data: Arc::new(StopDepartures {
                    stop: StopRef {
                        id: stop_id.to_owned(),
                        code: "39".to_owned(),
                        name: "Rebaudengo".to_owned(),
                    },
                    fetched_at: Utc::now(),
                    departures,
                }),
                stale: false,
            }),
            None => Err(Arc::new(DeparturesError::StopNotFound(stop_id.to_owned()))),
        };
        Box::pin(async { result })
    }
}

struct FakeAlerts(Mutex<Arc<Alerts>>);

impl AlertSource for FakeAlerts {
    fn current(&self) -> Arc<Alerts> {
        self.0.lock().unwrap().clone()
    }
}

fn translated(text: &str) -> Option<proto::TranslatedString> {
    Some(proto::TranslatedString {
        translation: vec![proto::translated_string::Translation {
            text: text.to_owned(),
            language: Some("it".to_owned()),
        }],
    })
}

/// An alert at GTT stop 2843 (our `gtt:39`) naming `routes`, active always.
fn alert(id: &str, routes: &[&str]) -> proto::FeedEntity {
    let mut informed: Vec<proto::EntitySelector> = routes
        .iter()
        .map(|route| proto::EntitySelector {
            route_id: Some((*route).to_owned()),
            ..Default::default()
        })
        .collect();
    informed.push(proto::EntitySelector {
        stop_id: Some("2843".to_owned()),
        ..Default::default()
    });
    proto::FeedEntity {
        id: id.to_owned(),
        alert: Some(proto::Alert {
            informed_entity: informed,
            header_text: translated(&format!("Avviso {id}")),
            description_text: translated(&format!("Dettagli {id}")),
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn alerts_read(entities: Vec<proto::FeedEntity>, read_at: &str) -> Arc<Alerts> {
    let stop_ids = [("2843".to_owned(), "gtt:39".to_owned())].into();
    Arc::new(Alerts::from_feed(
        proto::FeedMessage {
            entity: entities,
            ..Default::default()
        },
        &stop_ids,
        &[],
        utc(read_at),
    ))
}

fn departure(
    line: &str,
    headsign: &str,
    expected: &str,
    delay_seconds: i64,
    source: Source,
    canceled: bool,
) -> Departure {
    let expected = utc(expected).with_timezone(&TIME_ZONE);
    Departure {
        line: line.to_owned(),
        mode: None,
        headsign: Some(headsign.to_owned()),
        scheduled: expected - chrono::TimeDelta::seconds(delay_seconds),
        expected,
        delay_seconds,
        realtime: source != Source::Scheduled,
        canceled,
        trip_id: format!("trip-{line}"),
        source,
        stop_sequence: None,
        service_date: None,
    }
}

fn standard_departures() -> Vec<Departure> {
    vec![
        departure(
            "4",
            "Falchera",
            "2026-11-12T06:43:10Z",
            120,
            Source::Otp,
            false,
        ),
        departure(
            "15",
            "Sassi",
            "2026-11-12T06:47:10Z",
            0,
            Source::Scheduled,
            false,
        ),
        departure("9", "Centro", "2026-11-12T06:50:10Z", 0, Source::Gtt, true),
    ]
}

struct Harness {
    pool: SqlitePool,
    rules: RuleStore,
    store: SchedulerStore,
    departures: Arc<FakeDepartures>,
    alerts: Arc<FakeAlerts>,
    notifier: Arc<Recorder>,
    with_alerts: bool,
}

impl Harness {
    async fn new() -> Self {
        let pool = db::connect_in_memory().await.unwrap();
        Self {
            rules: RuleStore::new(pool.clone()),
            store: SchedulerStore::new(pool.clone()),
            pool,
            departures: Arc::new(FakeDepartures(Mutex::new(Some(standard_departures())))),
            alerts: Arc::new(FakeAlerts(Mutex::new(Arc::default()))),
            notifier: Arc::new(Recorder::default()),
            with_alerts: false,
        }
    }

    async fn with_alerts() -> Self {
        Self {
            with_alerts: true,
            ..Self::new().await
        }
    }

    /// A new scheduler on the same database, as after a restart.
    fn scheduler(&self) -> Scheduler {
        Scheduler::new(
            self.rules.clone(),
            self.store.clone(),
            self.departures.clone(),
            self.with_alerts
                .then(|| self.alerts.clone() as Arc<dyn AlertSource>),
            StopIndex::new(self.pool.clone()),
            self.notifier.clone(),
            Duration::from_secs(5 * 60),
        )
    }

    async fn rule(&self, edit: impl FnOnce(&mut NewRule)) -> Rule {
        self.rule_created("2026-11-01T00:00:00Z", edit).await
    }

    async fn rule_created(&self, created: &str, edit: impl FnOnce(&mut NewRule)) -> Rule {
        let mut rule = NewRule {
            name: "Morning".to_owned(),
            enabled: true,
            stop_id: "gtt:39".to_owned(),
            days: ALL_DAYS.to_vec(),
            date: None,
            skip_holidays: false,
            paused_until: None,
            config: RuleConfig::Summary(SummaryConfig {
                time: NaiveTime::from_hms_opt(7, 40, 0).unwrap(),
                lines: vec![],
                window_minutes: 30,
                max_departures: 6,
            }),
        };
        edit(&mut rule);
        self.rules.create(rule, utc(created)).await.unwrap()
    }

    async fn status(&self, rule: &Rule, date: NaiveDate) -> Option<String> {
        self.store
            .get_run(rule.id, date)
            .await
            .unwrap()
            .map(|run| run.status)
    }

    fn set_departures(&self, departures: Option<Vec<Departure>>) {
        *self.departures.0.lock().unwrap() = departures;
    }
}

fn at_time(hour: u32, minute: u32) -> impl FnOnce(&mut NewRule) {
    move |rule| {
        if let RuleConfig::Summary(config) = &mut rule.config {
            config.time = NaiveTime::from_hms_opt(hour, minute, 0).unwrap();
        }
    }
}

fn with_lines(lines: &[&str]) -> impl FnOnce(&mut NewRule) {
    let lines: Vec<String> = lines.iter().map(|line| (*line).to_owned()).collect();
    move |rule| {
        if let RuleConfig::Summary(config) = &mut rule.config {
            config.lines = lines;
        }
    }
}

#[tokio::test]
async fn sends_once_across_ticks_and_restarts() {
    let h = Harness::new().await;
    let rule = h.rule(|_| {}).await;
    let scheduler = h.scheduler();
    scheduler.tick(utc("2026-11-12T06:40:10Z")).await;
    scheduler.tick(utc("2026-11-12T06:40:25Z")).await;
    h.scheduler().tick(utc("2026-11-12T06:40:40Z")).await;

    let sent = h.notifier.all();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].title, "🚌 Morning — Rebaudengo (39)");
    assert_eq!(
        sent[0].body,
        "4 → Falchera 07:43 (+2) tra 3'\n15 → Sassi 07:47 (orario) tra 7'\n9 → Centro 07:50 cancellata"
    );
    assert_eq!(sent[0].url, "/fermata/39");
    assert_eq!(sent[0].tag.as_deref(), Some(&*format!("rule-{}", rule.id)));
    assert_eq!(
        h.status(&rule, date(2026, 11, 12)).await.as_deref(),
        Some("sent")
    );
    // Yesterday's occurrence was long past when the rule was first seen: skipped, not sent.
    assert_eq!(
        h.status(&rule, date(2026, 11, 11)).await.as_deref(),
        Some("skipped")
    );
    let log = h.store.list_notifications(10).await.unwrap();
    assert_eq!(log.len(), 1);
    assert_eq!(
        (log[0].kind.as_str(), log[0].channel.as_str(), log[0].sent),
        ("summary", "test", 1)
    );
}

#[tokio::test]
async fn six_minutes_late_is_skipped_and_five_is_sent() {
    let h = Harness::new().await;
    let late = h.rule(|_| {}).await;
    h.scheduler().tick(utc("2026-11-12T06:46:00Z")).await;
    assert_eq!(h.notifier.count(), 0);
    assert_eq!(
        h.status(&late, date(2026, 11, 12)).await.as_deref(),
        Some("skipped")
    );
    // Never sent later either.
    h.scheduler().tick(utc("2026-11-12T06:47:00Z")).await;
    assert_eq!(h.notifier.count(), 0);

    let h = Harness::new().await;
    let edge = h.rule(|_| {}).await;
    h.scheduler().tick(utc("2026-11-12T06:45:00Z")).await;
    assert_eq!(h.notifier.count(), 1);
    assert_eq!(
        h.status(&edge, date(2026, 11, 12)).await.as_deref(),
        Some("sent")
    );
}

#[tokio::test]
async fn a_rule_created_after_its_time_today_is_skipped() {
    let h = Harness::new().await;
    let rule = h.rule_created("2026-11-12T06:41:00Z", |_| {}).await;
    h.scheduler().tick(utc("2026-11-12T06:42:00Z")).await;
    assert_eq!(h.notifier.count(), 0);
    assert_eq!(
        h.status(&rule, date(2026, 11, 12)).await.as_deref(),
        Some("skipped")
    );
    assert_eq!(h.status(&rule, date(2026, 11, 11)).await, None);
}

#[tokio::test]
async fn sends_nothing_when_paused_unscheduled_or_disabled() {
    let h = Harness::new().await;
    let paused = h
        .rule(|rule| rule.paused_until = Some(date(2026, 11, 12)))
        .await;
    let other_day = h
        .rule(|rule| rule.days = vec![Weekday::Fri, Weekday::Mon])
        .await;
    let disabled = h.rule(|rule| rule.enabled = false).await;
    h.scheduler().tick(utc("2026-11-12T06:40:10Z")).await;
    assert_eq!(h.notifier.count(), 0);
    for rule in [&paused, &other_day, &disabled] {
        assert_eq!(h.status(rule, date(2026, 11, 12)).await, None);
    }
}

#[tokio::test]
async fn sends_nothing_on_a_holiday() {
    let h = Harness::new().await;
    let rule = h.rule(|rule| rule.skip_holidays = true).await;
    // Friday 25 December 2026, 07:40 CET.
    h.scheduler().tick(utc("2026-12-25T06:40:10Z")).await;
    assert_eq!(h.notifier.count(), 0);
    assert_eq!(h.status(&rule, date(2026, 12, 25)).await, None);
}

#[tokio::test]
async fn a_late_evening_summary_is_caught_after_midnight() {
    let h = Harness::new().await;
    let rule = h.rule(at_time(23, 58)).await;
    // 23:58 CET on 11 November is 22:58 UTC; seen at 00:01 on the 12th.
    h.set_departures(Some(vec![departure(
        "4",
        "Falchera",
        "2026-11-11T23:10:00Z",
        0,
        Source::Otp,
        false,
    )]));
    let scheduler = h.scheduler();
    scheduler.tick(utc("2026-11-11T23:01:00Z")).await;
    scheduler.tick(utc("2026-11-11T23:01:15Z")).await;
    assert_eq!(h.notifier.count(), 1);
    assert_eq!(
        h.status(&rule, date(2026, 11, 11)).await.as_deref(),
        Some("sent")
    );
    assert_eq!(h.status(&rule, date(2026, 11, 12)).await, None);
}

#[tokio::test]
async fn daylight_saving_gap_sends_at_the_first_valid_time() {
    // 29 March 2026: 02:30 does not exist; the summary is due at 03:00 CEST, 01:00 UTC.
    let h = Harness::new().await;
    let rule = h.rule_created("2026-03-01T00:00:00Z", at_time(2, 30)).await;
    h.set_departures(Some(vec![]));
    let scheduler = h.scheduler();
    scheduler.tick(utc("2026-03-29T00:59:59Z")).await;
    assert_eq!(h.notifier.count(), 0);
    scheduler.tick(utc("2026-03-29T01:00:00Z")).await;
    scheduler.tick(utc("2026-03-29T01:00:30Z")).await;
    assert_eq!(h.notifier.count(), 1);
    assert_eq!(
        h.status(&rule, date(2026, 3, 29)).await.as_deref(),
        Some("sent")
    );
}

#[tokio::test]
async fn daylight_saving_repeat_sends_once() {
    // 25 October 2026: 02:30 happens twice; the first one is 00:30 UTC.
    let h = Harness::new().await;
    let rule = h.rule_created("2026-10-01T00:00:00Z", at_time(2, 30)).await;
    h.set_departures(Some(vec![]));
    let scheduler = h.scheduler();
    scheduler.tick(utc("2026-10-25T00:29:59Z")).await;
    assert_eq!(h.notifier.count(), 0);
    scheduler.tick(utc("2026-10-25T00:30:00Z")).await;
    assert_eq!(h.notifier.count(), 1);
    // The second 02:30 (01:30 UTC) does not send again.
    h.scheduler().tick(utc("2026-10-25T01:30:00Z")).await;
    assert_eq!(h.notifier.count(), 1);
    assert_eq!(
        h.status(&rule, date(2026, 10, 25)).await.as_deref(),
        Some("sent")
    );
}

#[tokio::test]
async fn an_empty_window_still_sends_a_short_message() {
    let h = Harness::new().await;
    h.rule(|_| {}).await;
    h.set_departures(Some(vec![]));
    h.scheduler().tick(utc("2026-11-12T06:40:10Z")).await;
    let sent = h.notifier.all();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].body, "Nessuna partenza nei prossimi 30 minuti.");
}

#[tokio::test]
async fn an_upstream_failure_is_retried_then_reported_once() {
    let h = Harness::new().await;
    let rule = h.rule(|_| {}).await;
    h.set_departures(None);
    let scheduler = h.scheduler();
    scheduler.tick(utc("2026-11-12T06:40:10Z")).await;
    scheduler.tick(utc("2026-11-12T06:44:00Z")).await;
    assert_eq!(h.notifier.count(), 0);
    assert_eq!(
        h.status(&rule, date(2026, 11, 12)).await.as_deref(),
        Some("pending")
    );

    scheduler.tick(utc("2026-11-12T06:45:30Z")).await;
    scheduler.tick(utc("2026-11-12T06:46:00Z")).await;
    let sent = h.notifier.all();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].title, "⚠️ Morning");
    assert_eq!(
        sent[0].body,
        "Riepilogo non disponibile: dati non raggiungibili."
    );
    assert_eq!(
        h.status(&rule, date(2026, 11, 12)).await.as_deref(),
        Some("failed")
    );
}

#[tokio::test]
async fn a_pending_run_found_after_the_grace_never_sends_the_summary() {
    let h = Harness::new().await;
    let rule = h.rule(|_| {}).await;
    h.set_departures(None);
    h.scheduler().tick(utc("2026-11-12T06:40:10Z")).await;
    assert_eq!(
        h.status(&rule, date(2026, 11, 12)).await.as_deref(),
        Some("pending")
    );
    // The process was down; a new scheduler finds the run long after the grace, upstream OK.
    h.set_departures(Some(standard_departures()));
    h.scheduler().tick(utc("2026-11-12T06:58:00Z")).await;
    assert_eq!(h.notifier.count(), 0);
    assert_eq!(
        h.status(&rule, date(2026, 11, 12)).await.as_deref(),
        Some("failed")
    );
    h.scheduler().tick(utc("2026-11-12T06:59:00Z")).await;
    assert_eq!(h.notifier.count(), 0);
}

#[tokio::test]
async fn a_pending_run_just_past_the_grace_never_sends_the_summary_even_if_upstream_is_back() {
    let h = Harness::new().await;
    let rule = h.rule(|_| {}).await;
    h.set_departures(None);
    h.scheduler().tick(utc("2026-11-12T06:40:10Z")).await;
    h.set_departures(Some(standard_departures()));
    h.scheduler().tick(utc("2026-11-12T06:46:00Z")).await;
    let sent = h.notifier.all();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].title, "⚠️ Morning");
    assert_eq!(
        h.status(&rule, date(2026, 11, 12)).await.as_deref(),
        Some("failed")
    );
}

#[tokio::test]
async fn a_retry_that_succeeds_sends_the_summary() {
    let h = Harness::new().await;
    let rule = h.rule(|_| {}).await;
    h.set_departures(None);
    h.scheduler().tick(utc("2026-11-12T06:40:10Z")).await;
    h.set_departures(Some(standard_departures()));
    h.scheduler().tick(utc("2026-11-12T06:42:10Z")).await;
    h.scheduler().tick(utc("2026-11-12T06:42:25Z")).await;
    assert_eq!(h.notifier.count(), 1);
    assert!(h.notifier.all()[0].title.starts_with("🚌"));
    assert_eq!(
        h.status(&rule, date(2026, 11, 12)).await.as_deref(),
        Some("sent")
    );
}

#[tokio::test]
async fn a_run_left_running_by_a_crash_is_failed_and_never_sent() {
    let h = Harness::new().await;
    let rule = h.rule(|_| {}).await;
    let now = utc("2026-11-12T06:40:10Z");
    h.store
        .insert_run_if_absent(rule.id, date(2026, 11, 12), utc(DUE), now)
        .await
        .unwrap();
    let scheduler = h.scheduler();
    scheduler.recover(now).await;
    scheduler.tick(now).await;
    assert_eq!(h.notifier.count(), 0);
    assert_eq!(
        h.status(&rule, date(2026, 11, 12)).await.as_deref(),
        Some("failed")
    );
}

#[tokio::test]
async fn the_payload_is_cut_to_the_limit() {
    let h = Harness::new().await;
    h.rule(|rule| {
        if let RuleConfig::Summary(config) = &mut rule.config {
            config.max_departures = 20;
        }
    })
    .await;
    let long = "Capolinea molto lontano ".repeat(10);
    h.set_departures(Some(
        (0..20)
            .map(|i| {
                departure(
                    "4",
                    &long,
                    &format!("2026-11-12T06:41:{:02}Z", 10 + i),
                    0,
                    Source::Otp,
                    false,
                )
            })
            .collect(),
    ));
    h.scheduler().tick(utc("2026-11-12T06:40:10Z")).await;
    let sent = h.notifier.all();
    assert_eq!(sent.len(), 1);
    assert!(sent[0].payload().unwrap().len() <= MAX_PAYLOAD_BYTES);
    assert!(sent[0].body.starts_with("4 → Capolinea"));
    assert!(sent[0].body.lines().count() < 20);
}

#[test]
fn fit_payload_cuts_a_single_huge_line() {
    let fitted = fit_payload(Notification {
        title: "t".to_owned(),
        body: "è".repeat(5000),
        url: "/x".to_owned(),
        tag: None,
    });
    assert!(fitted.payload().unwrap().len() <= MAX_PAYLOAD_BYTES);
    assert!(fitted.body.ends_with('…'));
}

#[tokio::test]
async fn test_sends_log_but_write_no_run() {
    let h = Harness::new().await;
    let rule = h.rule(|_| {}).await;
    let now = utc("2026-11-12T06:40:10Z");
    let stops = StopIndex::new(h.pool.clone());
    let notification = summary_notification(h.departures.as_ref(), &stops, &rule, now)
        .await
        .unwrap();
    let (sent, report) = send_and_log(
        h.notifier.as_ref(),
        &h.store,
        Some(rule.id),
        "summary",
        notification,
        now,
    )
    .await
    .unwrap();
    assert_eq!(report.sent, 1);
    assert_eq!(h.notifier.all(), vec![sent]);
    assert_eq!(h.status(&rule, date(2026, 11, 12)).await, None);
    assert_eq!(h.store.list_notifications(10).await.unwrap().len(), 1);
}

#[tokio::test]
async fn new_alerts_are_sent_once_after_the_summary_and_never_the_baseline() {
    let h = Harness::with_alerts().await;
    let rule = h.rule(|_| {}).await;
    // The first read is the baseline: "old" is never sent.
    *h.alerts.0.lock().unwrap() = alerts_read(vec![alert("old", &["4U"])], "2026-11-12T06:00:00Z");
    let scheduler = h.scheduler();
    scheduler.tick(utc("2026-11-12T06:00:30Z")).await;
    assert_eq!(h.notifier.count(), 0);

    *h.alerts.0.lock().unwrap() = alerts_read(
        vec![alert("old", &["4U"]), alert("new", &["4U"])],
        "2026-11-12T06:30:00Z",
    );
    scheduler.tick(utc("2026-11-12T06:30:30Z")).await;
    // Not at night or off schedule: only at the occurrence.
    assert_eq!(h.notifier.count(), 0);
    scheduler.tick(utc("2026-11-12T06:40:10Z")).await;
    scheduler.tick(utc("2026-11-12T06:40:25Z")).await;
    let sent = h.notifier.all();
    assert_eq!(sent.len(), 2);
    assert!(sent[0].title.starts_with("🚌"));
    assert_eq!(sent[1].title, "🚧 Avviso new");
    assert_eq!(sent[1].body, "Dettagli new");
    assert_eq!(sent[1].tag.as_deref(), Some("alert-new"));
    assert_eq!(sent[1].url, "/fermata/39");
    assert_eq!(
        h.status(&rule, date(2026, 11, 12)).await.as_deref(),
        Some("sent")
    );

    // The next day, after a restart, the alert is not sent again.
    h.scheduler().tick(utc("2026-11-13T06:40:10Z")).await;
    let sent = h.notifier.all();
    assert_eq!(sent.len(), 3);
    assert!(sent[2].title.starts_with("🚌"));
    let kinds: Vec<String> = h
        .store
        .list_notifications(10)
        .await
        .unwrap()
        .into_iter()
        .map(|record| record.kind)
        .collect();
    assert_eq!(kinds.iter().filter(|kind| *kind == "alert").count(), 1);
}

#[tokio::test]
async fn alerts_are_filtered_by_the_rules_lines() {
    let h = Harness::with_alerts().await;
    h.rule(with_lines(&["9"])).await;
    *h.alerts.0.lock().unwrap() = alerts_read(vec![], "2026-11-12T06:00:00Z");
    h.scheduler().tick(utc("2026-11-12T06:00:30Z")).await;
    // Seen at least once with an alert, so the table is not empty (baseline already done by
    // an alert-less first read? no: an empty feed stores nothing, see the report).
    *h.alerts.0.lock().unwrap() = alerts_read(vec![alert("seed", &["9U"])], "2026-11-12T06:10:00Z");
    h.scheduler().tick(utc("2026-11-12T06:10:30Z")).await;
    *h.alerts.0.lock().unwrap() = alerts_read(
        vec![
            alert("seed", &["9U"]),
            alert("line4", &["4U"]),
            alert("line9", &["9U"]),
            alert("nolines", &[]),
        ],
        "2026-11-12T06:30:00Z",
    );
    h.scheduler().tick(utc("2026-11-12T06:40:10Z")).await;
    let titles: Vec<String> = h.notifier.all().into_iter().map(|n| n.title).collect();
    // Two alerts: each on its own, the ones naming line 9 or no line; never line 4.
    assert!(titles.iter().any(|t| t == "🚧 Avviso line9"), "{titles:?}");
    assert!(
        titles.iter().any(|t| t == "🚧 Avviso nolines"),
        "{titles:?}"
    );
    assert!(!titles.iter().any(|t| t.contains("line4")), "{titles:?}");
}

#[tokio::test]
async fn alerts_older_than_the_rule_are_not_sent() {
    let h = Harness::with_alerts().await;
    *h.alerts.0.lock().unwrap() = alerts_read(vec![alert("base", &["4U"])], "2026-11-12T06:00:00Z");
    h.scheduler().tick(utc("2026-11-12T06:00:30Z")).await;
    *h.alerts.0.lock().unwrap() = alerts_read(
        vec![alert("base", &["4U"]), alert("mid", &["4U"])],
        "2026-11-12T06:20:00Z",
    );
    h.scheduler().tick(utc("2026-11-12T06:20:30Z")).await;
    // The rule is created after "mid" was first seen.
    h.rule_created("2026-11-12T06:30:00Z", at_time(7, 50)).await;
    h.scheduler().tick(utc("2026-11-12T06:50:10Z")).await;
    let titles: Vec<String> = h.notifier.all().into_iter().map(|n| n.title).collect();
    assert_eq!(titles.len(), 1, "{titles:?}");
    assert!(titles[0].starts_with("🚌"));
}

#[tokio::test]
async fn many_new_alerts_are_one_notification() {
    let h = Harness::with_alerts().await;
    h.rule(|_| {}).await;
    *h.alerts.0.lock().unwrap() = alerts_read(vec![alert("base", &[])], "2026-11-12T06:00:00Z");
    let scheduler = h.scheduler();
    scheduler.tick(utc("2026-11-12T06:00:30Z")).await;
    *h.alerts.0.lock().unwrap() = alerts_read(
        vec![
            alert("base", &[]),
            alert("a", &[]),
            alert("b", &[]),
            alert("c", &[]),
        ],
        "2026-11-12T06:30:00Z",
    );
    scheduler.tick(utc("2026-11-12T06:40:10Z")).await;
    let sent = h.notifier.all();
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[1].title, "🚧 3 nuovi avvisi a 39");
    assert_eq!(sent[1].body.lines().count(), 3);
}
