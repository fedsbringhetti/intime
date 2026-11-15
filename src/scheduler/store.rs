//! Storage of the scheduler (Phase 3, step 5): runs of rules, the log of notifications and
//! the alerts already handled.

use chrono::{DateTime, Duration, NaiveDate, Utc};
use serde::Serialize;
use sqlx::SqlitePool;

use crate::notify::SendReport;

/// Rows of `notifications` and `rule_runs` older than this are deleted (decision 13).
pub const RETENTION_DAYS: i64 = 90;

#[derive(Debug, thiserror::Error)]
pub enum SchedulerStoreError {
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),
}

/// A run of a rule on a calendar day.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, sqlx::FromRow)]
pub struct RuleRun {
    pub rule_id: i64,
    pub date: NaiveDate,
    pub status: String,
    pub due_at: DateTime<Utc>,
    /// JSON; `None` for summaries.
    pub state: Option<String>,
    pub updated_at: DateTime<Utc>,
}

/// A notification to record in the log.
#[derive(Debug, Clone)]
pub struct NewNotification {
    pub rule_id: Option<i64>,
    pub kind: String,
    pub title: String,
    pub body: String,
    pub url: Option<String>,
    pub channel: String,
    pub report: SendReport,
    pub sent_at: DateTime<Utc>,
}

/// A logged notification, with the rule's name while the rule still exists.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, sqlx::FromRow)]
pub struct NotificationRecord {
    pub id: i64,
    pub rule_id: Option<i64>,
    pub rule_name: Option<String>,
    pub kind: String,
    pub title: String,
    pub body: String,
    pub url: Option<String>,
    pub channel: String,
    pub sent: i64,
    pub removed: i64,
    pub failed: i64,
    pub sent_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct SchedulerStore {
    pool: SqlitePool,
}

impl SchedulerStore {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    // Rule runs

