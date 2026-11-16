//! Rules (Phase 3, section 5): scheduled summaries and trip trackings, their calendar and
//! their storage.

use chrono::{DateTime, NaiveDate, NaiveTime, Utc, Weekday};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

pub mod holidays;
pub mod schedule;

use schedule::Schedule;

/// A rule as stored, with its kind's fields typed in `config`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Rule {
    pub id: i64,
    pub name: String,
    pub enabled: bool,
    pub stop_id: String,
    #[serde(with = "weekdays")]
    pub days: Vec<Weekday>,
    /// One-off trackings only (step 6).
    pub date: Option<NaiveDate>,
    pub skip_holidays: bool,
    /// The last day skipped, included (decision AH).
    pub paused_until: Option<NaiveDate>,
    /// `kind` and `config` in JSON.
    #[serde(flatten)]
    pub config: RuleConfig,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// The fields of each kind of rule, as `{"kind": "summary", "config": {...}}` (decision AG).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "config", rename_all = "snake_case")]
pub enum RuleConfig {
    Summary(SummaryConfig),
    Track(TrackConfig),
}

/// At `time`, the next departures from the stop.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SummaryConfig {
    #[serde(with = "hh_mm")]
    pub time: NaiveTime,
    /// Empty means all lines.
    pub lines: Vec<String>,
    pub window_minutes: u32,
    pub max_departures: u32,
}

/// The run of `line` scheduled at `scheduled_time`, followed every matching day.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrackConfig {
    pub line: String,
    #[serde(with = "hh_mm")]
    pub scheduled_time: NaiveTime,
    /// How far from `scheduled_time` the run may be in the timetable.
    pub tolerance_minutes: u32,
    /// Tracking starts this many minutes before `scheduled_time`.
    pub watch_from_minutes: u32,
    /// Reminders before the expected departure, longest first.
    pub alert_before_minutes: Vec<u32>,
    pub delay_alert_minutes: u32,
    /// One-off trackings: the run followed (OpenTripPlanner's trip id, `gtt:` included).
    /// Recurring rules find their run each day.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trip_id: Option<String>,
}

/// The kind of a rule, as stored in the `kind` column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Summary,
    Track,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Summary => "summary",
            Self::Track => "track",
        }
    }
}

impl RuleConfig {
    pub fn kind(&self) -> Kind {
        match self {
            Self::Summary(_) => Kind::Summary,
            Self::Track(_) => Kind::Track,
        }
    }

    /// The local time of an occurrence: the summary's time, or the run's scheduled time.
    pub fn time(&self) -> NaiveTime {
        match self {
            Self::Summary(summary) => summary.time,
            Self::Track(track) => track.scheduled_time,
        }
    }

    fn to_json(&self) -> Result<String, serde_json::Error> {
        match self {
            Self::Summary(summary) => serde_json::to_string(summary),
            Self::Track(track) => serde_json::to_string(track),
        }
    }

    fn from_json(kind: &str, json: &str) -> Result<Self, RulesError> {
        match kind {
            "summary" => Ok(Self::Summary(serde_json::from_str(json)?)),
            "track" => Ok(Self::Track(serde_json::from_str(json)?)),
            other => Err(RulesError::Kind(other.to_owned())),
        }
    }
}

impl Rule {
    /// The rule's calendar, for its occurrences.
    pub fn schedule(&self) -> Schedule {
        Schedule {
            days: self.days.clone(),
            date: self.date,
            time: self.config.time(),
            skip_holidays: self.skip_holidays,
            paused_until: self.paused_until,
            enabled: self.enabled,
        }
    }
}

/// A rule to create: everything but the id and the timestamps.
#[derive(Debug, Clone)]
pub struct NewRule {
    pub name: String,
    pub enabled: bool,
    pub stop_id: String,
    pub days: Vec<Weekday>,
    pub date: Option<NaiveDate>,
    pub skip_holidays: bool,
    pub paused_until: Option<NaiveDate>,
    pub config: RuleConfig,
}

#[derive(Debug, thiserror::Error)]
pub enum RulesError {
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),
    #[error("invalid stored rule: {0}")]
    Json(#[from] serde_json::Error),
    #[error("unknown rule kind {0:?}")]
    Kind(String),
}

