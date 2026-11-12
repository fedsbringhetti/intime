mod common;

use std::time::Duration;

use axum::{
    Router,
    body::Body,
    http::{Method, Request, StatusCode, header},
};
use base64ct::{Base64UrlUnpadded, Encoding};
use chrono::Utc;
use intime::{
    notify::vapid::VapidKey,
    push::{EndpointPolicy, NewSubscription},
};
use p256::{SecretKey, elliptic_curve::rand_core::OsRng};
use serde_json::{Value, json};
use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};

use common::{Options, TestApp, request, send, test_app_with};

/// A browser's keys: the secret half stays with the test, to decrypt what the server sends.
struct Device {
    secret: SecretKey,
    auth: [u8; 16],
}

impl Device {
    fn new() -> Self {
        Self {
            secret: SecretKey::random(&mut OsRng),
            auth: [7; 16],
        }
    }

    fn subscription(&self, endpoint: &str) -> Value {
        let point = self.secret.public_key().to_sec1_bytes();
        json!({
            "endpoint": endpoint,
            "expirationTime": null,
            "keys": {
                "p256dh": Base64UrlUnpadded::encode_string(&point),
                "auth": Base64UrlUnpadded::encode_string(&self.auth),
            }
        })
    }
}

/// An app with Web Push on and a fake push service that accepts `127.0.0.1`.
async fn push_app() -> (TestApp, MockServer, VapidKey) {
    let key = VapidKey::generate();
    let server = MockServer::start().await;
    let app = test_app_with(
        "http://127.0.0.1:9/",
        Utc::now(),
        Duration::from_secs(1),
        Options {
            vapid_key: Some(key.clone()),
            push_endpoints: Some(EndpointPolicy::new(vec!["127.0.0.1".into()], false)),
            ..Options::default()
        },
    )
    .await;
    (app, server, key)
}

async fn subscribe(app: &Router, body: Value) -> (StatusCode, Value) {
    send(app, Method::POST, "/api/push/subscriptions", Some(body)).await
}

async fn post_with_agent(app: &Router, user_agent: &str, body: Value) -> (StatusCode, Value) {
    let req = Request::builder()
        .method(Method::POST)
        .uri("/api/push/subscriptions")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::USER_AGENT, user_agent)
        .body(Body::from(body.to_string()))
        .unwrap();
    let (status, _, json) = request(app, req).await;
    (status, json)
}

async fn send_test(app: &Router) -> (StatusCode, Value) {
    send(app, Method::POST, "/api/notifications/test", None).await
}

#[tokio::test]
async fn key_is_served_with_a_vapid_key() {
    let (app, _server, key) = push_app().await;

    let (status, body) = send(&app.router, Method::GET, "/api/push/key", None).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({"public_key": key.public_key_base64()}));
}

#[tokio::test]
async fn key_and_subscribe_are_disabled_without_a_vapid_key() {
    let app = test_app_with(
        "http://127.0.0.1:9/",
        Utc::now(),
        Duration::from_secs(1),
        Options::default(),
    )
    .await;

    let (status, body) = send(&app.router, Method::GET, "/api/push/key", None).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["error"]["code"], "push_disabled");

    let device = Device::new();
    let (status, body) = subscribe(
        &app.router,
        device.subscription("https://fcm.googleapis.com/fcm/send/abc"),
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["error"]["code"], "push_disabled");
}

