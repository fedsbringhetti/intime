use std::{
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    process::ExitCode,
    sync::Arc,
    time::Duration,
};

use intime::{
    api::{self, AppState},
    config::Config,
    db,
    departures::DeparturesService,
    favorites::FavoriteStore,
    gtt::{
        GttRealtime,
        alerts::{self, AlertsService},
        static_data::{self, GttStatic},
    },
    notify::{Notifier, log::LogNotifier, vapid::VapidKey, webpush::WebPushNotifier},
    push::{EndpointPolicy, SubscriptionStore},
    rules::RuleStore,
    stops::{StopIndex, sync},
    time::{Clock, SystemClock},
    upstream::UpstreamClient,
    web::WebAssets,
};
use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();

    // Needs no configuration, which may well be invalid or incomplete when this is run.
    if let ["vapid-keys"] = args.as_slice() {
        return vapid_keys();
    }

    let config = match Config::from_env() {
        Ok(config) => config,
        Err(err) => {
            tracing::error!("configuration error: {err}");
            return ExitCode::FAILURE;
        }
    };

    match args.as_slice() {
        [] => serve(config).await,
        ["healthcheck"] => healthcheck(config.bind_addr).await,
        _ => {
            eprintln!("usage: intime [healthcheck | vapid-keys]");
            ExitCode::from(2)
        }
    }
}

/// Prints a new VAPID key pair: the private key goes in `VAPID_PRIVATE_KEY`, the public one is
/// derived from it by the service, and printed here only for reference.
fn vapid_keys() -> ExitCode {
    let key = VapidKey::generate();
    println!("VAPID_PRIVATE_KEY={}", key.private_key_base64());
    println!("# public key: {}", key.public_key_base64());
    println!("# also set VAPID_SUBJECT=mailto:you@example.com (or an https: URL)");
    ExitCode::SUCCESS
}

/// Runs the service until Ctrl+C or SIGTERM.
async fn serve(config: Config) -> ExitCode {
    let upstream = match UpstreamClient::new(config.upstream_url.clone(), config.upstream_timeout) {
        Ok(upstream) => upstream,
        Err(err) => {
            tracing::error!("cannot create the upstream client: {err}");
            return ExitCode::FAILURE;
        }
    };
    let pool = match db::connect(&config.data_dir).await {
        Ok(pool) => pool,
        Err(err) => {
            tracing::error!(
                "cannot open the database in {}: {err}",
                config.data_dir.display()
            );
            return ExitCode::FAILURE;
        }
    };
    let gtt = match &config.gtt_realtime_url {
        Some(url) => match GttRealtime::new(url, config.gtt_trip_updates_cache_ttl) {
            Ok(gtt) => Some(Arc::new(gtt)),
            Err(err) => {
                tracing::error!("cannot create the GTT client: {err}");
                return ExitCode::FAILURE;
            }
        },
        None => {
            tracing::info!("GTT_REALTIME_URL is empty: departures use OpenTripPlanner only");
            None
        }
    };
    let clock: Arc<dyn Clock> = Arc::new(SystemClock);
    let stops = StopIndex::new(pool.clone());
    let favorites = FavoriteStore::new(pool.clone());
    let rules = RuleStore::new(pool.clone());
    let subscriptions = SubscriptionStore::new(pool.clone());
    let (notifier, vapid_public_key): (Arc<dyn Notifier>, Option<Arc<str>>) =
        match (&config.vapid_private_key, &config.vapid_subject) {
            (Some(key), Some(subject)) => {
                match WebPushNotifier::new(
                    subscriptions.clone(),
                    key.clone(),
                    subject.clone(),
                    clock.clone(),
                ) {
                    Ok(notifier) => {
                        tracing::info!("notifications: Web Push");
                        (Arc::new(notifier), Some(key.public_key_base64().into()))
                    }
                    Err(err) => {
                        tracing::error!("cannot create the Web Push client: {err}");
                        return ExitCode::FAILURE;
                    }
                }
            }
            _ => {
                tracing::info!("VAPID_PRIVATE_KEY is not set: notifications only go to the log");
                (Arc::new(LogNotifier), None)
            }
        };
    sync::spawn(
        stops.clone(),
        upstream.clone(),
        clock.clone(),
        config.stops_sync_time,
    );
    // GTT's static data only serves the alerts, which come with GTT's real-time feeds.
    let gtt_static = match (&gtt, &config.gtt_gtfs_url) {
        (Some(_), Some(url)) => match GttStatic::new(pool.clone(), url.clone()) {
            Ok(gtt_static) => Some(gtt_static),
            Err(err) => {
                tracing::error!("cannot create the GTT static data client: {err}");
                return ExitCode::FAILURE;
            }
        },
        _ => None,
    };
    let alerts = gtt.as_ref().map(|gtt| {
        Arc::new(AlertsService::new(
            gtt.clone(),
            stops.clone(),
            gtt_static.clone(),
        ))
    });
    if let Some(alerts) = &alerts {
        alerts::spawn(alerts.clone(), clock.clone(), config.gtt_alerts_poll);
    }
    if let Some(gtt_static) = &gtt_static {
        let (check_time, _) = config
            .stops_sync_time
            .overflowing_add_signed(static_data::CHECK_AFTER_STOPS_SYNC);
        // New static data can link alerts to more stops: read them again at once.
        let alerts = alerts.clone();
        static_data::spawn(gtt_static.clone(), clock.clone(), check_time, move || {
            if let Some(alerts) = &alerts {
                alerts.refresh_soon();
            }
        });
    }

    if config.api_token.is_none() {
        tracing::warn!("API_TOKEN is not set: /api/* is open to anyone who can reach the service");
    }
    let web = WebAssets::dist();
    if !web.is_built() {
        tracing::warn!("web app not built: run `npm run build` in web/ (the API works anyway)");
    }
    let upstream_health = upstream.health();
    let state = AppState {
        departures: Arc::new(DeparturesService::new(
            upstream,
            gtt.clone(),
            config.departures_cache_ttl,
            clock.clone(),
        )),
        stops,
        favorites,
        rules,
        subscriptions,
        notifier,
        vapid_public_key,
        push_endpoints: EndpointPolicy::push_services(),
        default_feeds: config.default_feeds.clone(),
        upstream_health,
        gtt,
        alerts,
        gtt_static,
        api_token: config.api_token.clone(),
        clock,
        web,
    };

    let listener = match TcpListener::bind(config.bind_addr).await {
        Ok(listener) => listener,
        Err(err) => {
            tracing::error!("cannot bind {}: {err}", config.bind_addr);
            return ExitCode::FAILURE;
        }
    };
    tracing::info!("listening on {}", config.bind_addr);

    if let Err(err) = axum::serve(listener, api::router(state))
        .with_graceful_shutdown(shutdown_signal())
        .await
    {
        tracing::error!("server error: {err}");
        return ExitCode::FAILURE;
    }

    tracing::info!("shut down");
    ExitCode::SUCCESS
}

