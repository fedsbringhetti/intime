//! The scheduler (Phase 3, step 5): sends the summaries of summary rules at their time, and
//! the new alerts of their stop and lines right after. One task ticks every few seconds and
//! asks [`Scheduler::tick`] what is due; every decision comes from `Schedule::on` and from
//! the `rule_runs` table, never from "today + time".

use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use chrono::{DateTime, NaiveDate, TimeDelta, Utc};
use tokio::time::MissedTickBehavior;

use crate::{
    departures::{Departure, DeparturesError, DeparturesService, Fetched, Filter, Source},
    gtt::alerts::{Alert, Alerts, AlertsService},
    notify::{BoxFuture, MAX_PAYLOAD_BYTES, Notification, Notifier, NotifyError, SendReport},
    rules::{Rule, RuleConfig, RuleStore, SummaryConfig, schedule::Day, schedule::local_date},
    stops::{LineRef, StopIndex},
    time::Clock,
};

pub mod store;

use store::{NewNotification, SchedulerStore};

/// Descriptions of alerts are cut to about this many characters (decision 8).
const ALERT_BODY_CHARS: usize = 400;
/// More new alerts than this at once become a single notification (decision 8).
const MAX_ALERTS_EACH: usize = 2;

/// Run statuses of step 5. `pending`: the upstream failed, retried at each tick until the
/// grace ends (decision 10).
const RUNNING: &str = "running";
const PENDING: &str = "pending";
const SENT: &str = "sent";
const SKIPPED: &str = "skipped";
const FAILED: &str = "failed";

/// Where departures come from; `DeparturesService` in production, a fake in tests.
pub trait DepartureSource: Send + Sync {
    fn departures<'a>(
        &'a self,
        stop_id: &'a str,
    ) -> BoxFuture<'a, Result<Fetched, Arc<DeparturesError>>>;
}

impl DepartureSource for DeparturesService {
    fn departures<'a>(
        &'a self,
        stop_id: &'a str,
    ) -> BoxFuture<'a, Result<Fetched, Arc<DeparturesError>>> {
        Box::pin(self.get(stop_id))
    }
}

/// Where the current alerts come from; `AlertsService` in production.
pub trait AlertSource: Send + Sync {
    fn current(&self) -> Arc<Alerts>;
}