#[tokio::test]
async fn subscribing_twice_updates_the_same_subscription() {
    let (app, server, _) = push_app().await;
    let device = Device::new();
    let endpoint = format!("{}/push/a", server.uri());

    let (status, first) = post_with_agent(
        &app.router,
        "Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X)",
        device.subscription(&endpoint),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(first["label"], "iPhone");
    assert!(first["created_at"].is_string());
    assert!(first.get("endpoint").is_none() && first.get("auth").is_none());

    let (status, second) = post_with_agent(
        &app.router,
        "Mozilla/5.0 (Linux; Android 14)",
        device.subscription(&endpoint),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(second["id"], first["id"]);
    assert_eq!(second["label"], "Android");
    assert_eq!(second["created_at"], first["created_at"]);
    assert_eq!(app.subscriptions.list().await.unwrap().len(), 1);
}

#[tokio::test]
async fn unknown_user_agent_gives_no_label() {
    let (app, server, _) = push_app().await;
    let device = Device::new();

    // `send` sets no User-Agent at all.
    let (status, body) = subscribe(
        &app.router,
        device.subscription(&format!("{}/push/a", server.uri())),
    )
    .await;

    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(body["label"], Value::Null);
}

#[tokio::test]
async fn refuses_unknown_fields_but_not_expiration_time() {
    let (app, server, _) = push_app().await;
    let device = Device::new();
    let endpoint = format!("{}/push/a", server.uri());

    let mut body = device.subscription(&endpoint);
    body["expirationTime"] = json!(1_900_000_000_000_i64);
    let (status, _) = subscribe(&app.router, body).await;
    assert_eq!(status, StatusCode::CREATED);

    let mut body = device.subscription(&endpoint);
    body["extra"] = json!(1);
    let (status, body) = subscribe(&app.router, body).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["code"], "invalid_request");

    let mut body = device.subscription(&endpoint);
    body["keys"]["extra"] = json!(1);
    let (status, _) = subscribe(&app.router, body).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn refuses_endpoints_that_are_not_push_services() {
    let (app, _server, _) = push_app().await;
    let device = Device::new();
    for endpoint in [
        "https://example.com/push",
        "https://fcm.googleapis.com.evil.com/x",
        "not a url",
    ] {
        let (status, body) = subscribe(&app.router, device.subscription(endpoint)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{endpoint}");
        assert_eq!(body["error"]["code"], "invalid_request");
    }

    // The production policy: https only, on known hosts.
    let production = test_app_with(
        "http://127.0.0.1:9/",
        Utc::now(),
        Duration::from_secs(1),
        Options {
            vapid_key: Some(VapidKey::generate()),
            ..Options::default()
        },
    )
    .await;
    let (status, _) = subscribe(
        &production.router,
        device.subscription("http://fcm.googleapis.com/fcm/send/abc"),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = subscribe(
        &production.router,
        device.subscription("https://fcm.googleapis.com/fcm/send/abc"),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert!(app.subscriptions.list().await.unwrap().is_empty());
}

#[tokio::test]
async fn refuses_bad_keys_without_panicking() {
    let (app, server, _) = push_app().await;
    let device = Device::new();
    let endpoint = format!("{}/push/a", server.uri());
    let good = device.subscription(&endpoint);

    let cases = [
        ("p256dh", "AAAA"),
        ("p256dh", "not base64 !!"),
        ("auth", &Base64UrlUnpadded::encode_string(&[1u8; 15])),
        ("auth", &Base64UrlUnpadded::encode_string(&[1u8; 17])),
        ("auth", ""),
    ];
    for (field, value) in cases {
        let mut body = good.clone();
        body["keys"][field] = json!(value);
        let (status, body) = subscribe(&app.router, body).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{field}={value}");
        assert_eq!(body["error"]["code"], "invalid_request");
    }
    assert!(app.subscriptions.list().await.unwrap().is_empty());
}

#[tokio::test]
async fn unsubscribing_removes_the_subscription_and_is_idempotent() {
    let (app, server, _) = push_app().await;
    let device = Device::new();
    let endpoint = format!("{}/push/a", server.uri());
    subscribe(&app.router, device.subscription(&endpoint)).await;
    assert_eq!(app.subscriptions.list().await.unwrap().len(), 1);

    let unsubscribe = |endpoint: String| {
        let router = app.router.clone();
        async move {
            send(
                &router,
                Method::DELETE,
                "/api/push/subscriptions",
                Some(json!({"endpoint": endpoint})),
            )
            .await
            .0
        }
    };
    assert_eq!(unsubscribe(endpoint.clone()).await, StatusCode::NO_CONTENT);
    assert!(app.subscriptions.list().await.unwrap().is_empty());
    // Already gone, or never known.
    assert_eq!(unsubscribe(endpoint).await, StatusCode::NO_CONTENT);
    assert_eq!(
        unsubscribe("https://nowhere.example/x".into()).await,
        StatusCode::NO_CONTENT
    );
}

#[tokio::test]
async fn test_notification_reaches_every_device_encrypted_and_signed() {
    let (app, server, key) = push_app().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(201))
        .mount(&server)
        .await;
    let devices = [Device::new(), Device::new()];
    for (index, device) in devices.iter().enumerate() {
        let endpoint = format!("{}/push/{index}", server.uri());
        let (status, _) = subscribe(&app.router, device.subscription(&endpoint)).await;
        assert_eq!(status, StatusCode::CREATED);
    }

    let (status, report) = send_test(&app.router).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        report,
        json!({"channel": "webpush", "sent": 2, "removed": 0, "failed": 0})
    );
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 2);
    for request in requests {
        let index: usize = request
            .url
            .path()
            .trim_start_matches("/push/")
            .parse()
            .unwrap();
        let header = |name: &str| {
            request
                .headers
                .get(name)
                .unwrap()
                .to_str()
                .unwrap()
                .to_owned()
        };
        assert_eq!(header("ttl"), "900");
        assert_eq!(header("urgency"), "high");
        assert_eq!(header("content-encoding"), "aes128gcm");
        let authorization = header("authorization");
        assert!(authorization.starts_with("vapid t="), "{authorization}");
        assert!(
            authorization.ends_with(&format!(", k={}", key.public_key_base64())),
            "{authorization}"
        );

        let device = &devices[index];
        let plain = web_push_native::decrypt(
            request.body.clone(),
            &device.secret,
            &web_push_native::Auth::from(device.auth),
        )
        .unwrap();
        let payload: Value = serde_json::from_slice(&plain).unwrap();
        assert_eq!(
            payload,
            json!({
                "title": "🔔 Notifica di prova",
                "body": "Le notifiche di InTime arrivano su questo dispositivo.",
                "url": "/notifiche",
                "tag": "test"
            })
        );
    }
}

#[tokio::test]
async fn an_expired_subscription_is_removed_and_the_others_still_get_the_message() {
    let (app, server, _) = push_app().await;
    for (path, status) in [("/push/gone", 410), ("/push/ok", 201)] {
        Mock::given(method("POST"))
            .and(wiremock::matchers::path(path))
            .respond_with(ResponseTemplate::new(status))
            .mount(&server)
            .await;
    }
    for path in ["gone", "ok"] {
        let endpoint = format!("{}/push/{path}", server.uri());
        subscribe(&app.router, Device::new().subscription(&endpoint)).await;
    }

    let (status, report) = send_test(&app.router).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        report,
        json!({"channel": "webpush", "sent": 1, "removed": 1, "failed": 0})
    );
    let left = app.subscriptions.list().await.unwrap();
    assert_eq!(left.len(), 1);
    assert!(left[0].endpoint.ends_with("/push/ok"));
}

#[tokio::test]
async fn a_server_error_counts_as_failed_and_keeps_the_subscription() {
    let (app, server, _) = push_app().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(500).set_body_string("boom"))
        .mount(&server)
        .await;
    let endpoint = format!("{}/push/a", server.uri());
    subscribe(&app.router, Device::new().subscription(&endpoint)).await;

    let (status, report) = send_test(&app.router).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        report,
        json!({"channel": "webpush", "sent": 0, "removed": 0, "failed": 1})
    );
    assert_eq!(app.subscriptions.list().await.unwrap().len(), 1);
}

