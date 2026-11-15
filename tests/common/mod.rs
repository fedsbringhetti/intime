// Each test binary uses a different subset of these helpers.
#![allow(dead_code)]

use std::{sync::Arc, time::Duration};

use axum::{
    Router,
    body::Body,
    http::{HeaderMap, Method, Request, StatusCode, header},
};
use chrono::{DateTime, Utc};
use http_body_util::BodyExt;
use intime::{
    api::{self, AppState},
    db,
    departures::DeparturesService,
    favorites::FavoriteStore,
    gtt::{GttRealtime, alerts::AlertsService, static_data::GttStatic},
    notify::{Notifier, log::LogNotifier, vapid::VapidKey, webpush::WebPushNotifier},
    push::{EndpointPolicy, SubscriptionStore},
    rules::RuleStore,
    scheduler::store::SchedulerStore,
    stops::{Feeds, StopIndex, sync::sync_once},
    time::{Clock, FixedClock},
    upstream::UpstreamClient,
    web::WebAssets,
};
use rust_embed::RustEmbed;
use tower::ServiceExt;
use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};

/// First expected departure in `fixtures/stop_39.json` (17:23:53 on 2026-10-29).
pub const STOP_39_FIRST_DEPARTURE: i64 = 1_793_291_033;

/// Fixed "now" for tests that do not depend on departure times.
pub const NOW: i64 = STOP_39_FIRST_DEPARTURE;

pub struct TestApp {
    pub router: Router,
    pub stops: StopIndex,
    pub upstream: UpstreamClient,
    /// Set when `Options::gtt_url` is: tests read the alerts with `refresh`.
    pub alerts: Option<Arc<AlertsService>>,
    /// Set when `Options::gtt_url` and `Options::gtt_gtfs_url` are.
    pub gtt_static: Option<GttStatic>,
    pub subscriptions: SubscriptionStore,
    pub scheduler: SchedulerStore,
    /// Set when `Options::vapid_key` is.
    pub vapid_public_key: Option<String>,
}

/// A tiny stand-in for the built web app: `index.html` and one hashed script.
#[derive(RustEmbed)]
#[folder = "tests/fixtures/web"]
pub struct FixtureWeb;

/// Settings that some tests change.
#[derive(Debug, Clone)]
pub struct Options {
    pub cache_ttl: Duration,
    pub api_token: Option<&'static str>,
    pub web: WebAssets,
    /// Folder of GTT's feeds; `None` disables GTT, as in most tests.
    pub gtt_url: Option<String>,
    /// GTT's GTFS static archive; used only with `gtt_url`.
    pub gtt_gtfs_url: Option<String>,
    /// Enables Web Push, with the subject `mailto:test@example.com`.
    pub vapid_key: Option<VapidKey>,
    /// Where subscriptions may point; `None` is the production list of push services.
    pub push_endpoints: Option<EndpointPolicy>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            cache_ttl: Duration::from_secs(20),
            api_token: None,
            web: WebAssets::from_embed::<FixtureWeb>(),
            gtt_url: None,
            gtt_gtfs_url: None,
            vapid_key: None,
            push_endpoints: None,
        }
    }
}

/// The app with an in-memory database, a fixed clock and default feeds `gtt`.
pub async fn test_app(
    upstream_url: &str,
    now: DateTime<Utc>,
    upstream_timeout: Duration,
) -> TestApp {
    test_app_with(upstream_url, now, upstream_timeout, Options::default()).await
}