    /// Inserts the run as `running`; `false` when a row for `(rule_id, date)` already exists,
    /// whatever its status (decision 4).
    pub async fn insert_run_if_absent(
        &self,
        rule_id: i64,
        date: NaiveDate,
        due_at: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> Result<bool, SchedulerStoreError> {
        self.insert_run(rule_id, date, "running", due_at, now).await
    }

    /// Inserts the run with the given status (e.g. `skipped`); `false` when already present.
    pub async fn insert_run(
        &self,
        rule_id: i64,
        date: NaiveDate,
        status: &str,
        due_at: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> Result<bool, SchedulerStoreError> {
        let inserted = sqlx::query(
            "INSERT OR IGNORE INTO rule_runs (rule_id, date, status, due_at, updated_at)
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(rule_id)
        .bind(date)
        .bind(status)
        .bind(due_at)
        .bind(now)
        .execute(&self.pool)
        .await?
        .rows_affected();
        Ok(inserted > 0)
    }

    /// `false` when there is no such run.
    pub async fn set_run_status(
        &self,
        rule_id: i64,
        date: NaiveDate,
        status: &str,
        now: DateTime<Utc>,
    ) -> Result<bool, SchedulerStoreError> {
        let updated = sqlx::query(
            "UPDATE rule_runs SET status = ?, updated_at = ? WHERE rule_id = ? AND date = ?",
        )
        .bind(status)
        .bind(now)
        .bind(rule_id)
        .bind(date)
        .execute(&self.pool)
        .await?
        .rows_affected();
        Ok(updated > 0)
    }

    pub async fn get_run(
        &self,
        rule_id: i64,
        date: NaiveDate,
    ) -> Result<Option<RuleRun>, SchedulerStoreError> {
        Ok(sqlx::query_as(
            "SELECT rule_id, date, status, due_at, state, updated_at FROM rule_runs
             WHERE rule_id = ? AND date = ?",
        )
        .bind(rule_id)
        .bind(date)
        .fetch_optional(&self.pool)
        .await?)
    }

    /// The last `limit` runs of the rule, newest day first.
    pub async fn recent_runs(
        &self,
        rule_id: i64,
        limit: i64,
    ) -> Result<Vec<RuleRun>, SchedulerStoreError> {
        Ok(sqlx::query_as(
            "SELECT rule_id, date, status, due_at, state, updated_at FROM rule_runs
             WHERE rule_id = ? ORDER BY date DESC LIMIT ?",
        )
        .bind(rule_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?)
    }

    /// At startup: a run left `running` by a crash becomes `failed` and is never sent again
    /// (decision 4). Returns how many.
    pub async fn fail_stale_runs(&self, now: DateTime<Utc>) -> Result<u64, SchedulerStoreError> {
        Ok(sqlx::query(
            "UPDATE rule_runs SET status = 'failed', updated_at = ? WHERE status = 'running'",
        )
        .bind(now)
        .execute(&self.pool)
        .await?
        .rows_affected())
    }

    // Notifications log

    /// Records a notification and returns its id.
    pub async fn log_notification(
        &self,
        notification: &NewNotification,
    ) -> Result<i64, SchedulerStoreError> {
        Ok(sqlx::query(
            "INSERT INTO notifications
                 (rule_id, kind, title, body, url, channel, sent, removed, failed, sent_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(notification.rule_id)
        .bind(&notification.kind)
        .bind(&notification.title)
        .bind(&notification.body)
        .bind(&notification.url)
        .bind(&notification.channel)
        .bind(notification.report.sent as i64)
        .bind(notification.report.removed as i64)
        .bind(notification.report.failed as i64)
        .bind(notification.sent_at)
        .execute(&self.pool)
        .await?
        .last_insert_rowid())
    }

    /// The newest `limit` notifications first.
    pub async fn list_notifications(
        &self,
        limit: i64,
    ) -> Result<Vec<NotificationRecord>, SchedulerStoreError> {
        Ok(sqlx::query_as(
            "SELECT n.id, n.rule_id, r.name AS rule_name, n.kind, n.title, n.body, n.url,
                    n.channel, n.sent, n.removed, n.failed, n.sent_at
             FROM notifications n LEFT JOIN rules r ON r.id = n.rule_id
             ORDER BY n.sent_at DESC, n.id DESC LIMIT ?",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?)
    }

    /// Deletes the rows of `notifications` and `rule_runs` older than [`RETENTION_DAYS`]
    /// (runs by their due time). Returns `(notifications, runs)` deleted.
    pub async fn purge_old(&self, now: DateTime<Utc>) -> Result<(u64, u64), SchedulerStoreError> {
        let cutoff = now - Duration::days(RETENTION_DAYS);
        let notifications = sqlx::query("DELETE FROM notifications WHERE sent_at < ?")
            .bind(cutoff)
            .execute(&self.pool)
            .await?
            .rows_affected();
        let runs = sqlx::query("DELETE FROM rule_runs WHERE due_at < ?")
            .bind(cutoff)
            .execute(&self.pool)
            .await?
            .rows_affected();
        Ok((notifications, runs))
    }

    // Notified alerts

    pub async fn alerts_is_empty(&self) -> Result<bool, SchedulerStoreError> {
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM notified_alerts")
            .fetch_one(&self.pool)
            .await?;
        Ok(count == 0)
    }

    /// The baseline (decision 7): stores every id as already handled (`notified_at` equal to
    /// `first_seen_at`), so that none is ever sent.
    pub async fn insert_alerts_baseline(
        &self,
        ids: &[String],
        now: DateTime<Utc>,
    ) -> Result<(), SchedulerStoreError> {
        let mut tx = self.pool.begin().await?;
        for id in ids {
            sqlx::query(
                "INSERT OR IGNORE INTO notified_alerts (alert_id, first_seen_at, notified_at)
                 VALUES (?, ?, ?)",
            )
            .bind(id)
            .bind(now)
            .bind(now)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Records the ids of the current alerts; the ones already known keep their dates.
    pub async fn upsert_seen_alerts(
        &self,
        ids: &[String],
        now: DateTime<Utc>,
    ) -> Result<(), SchedulerStoreError> {
        let mut tx = self.pool.begin().await?;
        for id in ids {
            sqlx::query(
                "INSERT OR IGNORE INTO notified_alerts (alert_id, first_seen_at) VALUES (?, ?)",
            )
            .bind(id)
            .bind(now)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Marks the alerts as handled; a missing id is ignored.
    pub async fn mark_alerts_notified(
        &self,
        ids: &[String],
        now: DateTime<Utc>,
    ) -> Result<(), SchedulerStoreError> {
        let mut tx = self.pool.begin().await?;
        for id in ids {
            sqlx::query(
                "UPDATE notified_alerts SET notified_at = ? WHERE alert_id = ? AND notified_at IS NULL",
            )
            .bind(now)
            .bind(id)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Ids not handled yet and first seen after `after` (a rule's `created_at`), oldest first.
    pub async fn unnotified_alert_ids(
        &self,
        after: DateTime<Utc>,
    ) -> Result<Vec<String>, SchedulerStoreError> {
        Ok(sqlx::query_scalar(
            "SELECT alert_id FROM notified_alerts
             WHERE notified_at IS NULL AND first_seen_at > ?
             ORDER BY first_seen_at, alert_id",
        )
        .bind(after)
        .fetch_all(&self.pool)
        .await?)
    }
}

#[cfg(test)]
mod tests {
    use chrono::{NaiveTime, TimeZone};

    use super::*;
    use crate::{
        db,
        rules::{NewRule, RuleConfig, RuleStore, SummaryConfig},
    };

    fn at(day: u32, hour: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, day, hour, 0, 0).unwrap()
    }

    fn day(day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, day).unwrap()
    }

    async fn setup() -> (SchedulerStore, RuleStore, i64) {
        let pool = db::connect_in_memory().await.unwrap();
        let rules = RuleStore::new(pool.clone());
        let rule = rules
            .create(
                NewRule {
                    name: "Morning".into(),
                    enabled: true,
                    stop_id: "gtt:39".into(),
                    days: vec![],
                    date: None,
                    skip_holidays: true,
                    paused_until: None,
                    config: RuleConfig::Summary(SummaryConfig {
                        time: NaiveTime::from_hms_opt(7, 40, 0).unwrap(),
                        lines: vec![],
                        window_minutes: 30,
                        max_departures: 6,
                    }),
                },
                at(1, 0),
            )
            .await
            .unwrap();
        (SchedulerStore::new(pool), rules, rule.id)
    }

    fn notification(rule_id: Option<i64>, sent_at: DateTime<Utc>) -> NewNotification {
        NewNotification {
            rule_id,
            kind: "summary".into(),
            title: "title".into(),
            body: "body".into(),
            url: Some("/fermata/39".into()),
            channel: "webpush".into(),
            report: SendReport {
                sent: 2,
                removed: 1,
                failed: 0,
            },
            sent_at,
        }
    }

    #[tokio::test]
    async fn run_is_inserted_once_per_day() {
        let (store, _, rule) = setup().await;
        assert!(
            store
                .insert_run_if_absent(rule, day(12), at(12, 6), at(12, 6))
                .await
                .unwrap()
        );
        assert!(
            !store
                .insert_run_if_absent(rule, day(12), at(12, 6), at(12, 7))
                .await
                .unwrap()
        );
        // Whatever the status, a second insert does not succeed.
        store
            .set_run_status(rule, day(12), "sent", at(12, 7))
            .await
            .unwrap();
        assert!(
            !store
                .insert_run_if_absent(rule, day(12), at(12, 6), at(12, 8))
                .await
                .unwrap()
        );
        let run = store.get_run(rule, day(12)).await.unwrap().unwrap();
        assert_eq!(run.status, "sent");
        assert_eq!(run.updated_at, at(12, 7));
        assert_eq!(run.state, None);
        assert!(store.get_run(rule, day(13)).await.unwrap().is_none());
        assert!(
            !store
                .set_run_status(rule, day(13), "sent", at(13, 7))
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn skipped_run_blocks_the_day_and_recent_runs_are_newest_first() {
        let (store, _, rule) = setup().await;
        assert!(
            store
                .insert_run(rule, day(10), "skipped", at(10, 6), at(10, 9))
                .await
                .unwrap()
        );
        store
            .insert_run_if_absent(rule, day(11), at(11, 6), at(11, 6))
            .await
            .unwrap();
        store
            .insert_run_if_absent(rule, day(12), at(12, 6), at(12, 6))
            .await
            .unwrap();
        let runs = store.recent_runs(rule, 2).await.unwrap();
        let dates: Vec<_> = runs.iter().map(|run| run.date).collect();
        assert_eq!(dates, vec![day(12), day(11)]);
        assert_eq!(store.recent_runs(rule, 10).await.unwrap().len(), 3);
        assert!(store.recent_runs(rule + 1, 10).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn stale_running_becomes_failed() {
        let (store, _, rule) = setup().await;
        store
            .insert_run_if_absent(rule, day(11), at(11, 6), at(11, 6))
            .await
            .unwrap();
        store
            .insert_run_if_absent(rule, day(12), at(12, 6), at(12, 6))
            .await
            .unwrap();
        store
            .set_run_status(rule, day(11), "sent", at(11, 7))
            .await
            .unwrap();
        assert_eq!(store.fail_stale_runs(at(13, 0)).await.unwrap(), 1);
        assert_eq!(
            store.get_run(rule, day(11)).await.unwrap().unwrap().status,
            "sent"
        );
        let stale = store.get_run(rule, day(12)).await.unwrap().unwrap();
        assert_eq!(stale.status, "failed");
        assert_eq!(stale.updated_at, at(13, 0));
        assert_eq!(store.fail_stale_runs(at(13, 1)).await.unwrap(), 0);
    }

    #[tokio::test]
    async fn notifications_newest_first_with_rule_name() {
        let (store, rules, rule) = setup().await;
        store
            .log_notification(&notification(Some(rule), at(12, 6)))
            .await
            .unwrap();
        store
            .log_notification(&notification(None, at(12, 8)))
            .await
            .unwrap();
        store
            .log_notification(&notification(Some(rule), at(12, 7)))
            .await
            .unwrap();

        let list = store.list_notifications(2).await.unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].sent_at, at(12, 8));
        assert_eq!(list[0].rule_name, None);
        assert_eq!(list[1].sent_at, at(12, 7));
        assert_eq!(list[1].rule_name.as_deref(), Some("Morning"));
        assert_eq!((list[1].sent, list[1].removed, list[1].failed), (2, 1, 0));
        assert_eq!(list[1].url.as_deref(), Some("/fermata/39"));

        // The log survives a deleted rule, without its name.
        assert!(rules.delete(rule).await.unwrap());
        let list = store.list_notifications(10).await.unwrap();
        assert_eq!(list.len(), 3);
        assert!(
            list.iter()
                .all(|n| n.rule_id.is_none() && n.rule_name.is_none())
        );
    }

    #[tokio::test]
    async fn purge_deletes_rows_older_than_retention() {
        let (store, _, rule) = setup().await;
        let now = at(12, 0);
        let old = now - Duration::days(RETENTION_DAYS + 1);
        let recent = now - Duration::days(RETENTION_DAYS - 1);
        for when in [old, recent] {
            store
                .log_notification(&notification(Some(rule), when))
                .await
                .unwrap();
        }
        store
            .insert_run_if_absent(rule, old.date_naive(), old, old)
            .await
            .unwrap();
        store
            .insert_run_if_absent(rule, recent.date_naive(), recent, recent)
            .await
            .unwrap();
        assert_eq!(store.purge_old(now).await.unwrap(), (1, 1));
        assert_eq!(store.list_notifications(10).await.unwrap().len(), 1);
        assert_eq!(store.recent_runs(rule, 10).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn deleting_a_rule_deletes_its_runs() {
        let (store, rules, rule) = setup().await;
        store
            .insert_run_if_absent(rule, day(12), at(12, 6), at(12, 6))
            .await
            .unwrap();
        rules.delete(rule).await.unwrap();
        assert!(store.recent_runs(rule, 10).await.unwrap().is_empty());
    }

    fn ids(list: &[&str]) -> Vec<String> {
        list.iter().map(|id| (*id).to_owned()).collect()
    }

    #[tokio::test]
    async fn alerts_baseline_then_new_ones() {
        let (store, _, _) = setup().await;
        assert!(store.alerts_is_empty().await.unwrap());
        store
            .insert_alerts_baseline(&ids(&["a", "b"]), at(10, 0))
            .await
            .unwrap();
        assert!(!store.alerts_is_empty().await.unwrap());
        // Baseline alerts are handled: never new, even for a rule created before them.
        assert!(
            store
                .unnotified_alert_ids(at(1, 0))
                .await
                .unwrap()
                .is_empty()
        );

        // "a" is seen again: it keeps its dates. "c" and "d" are new.
        store
            .upsert_seen_alerts(&ids(&["a", "c"]), at(11, 0))
            .await
            .unwrap();
        store
            .upsert_seen_alerts(&ids(&["c", "d"]), at(12, 0))
            .await
            .unwrap();
        assert_eq!(
            store.unnotified_alert_ids(at(1, 0)).await.unwrap(),
            ids(&["c", "d"])
        );
        // Only alerts first seen after the rule was created.
        assert_eq!(
            store.unnotified_alert_ids(at(11, 0)).await.unwrap(),
            ids(&["d"])
        );

        store
            .mark_alerts_notified(&ids(&["c", "missing"]), at(12, 6))
            .await
            .unwrap();
        assert_eq!(
            store.unnotified_alert_ids(at(1, 0)).await.unwrap(),
            ids(&["d"])
        );
        // Marking again does not move the date; empty lists are fine.
        store.mark_alerts_notified(&[], at(12, 7)).await.unwrap();
        store.upsert_seen_alerts(&[], at(12, 7)).await.unwrap();
    }
}