#[tokio::test]
async fn without_vapid_the_test_notification_goes_to_the_log() {
    let app = test_app_with(
        "http://127.0.0.1:9/",
        Utc::now(),
        Duration::from_secs(1),
        Options::default(),
    )
    .await;

    let (status, report) = send_test(&app.router).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(report["channel"], "log");
    assert_eq!(report["sent"], 1);
}

fn report(sent: u32, removed: u32, failed: u32) -> Value {
    json!({"channel": "webpush", "sent": sent, "removed": removed, "failed": failed})
}

#[tokio::test]
async fn a_404_removes_the_subscription_like_a_410() {
    let (app, server, _) = push_app().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    let endpoint = format!("{}/push/a", server.uri());
    subscribe(&app.router, Device::new().subscription(&endpoint)).await;

    let (status, body) = send_test(&app.router).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, report(0, 1, 0));
    assert!(app.subscriptions.list().await.unwrap().is_empty());
}

#[tokio::test]
async fn a_network_error_counts_as_failed_and_keeps_the_subscription() {
    let (app, _server, _) = push_app().await;
    // A port nobody listens on: bind, note the port, close.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let endpoint = format!("http://127.0.0.1:{port}/push/a");
    let (status, _) = subscribe(&app.router, Device::new().subscription(&endpoint)).await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, body) = send_test(&app.router).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, report(0, 0, 1));
    assert_eq!(app.subscriptions.list().await.unwrap().len(), 1);
}