impl AlertSource for AlertsService {
    fn current(&self) -> Arc<Alerts> {
        AlertsService::current(self)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SummaryError {
    #[error("not a summary rule")]
    NotSummary,
    #[error("departures unavailable: {0}")]
    Upstream(Arc<DeparturesError>),
}

/// What a stop looks like in a message.
struct StopLabel {
    name: String,
    code: String,
    lines: Vec<LineRef>,
}

impl StopLabel {
    fn url(&self) -> String {
        format!("/fermata/{}", self.code)
    }
}

async fn stop_label(stops: &StopIndex, stop_id: &str, fetched: Option<&Fetched>) -> StopLabel {
    let info = match stops.get(stop_id).await {
        Ok(info) => info,
        Err(err) => {
            tracing::warn!("cannot read stop {stop_id} from the index: {err}");
            None
        }
    };
    if let Some(info) = info {
        return StopLabel {
            name: info.name,
            code: info.code,
            lines: info.lines,
        };
    }
    let code = stop_id.split_once(':').map_or(stop_id, |(_, code)| code);
    match fetched {
        Some(fetched) => StopLabel {
            name: fetched.data.stop.name.clone(),
            code: fetched.data.stop.code.clone(),
            lines: Vec::new(),
        },
        None => StopLabel {
            name: code.to_owned(),
            code: code.to_owned(),
            lines: Vec::new(),
        },
    }
}

/// The summary notification of a summary rule as it would be sent at `now` (decision 9). Used
/// by the scheduler and by `POST /api/rules/{id}/test`.
pub async fn summary_notification(
    departures: &dyn DepartureSource,
    stops: &StopIndex,
    rule: &Rule,
    now: DateTime<Utc>,
) -> Result<Notification, SummaryError> {
    let RuleConfig::Summary(config) = &rule.config else {
        return Err(SummaryError::NotSummary);
    };
    let fetched = departures
        .departures(&rule.stop_id)
        .await
        .map_err(SummaryError::Upstream)?;
    let label = stop_label(stops, &rule.stop_id, Some(&fetched)).await;
    let filter = Filter {
        lines: config.lines.clone(),
        window: TimeDelta::minutes(i64::from(config.window_minutes)),
        limit: config.max_departures as usize,
    };
    let lines: Vec<String> = fetched
        .data
        .select(&filter, now)
        .into_iter()
        .map(|departure| departure_line(departure, now))
        .collect();
    let body = if lines.is_empty() {
        format!(
            "Nessuna partenza nei prossimi {} minuti.",
            config.window_minutes
        )
    } else {
        lines.join("\n")
    };
    Ok(fit_payload(Notification {
        title: format!("🚌 {} — {} ({})", rule.name, label.name, label.code),
        body,
        url: label.url(),
        tag: Some(format!("rule-{}", rule.id)),
    }))
}

/// `4 → Falchera 07:43 (+2) tra 3'`; no padding, bodies are not monospace.
fn departure_line(departure: &Departure, now: DateTime<Utc>) -> String {
    let mut line = departure.line.clone();
    if let Some(headsign) = &departure.headsign {
        line.push_str(" → ");
        line.push_str(headsign);
    }
    line.push_str(&format!(" {}", departure.expected.format("%H:%M")));
    if departure.canceled {
        line.push_str(" cancellata");
        return line;
    }
    if departure.source == Source::Scheduled {
        line.push_str(" (orario)");
    } else if departure.delay_seconds >= 60 {
        line.push_str(&format!(" (+{})", departure.delay_seconds / 60));
    }
    let minutes = (departure.expected.with_timezone(&Utc) - now)
        .num_minutes()
        .max(0);
    line.push_str(&format!(" tra {minutes}'"));
    line
}

/// The failure notice of a summary (decision 10).
fn failure_notification(rule: &Rule, label: &StopLabel) -> Notification {
    Notification {
        title: format!("⚠️ {}", rule.name),
        body: "Riepilogo non disponibile: dati non raggiungibili.".to_owned(),
        url: label.url(),
        tag: Some(format!("rule-{}", rule.id)),
    }
}

/// The notifications for the new alerts of a rule's stop (decision 8): one each up to two,
/// otherwise a single listing.
fn alert_notifications(rule: &Rule, label: &StopLabel, alerts: &[Alert]) -> Vec<Notification> {
    if alerts.len() > MAX_ALERTS_EACH {
        return vec![fit_payload(Notification {
            title: format!("🚧 {} nuovi avvisi a {}", alerts.len(), label.name),
            body: alerts
                .iter()
                .map(|alert| alert.title.as_str())
                .collect::<Vec<_>>()
                .join("\n"),
            url: label.url(),
            tag: Some(format!("alerts-{}", rule.id)),
        })];
    }
    alerts
        .iter()
        .map(|alert| {
            fit_payload(Notification {
                title: format!("🚧 {}", alert.title),
                body: alert
                    .description
                    .as_deref()
                    .map(|text| cut_chars(text, ALERT_BODY_CHARS))
                    .unwrap_or_default(),
                url: label.url(),
                tag: Some(format!("alert-{}", alert.id)),
            })
        })
        .collect()
}

fn cut_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let mut cut: String = text.chars().take(max).collect();
    cut.push('…');
    cut
}

/// Shortens `text` to at most `max` bytes, at a character boundary.
fn truncate_bytes(text: &mut String, max: usize) {
    if text.len() <= max {
        return;
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
}

/// Cuts lines from the end of the body until the payload fits, so that
/// `NotifyError::PayloadTooLarge` never stops a summary; a single line too long is cut too.
pub fn fit_payload(mut notification: Notification) -> Notification {
    loop {
        let excess = match notification.payload() {
            Err(NotifyError::PayloadTooLarge(size)) => size - MAX_PAYLOAD_BYTES,
            _ => return notification,
        };
        if let Some(position) = notification.body.rfind('\n') {
            notification.body.truncate(position);
        } else if !notification.body.is_empty() {
            let keep = notification.body.len().saturating_sub(excess + 3);
            truncate_bytes(&mut notification.body, keep);
            if !notification.body.is_empty() {
                notification.body.push('…');
            }
        } else if !notification.title.is_empty() {
            let keep = notification.title.len().saturating_sub(excess + 3);
            truncate_bytes(&mut notification.title, keep);
        } else {
            notification.url.clear();
            notification.tag = None;
            return notification;
        }
    }
}

/// Sends `notification` (cut to fit the payload) and records it in the log with the counts
/// (decision 6). A failure to write the log never undoes a send. Test sends use it too:
/// they log but write no `rule_runs` row (decision 11).
pub async fn send_and_log(
    notifier: &dyn Notifier,
    store: &SchedulerStore,
    rule_id: Option<i64>,
    kind: &str,
    notification: Notification,
    now: DateTime<Utc>,
) -> Result<(Notification, SendReport), NotifyError> {
    let notification = fit_payload(notification);
    let report = notifier.send(&notification).await?;
    let record = NewNotification {
        rule_id,
        kind: kind.to_owned(),
        title: notification.title.clone(),
        body: notification.body.clone(),
        url: Some(notification.url.clone()),
        channel: notifier.channel().to_owned(),
        report,
        sent_at: now,
    };
    if let Err(err) = store.log_notification(&record).await {
        tracing::warn!("cannot record the notification in the log: {err}");
    }
    Ok((notification, report))
}

pub struct Scheduler {
    rules: RuleStore,
    store: SchedulerStore,
    departures: Arc<dyn DepartureSource>,
    /// `None` when GTT's data is off: summaries still work, no alerts.
    alerts: Option<Arc<dyn AlertSource>>,
    stops: StopIndex,
    notifier: Arc<dyn Notifier>,
    grace: TimeDelta,
    /// The `read_at` of the alerts already recorded.
    alerts_read_at: Mutex<Option<DateTime<Utc>>>,
    last_purge: Mutex<Option<NaiveDate>>,
}

impl Scheduler {
    pub fn new(
        rules: RuleStore,
        store: SchedulerStore,
        departures: Arc<dyn DepartureSource>,
        alerts: Option<Arc<dyn AlertSource>>,
        stops: StopIndex,
        notifier: Arc<dyn Notifier>,
        grace: Duration,
    ) -> Self {
        Self {
            rules,
            store,
            departures,
            alerts,
            stops,
            notifier,
            grace: TimeDelta::from_std(grace).unwrap_or(TimeDelta::MAX),
            alerts_read_at: Mutex::new(None),
            last_purge: Mutex::new(None),
        }
    }

    /// At startup: a run left `running` by a crash becomes `failed` (decision 4).
    pub async fn recover(&self, now: DateTime<Utc>) {
        match self.store.fail_stale_runs(now).await {
            Ok(0) => {}
            Ok(count) => tracing::warn!("{count} interrupted rule run(s) marked as failed"),
            Err(err) => tracing::error!("cannot mark interrupted rule runs: {err}"),
        }
    }

    /// Does what is due at `now`: today's and yesterday's occurrences of summary rules, one
    /// after another (decisions 1 and 2).
    pub async fn tick(&self, now: DateTime<Utc>) {
        let today = local_date(now);
        self.purge_once_a_day(today, now).await;
        self.record_alerts(now).await;

        let rules = match self.rules.list().await {
            Ok(rules) => rules,
            Err(err) => {
                tracing::error!("scheduler cannot read the rules: {err}");
                return;
            }
        };
        let schedule_dates = [today.pred_opt().unwrap_or(today), today];
        for rule in &rules {
            let RuleConfig::Summary(config) = &rule.config else {
                continue;
            };
            let schedule = rule.schedule();
            for date in schedule_dates {
                if let Day::Due { at } = schedule.on(date) {
                    self.occurrence(rule, config, date, date == today, at.to_utc(), now)
                        .await;
                }
            }
        }
    }

    async fn purge_once_a_day(&self, today: NaiveDate, now: DateTime<Utc>) {
        {
            let mut last = self
                .last_purge
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if *last == Some(today) {
                return;
            }
            *last = Some(today);
        }
        match self.store.purge_old(now).await {
            Ok((notifications, runs)) => {
                tracing::debug!("purged {notifications} notifications and {runs} rule runs");
            }
            Err(err) => tracing::warn!("cannot purge old scheduler rows: {err}"),
        }
    }

    /// Makes "new alert" measurable (decision 7): when the alerts were read again, stores the
    /// ids of the current ones. The very first read is the baseline: handled, never sent.
    async fn record_alerts(&self, now: DateTime<Utc>) {
        let Some(source) = &self.alerts else { return };
        let alerts = source.current();
        let Some(summary) = alerts.summary() else {
            return;
        };
        {
            let last = self
                .alerts_read_at
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if *last == Some(summary.read_at) {
                return;
            }
        }
        let ids: Vec<String> = alerts.all(now).into_iter().map(|alert| alert.id).collect();
        let result = match self.store.alerts_is_empty().await {
            Ok(true) => self.store.insert_alerts_baseline(&ids, now).await,
            Ok(false) => self.store.upsert_seen_alerts(&ids, now).await,
            Err(err) => Err(err),
        };
        match result {
            Ok(()) => {
                *self
                    .alerts_read_at
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(summary.read_at);
            }
            Err(err) => tracing::warn!("cannot record the alerts: {err}"),
        }
    }

    /// One occurrence of a summary rule on `date` (decisions 3, 4 and 10).
    async fn occurrence(
        &self,
        rule: &Rule,
        config: &SummaryConfig,
        date: NaiveDate,
        is_today: bool,
        at: DateTime<Utc>,
        now: DateTime<Utc>,
    ) {
        if now < at {
            return;
        }
        let run = match self.store.get_run(rule.id, date).await {
            Ok(run) => run,
            Err(err) => {
                tracing::error!("cannot read the run of rule {}: {err}", rule.id);
                return;
            }
        };
        let pending = match run {
            None => false,
            Some(run) if run.status == PENDING => true,
            Some(_) => return,
        };
        let late = now - at > self.grace;

        if !pending && (at < rule.created_at || late) {
            // A rule created or moved to a time already past never sends a late summary; the
            // occurrences before the rule existed are not even recorded for yesterday.
            if !is_today && at < rule.created_at {
                return;
            }
            tracing::info!(
                "rule {} ({}): occurrence of {date} skipped",
                rule.id,
                rule.name
            );
            self.record(self.store.insert_run(rule.id, date, SKIPPED, at, now))
                .await;
            return;
        }

        if pending && late {
            // The grace is over: a pending run never sends the summary, even if the upstream
            // answers now. The notice goes out only when found soon enough (2 x grace); after a
            // long stop it would be stale, so the run just becomes `failed`.
            tracing::warn!("rule {}: grace over while the upstream was down", rule.id);
            if now - at <= self.grace * 2 {
                self.record(self.store.set_run_status(rule.id, date, RUNNING, now))
                    .await;
                let label = stop_label(&self.stops, &rule.stop_id, None).await;
                let result = send_and_log(
                    self.notifier.as_ref(),
                    &self.store,
                    Some(rule.id),
                    "summary",
                    failure_notification(rule, &label),
                    now,
                )
                .await;
                if let Err(err) = result {
                    tracing::error!("rule {}: cannot send the failure notice: {err}", rule.id);
                }
            }
            self.record(self.store.set_run_status(rule.id, date, FAILED, now))
                .await;
            return;
        }

        let notification =
            match summary_notification(self.departures.as_ref(), &self.stops, rule, now).await {
                Ok(notification) => notification,
                Err(SummaryError::NotSummary) => return,
                Err(SummaryError::Upstream(err)) => {
                    tracing::warn!("rule {}: departures unavailable, retrying: {err}", rule.id);
                    if !pending {
                        self.record(self.store.insert_run(rule.id, date, PENDING, at, now))
                            .await;
                    }
                    return;
                }
            };

        // Claim the occurrence before sending: a crash leaves `running`, never a duplicate.
        let claimed = if pending {
            self.store.set_run_status(rule.id, date, RUNNING, now).await
        } else {
            self.store
                .insert_run_if_absent(rule.id, date, at, now)
                .await
        };
        if !matches!(claimed, Ok(true)) {
            if let Err(err) = claimed {
                tracing::error!("cannot claim the run of rule {}: {err}", rule.id);
            }
            return;
        }
        let sent = send_and_log(
            self.notifier.as_ref(),
            &self.store,
            Some(rule.id),
            "summary",
            notification,
            now,
        )
        .await;
        match sent {
            Ok((_, report)) => {
                tracing::info!(
                    "rule {} ({}): summary sent to {} device(s)",
                    rule.id,
                    rule.name,
                    report.sent
                );
                self.record(self.store.set_run_status(rule.id, date, SENT, now))
                    .await;
                self.notify_alerts(rule, config, now).await;
            }
            Err(err) => {
                tracing::error!("rule {}: cannot send the summary: {err}", rule.id);
                self.record(self.store.set_run_status(rule.id, date, FAILED, now))
                    .await;
            }
        }
    }

    async fn record<T>(&self, result: impl Future<Output = Result<T, store::SchedulerStoreError>>) {
        if let Err(err) = result.await {
            tracing::error!("scheduler store error: {err}");
        }
    }

    /// The new alerts of the rule's stop and lines, right after its summary (decisions 7, 8).
    async fn notify_alerts(&self, rule: &Rule, config: &SummaryConfig, now: DateTime<Utc>) {
        let Some(source) = &self.alerts else { return };
        let new_ids = match self.store.unnotified_alert_ids(rule.created_at).await {
            Ok(ids) => ids,
            Err(err) => {
                tracing::warn!("cannot read the new alerts: {err}");
                return;
            }
        };
        if new_ids.is_empty() {
            return;
        }
        let label = stop_label(&self.stops, &rule.stop_id, None).await;
        let alerts: Vec<Alert> = source
            .current()
            .for_stop(&rule.stop_id, &label.lines, now)
            .into_iter()
            .filter(|alert| new_ids.contains(&alert.id))
            .filter(|alert| {
                config.lines.is_empty()
                    || alert.lines.is_empty()
                    || config.lines.iter().any(|line| alert.names_line(line))
            })
            .collect();
        // An alert is marked right after its own send, so a failure midway never resends the
        // earlier ones. The grouped notification covers all its alerts at once.
        let grouped = alerts.len() > MAX_ALERTS_EACH;
        let notifications = alert_notifications(rule, &label, &alerts);
        for (index, notification) in notifications.into_iter().enumerate() {
            let result = send_and_log(
                self.notifier.as_ref(),
                &self.store,
                Some(rule.id),
                "alert",
                notification,
                now,
            )
            .await;
            if let Err(err) = result {
                tracing::error!("rule {}: cannot send an alert: {err}", rule.id);
                return;
            }
            let ids: Vec<String> = if grouped {
                alerts.iter().map(|alert| alert.id.clone()).collect()
            } else {
                vec![alerts[index].id.clone()]
            };
            if let Err(err) = self.store.mark_alerts_notified(&ids, now).await {
                tracing::error!("cannot mark the alerts as notified: {err}");
                return;
            }
        }
    }
}

/// Runs forever in the background: marks interrupted runs as failed, then ticks every `every`.
pub fn spawn(
    scheduler: Arc<Scheduler>,
    clock: Arc<dyn Clock>,
    every: Duration,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        scheduler.recover(clock.now()).await;
        let mut interval = tokio::time::interval(every);
        interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            scheduler.tick(clock.now()).await;
        }
    })
}

#[cfg(test)]
mod tests;