/// Checks that the local service answers `/health`, for the Docker health check (the image has
/// no curl). Exit code 0 when healthy.
async fn healthcheck(bind_addr: SocketAddr) -> ExitCode {
    let url = format!("http://{}/health", local_addr(bind_addr));
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(3))
        .build()
    {
        Ok(client) => client,
        Err(err) => {
            eprintln!("healthcheck: {err}");
            return ExitCode::FAILURE;
        }
    };
    match client.get(&url).send().await {
        Ok(response) if response.status().is_success() => ExitCode::SUCCESS,
        Ok(response) => {
            eprintln!("healthcheck: {url} returned {}", response.status());
            ExitCode::FAILURE
        }
        Err(err) => {
            eprintln!("healthcheck: {url}: {err}");
            ExitCode::FAILURE
        }
    }
}

/// The address to reach a server bound to `bind_addr` from the same machine: a wildcard
/// address (`0.0.0.0`, `::`) becomes loopback.
fn local_addr(bind_addr: SocketAddr) -> SocketAddr {
    let ip = match bind_addr.ip() {
        IpAddr::V4(ip) if ip.is_unspecified() => IpAddr::V4(Ipv4Addr::LOCALHOST),
        IpAddr::V6(ip) if ip.is_unspecified() => IpAddr::V6(Ipv6Addr::LOCALHOST),
        ip => ip,
    };
    SocketAddr::new(ip, bind_addr.port())
}

/// Resolves on Ctrl+C or SIGTERM (what `docker stop` sends).
async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(err) => {
                tracing::warn!("cannot listen for SIGTERM: {err}");
                std::future::pending::<()>().await;
            }
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
    tracing::info!("shutdown signal received");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wildcard_bind_addresses_become_loopback() {
        let addr = |s: &str| s.parse::<SocketAddr>().unwrap();
        assert_eq!(local_addr(addr("0.0.0.0:8080")), addr("127.0.0.1:8080"));
        assert_eq!(local_addr(addr("[::]:8080")), addr("[::1]:8080"));
        assert_eq!(
            local_addr(addr("192.168.1.5:9000")),
            addr("192.168.1.5:9000")
        );
    }
}