#[tokio::test]
async fn a_redirect_is_not_followed_and_counts_as_failed() {
    let (app, server, _) = push_app().await;
    let elsewhere = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(201))
        .mount(&elsewhere)
        .await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(307)
                .insert_header("location", format!("{}/stolen", elsewhere.uri()).as_str()),
        )
        .mount(&server)
        .await;
    let endpoint = format!("{}/push/a", server.uri());
    subscribe(&app.router, Device::new().subscription(&endpoint)).await;

    let (_, body) = send_test(&app.router).await;

    assert_eq!(body, report(0, 0, 1));
    assert!(elsewhere.received_requests().await.unwrap().is_empty());
    assert_eq!(app.subscriptions.list().await.unwrap().len(), 1);
}

#[tokio::test]
async fn stored_keys_that_do_not_decode_count_as_failed_without_panicking() {
    let (app, server, _) = push_app().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(201))
        .mount(&server)
        .await;
    // Inserted straight through the store, skipping the API's validation.
    app.subscriptions
        .upsert(
            NewSubscription {
                endpoint: format!("{}/push/broken", server.uri()),
                p256dh: "AAAA".to_owned(),
                auth: "x".to_owned(),
                label: None,
            },
            Utc::now(),
        )
        .await
        .unwrap();
    let endpoint = format!("{}/push/ok", server.uri());
    subscribe(&app.router, Device::new().subscription(&endpoint)).await;

    let (status, body) = send_test(&app.router).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, report(1, 0, 1));
    assert_eq!(app.subscriptions.list().await.unwrap().len(), 2);
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn the_vapid_jwt_names_the_push_service_origin_and_the_subject() {
    let (app, server, _) = push_app().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(201))
        .mount(&server)
        .await;
    let endpoint = format!("{}/push/a", server.uri());
    subscribe(&app.router, Device::new().subscription(&endpoint)).await;

    send_test(&app.router).await;

    let requests = server.received_requests().await.unwrap();
    let authorization = requests[0]
        .headers
        .get("authorization")
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();
    let jwt = authorization
        .strip_prefix("vapid t=")
        .unwrap()
        .split_once(", k=")
        .unwrap()
        .0;
    let payload = jwt.split('.').nth(1).unwrap();
    let claims: Value =
        serde_json::from_slice(&Base64UrlUnpadded::decode_vec(payload).unwrap()).unwrap();
    // `server.uri()` is `http://127.0.0.1:<port>`: no path, which is the origin.
    assert_eq!(claims["aud"], server.uri());
    assert_eq!(claims["sub"], "mailto:test@example.com");
}
