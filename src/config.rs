use std::{net::SocketAddr, path::PathBuf, time::Duration};

use chrono::NaiveTime;

use crate::{api::auth::ApiToken, notify::vapid::VapidKey, stops::Feeds};

const DEFAULT_BIND_ADDR: &str = "0.0.0.0:8080";
const DEFAULT_UPSTREAM_URL: &str = "https://plan.muoversiatorino.it/otp/routers/mato/index/graphql";
const DEFAULT_UPSTREAM_TIMEOUT_SECS: &str = "10";
const DEFAULT_DEPARTURES_CACHE_SECS: &str = "20";
const DEFAULT_DATA_DIR: &str = "data";
const DEFAULT_STOPS_SYNC_TIME: &str = "04:00";
const DEFAULT_FEEDS: &str = "gtt";
const DEFAULT_GTT_REALTIME_URL: &str = "https://percorsieorari.gtt.to.it/das_gtfsrt/";
const DEFAULT_GTT_TRIP_UPDATES_CACHE_SECS: &str = "20";
const DEFAULT_GTT_ALERTS_POLL_SECS: &str = "300";
const DEFAULT_SCHEDULER_TICK_SECS: &str = "15";
const DEFAULT_MISSED_RUN_GRACE_MINUTES: &str = "5";
const DEFAULT_GTT_GTFS_URL: &str = "https://www.gtt.to.it/open_data/gtt_gtfs.zip";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub bind_addr: SocketAddr,
    pub upstream_url: reqwest::Url,
    pub upstream_timeout: Duration,
    pub departures_cache_ttl: Duration,
    pub data_dir: PathBuf,
    /// Local time (Europe/Rome) of the nightly stop index sync.
    pub stops_sync_time: NaiveTime,
    /// Feeds searched when a request does not specify any.
    pub default_feeds: Feeds,
    /// When set, `/api/*` requires `Authorization: Bearer <token>`.
    pub api_token: Option<ApiToken>,
    /// Folder of GTT's real-time feeds. `None` disables GTT's data:
    /// departures use OpenTripPlanner alone.
    pub gtt_realtime_url: Option<reqwest::Url>,
    pub gtt_trip_updates_cache_ttl: Duration,
    /// How often GTT's alerts are read (when `gtt_realtime_url` is set).
    pub gtt_alerts_poll: Duration,
    /// How often the scheduler looks for rules that are due.
    pub scheduler_tick: Duration,
    /// A summary found due this late or less is still sent; later it is skipped.
    pub missed_run_grace: Duration,
    /// GTT's GTFS static archive, for its `stops.txt`. `None` disables it: alerts are then
    /// linked by line only.
    pub gtt_gtfs_url: Option<reqwest::Url>,
    /// Private key of the Web Push (VAPID) identity. `None` disables Web Push: notifications
    /// are only written to the log.
    pub vapid_private_key: Option<VapidKey>,
    /// Contact of the VAPID identity (`mailto:` or `https:`); set whenever the key is.
    pub vapid_subject: Option<String>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ConfigError {
    #[error("invalid value {value:?} for {var}: {reason}")]
    Invalid {
        var: &'static str,
        value: String,
        reason: String,
    },
}