#[derive(sqlx::FromRow)]
struct RuleRow {
    id: i64,
    name: String,
    kind: String,
    enabled: bool,
    stop_id: String,
    days: String,
    date: Option<NaiveDate>,
    skip_holidays: bool,
    paused_until: Option<NaiveDate>,
    config: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl TryFrom<RuleRow> for Rule {
    type Error = RulesError;

    fn try_from(row: RuleRow) -> Result<Self, Self::Error> {
        Ok(Self {
            config: RuleConfig::from_json(&row.kind, &row.config)?,
            days: weekdays::from_json(&row.days)?,
            id: row.id,
            name: row.name,
            enabled: row.enabled,
            stop_id: row.stop_id,
            date: row.date,
            skip_holidays: row.skip_holidays,
            paused_until: row.paused_until,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }
}

const SELECT_RULE: &str = "SELECT id, name, kind, enabled, stop_id, days, date, skip_holidays, \
                           paused_until, config, created_at, updated_at FROM rules";

#[derive(Debug, Clone)]
pub struct RuleStore {
    pool: SqlitePool,
}

impl RuleStore {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// All rules, oldest first.
    pub async fn list(&self) -> Result<Vec<Rule>, RulesError> {
        let rows: Vec<RuleRow> =
            sqlx::query_as(sqlx::AssertSqlSafe(format!("{SELECT_RULE} ORDER BY id")))
                .fetch_all(&self.pool)
                .await?;
        rows.into_iter().map(Rule::try_from).collect()
    }

    pub async fn get(&self, id: i64) -> Result<Option<Rule>, RulesError> {
        let row: Option<RuleRow> =
            sqlx::query_as(sqlx::AssertSqlSafe(format!("{SELECT_RULE} WHERE id = ?")))
                .bind(id)
                .fetch_optional(&self.pool)
                .await?;
        row.map(Rule::try_from).transpose()
    }

    pub async fn create(&self, rule: NewRule, now: DateTime<Utc>) -> Result<Rule, RulesError> {
        let id = sqlx::query(
            "INSERT INTO rules (name, kind, enabled, stop_id, days, date, skip_holidays,
                                paused_until, config, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&rule.name)
        .bind(rule.config.kind().as_str())
        .bind(rule.enabled)
        .bind(&rule.stop_id)
        .bind(weekdays::to_json(&rule.days)?)
        .bind(rule.date)
        .bind(rule.skip_holidays)
        .bind(rule.paused_until)
        .bind(rule.config.to_json()?)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await?
        .last_insert_rowid();

        Ok(Rule {
            id,
            name: rule.name,
            enabled: rule.enabled,
            stop_id: rule.stop_id,
            days: rule.days,
            date: rule.date,
            skip_holidays: rule.skip_holidays,
            paused_until: rule.paused_until,
            config: rule.config,
            created_at: now,
            updated_at: now,
        })
    }

    /// Saves every field of `rule` but its kind, id and creation time; `None` when the rule
    /// does not exist. The caller has checked the fields, the kind included.
    pub async fn update(
        &self,
        rule: &Rule,
        now: DateTime<Utc>,
    ) -> Result<Option<Rule>, RulesError> {
        let updated = sqlx::query(
            "UPDATE rules SET name = ?, enabled = ?, stop_id = ?, days = ?, date = ?,
                              skip_holidays = ?, paused_until = ?, config = ?, updated_at = ?
             WHERE id = ? AND kind = ?",
        )
        .bind(&rule.name)
        .bind(rule.enabled)
        .bind(&rule.stop_id)
        .bind(weekdays::to_json(&rule.days)?)
        .bind(rule.date)
        .bind(rule.skip_holidays)
        .bind(rule.paused_until)
        .bind(rule.config.to_json()?)
        .bind(now)
        .bind(rule.id)
        .bind(rule.config.kind().as_str())
        .execute(&self.pool)
        .await?
        .rows_affected();
        if updated == 0 {
            return Ok(None);
        }
        self.get(rule.id).await
    }

    /// `false` when the rule does not exist.
    pub async fn delete(&self, id: i64) -> Result<bool, RulesError> {
        let deleted = sqlx::query("DELETE FROM rules WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?
            .rows_affected();
        Ok(deleted > 0)
    }
}

/// Days of the week as `["mon", "tue", ...]`, in the API and in the database.
pub mod weekdays {
    use chrono::Weekday;
    use serde::{Deserialize, Deserializer, Serializer, de::Error, ser::SerializeSeq};

    const NAMES: [(&str, Weekday); 7] = [
        ("mon", Weekday::Mon),
        ("tue", Weekday::Tue),
        ("wed", Weekday::Wed),
        ("thu", Weekday::Thu),
        ("fri", Weekday::Fri),
        ("sat", Weekday::Sat),
        ("sun", Weekday::Sun),
    ];

    pub fn name(day: Weekday) -> &'static str {
        NAMES[day.num_days_from_monday() as usize].0
    }

    pub fn parse(name: &str) -> Option<Weekday> {
        NAMES
            .iter()
            .find(|(other, _)| *other == name)
            .map(|(_, day)| *day)
    }

    pub fn serialize<S: Serializer>(days: &[Weekday], serializer: S) -> Result<S::Ok, S::Error> {
        let mut seq = serializer.serialize_seq(Some(days.len()))?;
        for day in days {
            seq.serialize_element(name(*day))?;
        }
        seq.end()
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Vec<Weekday>, D::Error> {
        Vec::<String>::deserialize(deserializer)?
            .iter()
            .map(|day| {
                parse(day).ok_or_else(|| {
                    D::Error::custom(format!(
                        "invalid day {day:?}, expected one of mon, tue, wed, thu, fri, sat, sun"
                    ))
                })
            })
            .collect()
    }

    pub(super) fn to_json(days: &[Weekday]) -> Result<String, serde_json::Error> {
        serde_json::to_string(&days.iter().map(|day| name(*day)).collect::<Vec<_>>())
    }

    pub(super) fn from_json(json: &str) -> Result<Vec<Weekday>, serde_json::Error> {
        deserialize(&mut serde_json::Deserializer::from_str(json))
    }
}

/// A local time as `"07:40"`.
pub mod hh_mm {
    use chrono::NaiveTime;
    use serde::{Deserialize, Deserializer, Serializer, de::Error};

    const FORMAT: &str = "%H:%M";

    pub fn parse(text: &str) -> Option<NaiveTime> {
        // Exactly two digits each, so "7:40" and "07:40:00" are refused alike.
        let valid = text.len() == 5 && text.as_bytes()[2] == b':';
        valid
            .then(|| NaiveTime::parse_from_str(text, FORMAT).ok())
            .flatten()
    }

    pub fn serialize<S: Serializer>(time: &NaiveTime, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(&time.format(FORMAT))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<NaiveTime, D::Error> {
        let text = String::deserialize(deserializer)?;
        parse(&text)
            .ok_or_else(|| D::Error::custom(format!("invalid time {text:?}, expected HH:MM")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary() -> RuleConfig {
        RuleConfig::Summary(SummaryConfig {
            time: NaiveTime::from_hms_opt(7, 40, 0).unwrap(),
            lines: vec!["4".into(), "15".into()],
            window_minutes: 30,
            max_departures: 6,
        })
    }

    #[test]
    fn config_as_kind_and_config() {
        let json = serde_json::to_value(summary()).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "kind": "summary",
                "config": {"time": "07:40", "lines": ["4", "15"], "window_minutes": 30, "max_departures": 6}
            })
        );
        let stored = summary().to_json().unwrap();
        assert_eq!(
            RuleConfig::from_json("summary", &stored).unwrap(),
            summary()
        );
        assert!(RuleConfig::from_json("track", &stored).is_err());
        assert!(RuleConfig::from_json("other", &stored).is_err());
    }

    #[test]
    fn times_are_hh_mm() {
        let time = |h, m| NaiveTime::from_hms_opt(h, m, 0);
        assert_eq!(hh_mm::parse("07:40"), time(7, 40));
        assert_eq!(hh_mm::parse("00:00"), time(0, 0));
        assert_eq!(hh_mm::parse("23:59"), time(23, 59));
        for invalid in [
            "7:40", "07:40:00", "24:00", "07:60", "0740", "", "ab:cd", "07.40",
        ] {
            assert_eq!(hh_mm::parse(invalid), None, "{invalid}");
        }
    }

    #[test]
    fn days_are_short_names() {
        let days = vec![Weekday::Mon, Weekday::Sun];
        assert_eq!(weekdays::to_json(&days).unwrap(), r#"["mon","sun"]"#);
        assert_eq!(weekdays::from_json(r#"["mon","sun"]"#).unwrap(), days);
        assert!(weekdays::from_json(r#"["Mon"]"#).is_err());
        assert!(weekdays::from_json(r#"["monday"]"#).is_err());
    }
}