pub async fn test_app_with(
    upstream_url: &str,
    now: DateTime<Utc>,
    upstream_timeout: Duration,
    options: Options,
) -> TestApp {
    let upstream = UpstreamClient::new(upstream_url.parse().unwrap(), upstream_timeout).unwrap();
    let pool = db::connect_in_memory().await.unwrap();
    let stops = StopIndex::new(pool.clone());
    let clock: Arc<dyn Clock> = Arc::new(FixedClock(now));
    let gtt = options.gtt_url.as_ref().map(|url| {
        Arc::new(GttRealtime::new(&url.parse().unwrap(), Duration::from_secs(20)).unwrap())
    });
    let gtt_static = match (&gtt, &options.gtt_gtfs_url) {
        (Some(_), Some(url)) => Some(GttStatic::new(pool.clone(), url.parse().unwrap()).unwrap()),
        _ => None,
    };
    let alerts = gtt.as_ref().map(|gtt| {
        Arc::new(AlertsService::new(
            gtt.clone(),
            stops.clone(),
            gtt_static.clone(),
        ))
    });
    let subscriptions = SubscriptionStore::new(pool.clone());
    let notifier: Arc<dyn Notifier> = match &options.vapid_key {
        Some(key) => Arc::new(
            WebPushNotifier::new(
                subscriptions.clone(),
                key.clone(),
                "mailto:test@example.com".to_owned(),
                clock.clone(),
            )
            .unwrap(),
        ),
        None => Arc::new(LogNotifier),
    };
    let vapid_public_key = options.vapid_key.as_ref().map(VapidKey::public_key_base64);
    let scheduler = SchedulerStore::new(pool.clone());
    let router = api::router(AppState {
        departures: Arc::new(DeparturesService::new(
            upstream.clone(),
            gtt.clone(),
            options.cache_ttl,
            clock.clone(),
        )),
        stops: stops.clone(),
        favorites: FavoriteStore::new(pool.clone()),
        rules: RuleStore::new(pool.clone()),
        scheduler: scheduler.clone(),
        subscriptions: subscriptions.clone(),
        notifier,
        vapid_public_key: vapid_public_key.as_deref().map(Arc::from),
        push_endpoints: options
            .push_endpoints
            .unwrap_or_else(EndpointPolicy::push_services),
        default_feeds: Feeds::Only(vec!["gtt".to_owned()]),
        upstream_health: upstream.health(),
        gtt,
        alerts: alerts.clone(),
        gtt_static: gtt_static.clone(),
        api_token: options.api_token.map(|token| token.parse().unwrap()),
        clock,
        web: options.web,
    });
    TestApp {
        router,
        stops,
        upstream,
        alerts,
        gtt_static,
        subscriptions,
        scheduler,
        vapid_public_key,
    }
}

pub async fn app(upstream_url: &str, now: DateTime<Utc>, upstream_timeout: Duration) -> Router {
    test_app(upstream_url, now, upstream_timeout).await.router
}

/// An app whose upstream is never called.
pub async fn offline_app() -> Router {
    app("http://127.0.0.1:9/", Utc::now(), Duration::from_secs(1)).await
}

/// App whose upstream serves `all_stops.json` (44 real stops around Porta Nuova and a few
/// others), with the index already synced. Keep the server alive for the whole test.
pub async fn synced_app() -> (TestApp, MockServer) {
    synced_app_at(at(NOW)).await
}

/// `synced_app` with the clock at `now`.
pub async fn synced_app_at(now: DateTime<Utc>) -> (TestApp, MockServer) {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("all_stops.json")))
        .mount(&server)
        .await;
    let app = test_app(&server.uri(), now, Duration::from_secs(2)).await;
    let count = sync_once(&app.stops, &app.upstream, now).await.unwrap();
    assert_eq!(count, 44);
    (app, server)
}

pub fn fixture(name: &str) -> serde_json::Value {
    let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

pub fn at(timestamp: i64) -> DateTime<Utc> {
    DateTime::from_timestamp(timestamp, 0).unwrap()
}

pub async fn get(app: &Router, uri: &str) -> (StatusCode, serde_json::Value) {
    send(app, Method::GET, uri, None).await
}

/// Sends a prepared request; returns status, headers and the body parsed as JSON.
pub async fn request(
    app: &Router,
    request: Request<Body>,
) -> (StatusCode, HeaderMap, serde_json::Value) {
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json = serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null);
    (status, headers, json)
}

/// Sends a GET and returns status, headers and the body as text, for non-JSON responses.
pub async fn get_text(app: &Router, uri: &str) -> (StatusCode, HeaderMap, String) {
    let request = Request::builder().uri(uri).body(Body::empty()).unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    (status, headers, String::from_utf8(body.to_vec()).unwrap())
}

/// Sends a request with an optional JSON body; the response body is parsed as JSON
/// (`Null` when empty or not JSON).
pub async fn send(
    app: &Router,
    method: Method,
    uri: &str,
    body: Option<serde_json::Value>,
) -> (StatusCode, serde_json::Value) {
    let request = Request::builder().method(method).uri(uri);
    let request = match body {
        Some(body) => request
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string())),
        None => request.body(Body::empty()),
    };
    let response = app.clone().oneshot(request.unwrap()).await.unwrap();
    let status = response.status();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json = serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null);
    (status, json)
}