impl Config {
    /// Reads the configuration from the process environment.
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_lookup(|name| std::env::var(name).ok())
    }

    /// Reads the configuration through `lookup`, so tests do not touch the real environment.
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Result<Self, ConfigError> {
        let (vapid_private_key, vapid_subject) = parse_vapid(&lookup)?;
        Ok(Self {
            bind_addr: parse_or_default(&lookup, "BIND_ADDR", DEFAULT_BIND_ADDR)?,
            upstream_url: parse_or_default(&lookup, "UPSTREAM_URL", DEFAULT_UPSTREAM_URL)?,
            upstream_timeout: Duration::from_secs(parse_or_default(
                &lookup,
                "UPSTREAM_TIMEOUT_SECS",
                DEFAULT_UPSTREAM_TIMEOUT_SECS,
            )?),
            departures_cache_ttl: Duration::from_secs(parse_or_default(
                &lookup,
                "DEPARTURES_CACHE_SECS",
                DEFAULT_DEPARTURES_CACHE_SECS,
            )?),
            data_dir: parse_or_default(&lookup, "DATA_DIR", DEFAULT_DATA_DIR)?,
            stops_sync_time: parse_with(
                &lookup,
                "STOPS_SYNC_TIME",
                DEFAULT_STOPS_SYNC_TIME,
                |value| NaiveTime::parse_from_str(value, "%H:%M").map_err(|err| err.to_string()),
            )?,
            default_feeds: parse_or_default(&lookup, "DEFAULT_FEEDS", DEFAULT_FEEDS)?,
            // Unset or empty means no token.
            api_token: match lookup("API_TOKEN").filter(|value| !value.trim().is_empty()) {
                Some(value) => Some(value.parse().map_err(|reason| ConfigError::Invalid {
                    var: "API_TOKEN",
                    value: "***".to_owned(),
                    reason,
                })?),
                None => None,
            },
            gtt_realtime_url: parse_with(
                &lookup,
                "GTT_REALTIME_URL",
                DEFAULT_GTT_REALTIME_URL,
                parse_optional_url,
            )?,
            gtt_trip_updates_cache_ttl: Duration::from_secs(parse_or_default(
                &lookup,
                "GTT_TRIP_UPDATES_CACHE_SECS",
                DEFAULT_GTT_TRIP_UPDATES_CACHE_SECS,
            )?),
            gtt_alerts_poll: Duration::from_secs(parse_with(
                &lookup,
                "GTT_ALERTS_POLL_SECS",
                DEFAULT_GTT_ALERTS_POLL_SECS,
                |value| match value.parse::<u64>() {
                    Ok(0) => Err("must be at least 1".to_owned()),
                    Ok(secs) => Ok(secs),
                    Err(err) => Err(err.to_string()),
                },
            )?),
            scheduler_tick: Duration::from_secs(parse_with(
                &lookup,
                "SCHEDULER_TICK_SECS",
                DEFAULT_SCHEDULER_TICK_SECS,
                at_least_one,
            )?),
            missed_run_grace: Duration::from_secs(
                60 * parse_with(
                    &lookup,
                    "MISSED_RUN_GRACE_MINUTES",
                    DEFAULT_MISSED_RUN_GRACE_MINUTES,
                    at_least_one,
                )?,
            ),
            gtt_gtfs_url: parse_with(
                &lookup,
                "GTT_GTFS_URL",
                DEFAULT_GTT_GTFS_URL,
                parse_optional_url,
            )?,
            vapid_private_key,
            vapid_subject,
        })
    }
}

/// `VAPID_PRIVATE_KEY` and `VAPID_SUBJECT`: the subject is required with a key and ignored
/// without one.
fn parse_vapid(
    lookup: &impl Fn(&str) -> Option<String>,
) -> Result<(Option<VapidKey>, Option<String>), ConfigError> {
    let Some(key) = lookup("VAPID_PRIVATE_KEY").filter(|value| !value.trim().is_empty()) else {
        return Ok((None, None));
    };
    let key = key.parse().map_err(|reason| ConfigError::Invalid {
        var: "VAPID_PRIVATE_KEY",
        value: "***".to_owned(),
        reason,
    })?;
    let subject = lookup("VAPID_SUBJECT").unwrap_or_default();
    let subject = subject.trim();
    let rest = subject
        .strip_prefix("mailto:")
        .or_else(|| subject.strip_prefix("https:"));
    if rest.is_none_or(|rest| rest.is_empty()) {
        return Err(ConfigError::Invalid {
            var: "VAPID_SUBJECT",
            value: subject.to_owned(),
            reason: "required with VAPID_PRIVATE_KEY, must start with mailto: or https:".to_owned(),
        });
    }
    Ok((Some(key), Some(subject.to_owned())))
}

/// An empty value means "disabled".
fn parse_optional_url(value: &str) -> Result<Option<reqwest::Url>, String> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    value
        .parse::<reqwest::Url>()
        .map(Some)
        .map_err(|err| err.to_string())
}

fn parse_or_default<T>(
    lookup: &impl Fn(&str) -> Option<String>,
    var: &'static str,
    default: &str,
) -> Result<T, ConfigError>
where
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    parse_with(lookup, var, default, |value| {
        value.parse().map_err(|err: T::Err| err.to_string())
    })
}

fn at_least_one(value: &str) -> Result<u64, String> {
    match value.parse::<u64>() {
        Ok(0) => Err("must be at least 1".to_owned()),
        Ok(number) => Ok(number),
        Err(err) => Err(err.to_string()),
    }
}

fn parse_with<T>(
    lookup: &impl Fn(&str) -> Option<String>,
    var: &'static str,
    default: &str,
    parse: impl Fn(&str) -> Result<T, String>,
) -> Result<T, ConfigError> {
    let value = lookup(var).unwrap_or_else(|| default.to_owned());
    parse(&value).map_err(|reason| ConfigError::Invalid { var, value, reason })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn config_from(vars: &[(&str, &str)]) -> Result<Config, ConfigError> {
        let vars: HashMap<String, String> = vars
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        Config::from_lookup(|name| vars.get(name).cloned())
    }

    #[test]
    fn uses_defaults_when_nothing_is_set() {
        let config = config_from(&[]).unwrap();
        assert_eq!(config.bind_addr, "0.0.0.0:8080".parse().unwrap());
        assert_eq!(config.upstream_url.as_str(), DEFAULT_UPSTREAM_URL);
        assert_eq!(config.upstream_timeout, Duration::from_secs(10));
        assert_eq!(config.departures_cache_ttl, Duration::from_secs(20));
        assert_eq!(config.data_dir, PathBuf::from("data"));
        assert_eq!(
            config.stops_sync_time,
            NaiveTime::from_hms_opt(4, 0, 0).unwrap()
        );
        assert_eq!(config.default_feeds, Feeds::Only(vec!["gtt".to_owned()]));
        assert_eq!(config.api_token, None);
        assert_eq!(
            config.gtt_realtime_url.unwrap().as_str(),
            DEFAULT_GTT_REALTIME_URL
        );
        assert_eq!(config.gtt_trip_updates_cache_ttl, Duration::from_secs(20));
        assert_eq!(config.gtt_alerts_poll, Duration::from_secs(300));
        assert_eq!(config.gtt_gtfs_url.unwrap().as_str(), DEFAULT_GTT_GTFS_URL);
        assert_eq!(config.vapid_private_key, None);
        assert_eq!(config.vapid_subject, None);
    }

    #[test]
    fn reads_vapid_settings() {
        let key = VapidKey::generate();
        let private = key.private_key_base64();

        // Unset or empty: no Web Push, and the subject is ignored.
        for vars in [
            vec![],
            vec![("VAPID_PRIVATE_KEY", "")],
            vec![("VAPID_PRIVATE_KEY", " "), ("VAPID_SUBJECT", "nonsense")],
        ] {
            let config = config_from(&vars).unwrap();
            assert_eq!(config.vapid_private_key, None);
            assert_eq!(config.vapid_subject, None);
        }

        for subject in ["mailto:me@example.com", "https://example.com/contact"] {
            let config = config_from(&[
                ("VAPID_PRIVATE_KEY", private.as_str()),
                ("VAPID_SUBJECT", subject),
            ])
            .unwrap();
            assert_eq!(config.vapid_private_key, Some(key.clone()));
            assert_eq!(config.vapid_subject.as_deref(), Some(subject));
        }

        // Invalid key: the error does not show the value.
        let err = config_from(&[
            ("VAPID_PRIVATE_KEY", "bad key"),
            ("VAPID_SUBJECT", "mailto:me@example.com"),
        ])
        .unwrap_err();
        assert!(matches!(
            err,
            ConfigError::Invalid {
                var: "VAPID_PRIVATE_KEY",
                ..
            }
        ));
        assert!(!err.to_string().contains("bad key"), "{err}");

        // Key without a usable subject.
        for vars in [
            vec![("VAPID_PRIVATE_KEY", private.as_str())],
            vec![
                ("VAPID_PRIVATE_KEY", private.as_str()),
                ("VAPID_SUBJECT", "me@example.com"),
            ],
            vec![
                ("VAPID_PRIVATE_KEY", private.as_str()),
                ("VAPID_SUBJECT", "mailto:"),
            ],
            vec![
                ("VAPID_PRIVATE_KEY", private.as_str()),
                ("VAPID_SUBJECT", "https:"),
            ],
        ] {
            let err = config_from(&vars).unwrap_err();
            assert!(matches!(
                err,
                ConfigError::Invalid {
                    var: "VAPID_SUBJECT",
                    ..
                }
            ));
        }
    }

    #[test]
    fn reads_scheduler_settings() {
        let config = config_from(&[]).unwrap();
        assert_eq!(config.scheduler_tick, Duration::from_secs(15));
        assert_eq!(config.missed_run_grace, Duration::from_secs(300));
        let config = config_from(&[
            ("SCHEDULER_TICK_SECS", "5"),
            ("MISSED_RUN_GRACE_MINUTES", "10"),
        ])
        .unwrap();
        assert_eq!(config.scheduler_tick, Duration::from_secs(5));
        assert_eq!(config.missed_run_grace, Duration::from_secs(600));
        for var in ["SCHEDULER_TICK_SECS", "MISSED_RUN_GRACE_MINUTES"] {
            for value in ["0", "five"] {
                let err = config_from(&[(var, value)]).unwrap_err();
                assert!(matches!(err, ConfigError::Invalid { var: v, .. } if v == var));
            }
        }
    }

    #[test]
    fn reads_gtt_alerts_and_static_settings() {
        let config = config_from(&[
            ("GTT_ALERTS_POLL_SECS", "60"),
            ("GTT_GTFS_URL", "http://localhost:9999/gtfs.zip"),
        ])
        .unwrap();
        assert_eq!(config.gtt_alerts_poll, Duration::from_secs(60));
        assert_eq!(
            config.gtt_gtfs_url.unwrap().as_str(),
            "http://localhost:9999/gtfs.zip"
        );
        assert_eq!(
            config_from(&[("GTT_GTFS_URL", "")]).unwrap().gtt_gtfs_url,
            None
        );
        for value in ["0", "five"] {
            let err = config_from(&[("GTT_ALERTS_POLL_SECS", value)]).unwrap_err();
            assert!(matches!(
                err,
                ConfigError::Invalid {
                    var: "GTT_ALERTS_POLL_SECS",
                    ..
                }
            ));
        }
    }

    #[test]
    fn reads_gtt_settings() {
        let config = config_from(&[
            ("GTT_REALTIME_URL", "http://localhost:9999/feeds"),
            ("GTT_TRIP_UPDATES_CACHE_SECS", "5"),
        ])
        .unwrap();
        assert_eq!(
            config.gtt_realtime_url.unwrap().as_str(),
            "http://localhost:9999/feeds"
        );
        assert_eq!(config.gtt_trip_updates_cache_ttl, Duration::from_secs(5));

        let disabled = config_from(&[("GTT_REALTIME_URL", " ")]).unwrap();
        assert_eq!(disabled.gtt_realtime_url, None);

        let err = config_from(&[("GTT_REALTIME_URL", "not a url")]).unwrap_err();
        assert!(matches!(
            err,
            ConfigError::Invalid {
                var: "GTT_REALTIME_URL",
                ..
            }
        ));
    }

    #[test]
    fn reads_api_token_without_leaking_it() {
        assert_eq!(config_from(&[("API_TOKEN", "")]).unwrap().api_token, None);
        assert_eq!(
            config_from(&[("API_TOKEN", "s3cret")]).unwrap().api_token,
            Some("s3cret".parse().unwrap())
        );
        let err = config_from(&[("API_TOKEN", "two words")]).unwrap_err();
        assert!(!err.to_string().contains("two words"), "{err}");
    }

    #[test]
    fn reads_stop_index_settings() {
        let config = config_from(&[
            ("DATA_DIR", "/data"),
            ("STOPS_SYNC_TIME", "03:30"),
            ("DEFAULT_FEEDS", "all"),
        ])
        .unwrap();
        assert_eq!(config.data_dir, PathBuf::from("/data"));
        assert_eq!(
            config.stops_sync_time,
            NaiveTime::from_hms_opt(3, 30, 0).unwrap()
        );
        assert_eq!(config.default_feeds, Feeds::All);
    }

    #[test]
    fn rejects_invalid_sync_time_and_feeds() {
        for (var, value) in [("STOPS_SYNC_TIME", "4am"), ("DEFAULT_FEEDS", "gtt;drop")] {
            let err = config_from(&[(var, value)]).unwrap_err();
            assert!(
                matches!(err, ConfigError::Invalid { var: v, .. } if v == var),
                "{var}"
            );
        }
    }

    #[test]
    fn reads_upstream_settings() {
        let config = config_from(&[
            ("UPSTREAM_URL", "http://localhost:9999/graphql"),
            ("UPSTREAM_TIMEOUT_SECS", "3"),
            ("DEPARTURES_CACHE_SECS", "45"),
        ])
        .unwrap();
        assert_eq!(
            config.upstream_url.as_str(),
            "http://localhost:9999/graphql"
        );
        assert_eq!(config.upstream_timeout, Duration::from_secs(3));
        assert_eq!(config.departures_cache_ttl, Duration::from_secs(45));
    }

    #[test]
    fn rejects_invalid_numbers_and_urls() {
        let err = config_from(&[("UPSTREAM_TIMEOUT_SECS", "ten")]).unwrap_err();
        assert!(matches!(
            err,
            ConfigError::Invalid {
                var: "UPSTREAM_TIMEOUT_SECS",
                ..
            }
        ));
        let err = config_from(&[("UPSTREAM_URL", "not a url")]).unwrap_err();
        assert!(matches!(
            err,
            ConfigError::Invalid {
                var: "UPSTREAM_URL",
                ..
            }
        ));
    }

    #[test]
    fn reads_bind_addr() {
        let config = config_from(&[("BIND_ADDR", "127.0.0.1:9000")]).unwrap();
        assert_eq!(config.bind_addr, "127.0.0.1:9000".parse().unwrap());
    }

    #[test]
    fn rejects_invalid_bind_addr() {
        let err = config_from(&[("BIND_ADDR", "not-an-address")]).unwrap_err();
        assert!(matches!(
            err,
            ConfigError::Invalid {
                var: "BIND_ADDR",
                ..
            }
        ));
    }
}
