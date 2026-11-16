mod common;

use axum::{
    Router,
    http::{Method, StatusCode},
};
use serde_json::{Value, json};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{body_partial_json, method},
};

use common::{STOP_39_FIRST_DEPARTURE, at, get, send, synced_app_at};

/// The 67 to Moncalieri at stop 39: 17:24 in Rome on 2026-10-29, 7 seconds early.
const TRIP: &str = "gtt:28459721U";
/// The 33, the next departure at the stop.
const OTHER_TRIP: &str = "gtt:28436808U";

/// Half an hour before the 67 leaves.
fn before() -> chrono::DateTime<chrono::Utc> {
    at(STOP_39_FIRST_DEPARTURE - 1800)
}

async fn stop_39(server: &MockServer, body: Value) {
    Mock::given(method("POST"))
        .and(body_partial_json(
            json!({ "variables": { "id": "gtt:39" } }),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .with_priority(1)
        .mount(server)
        .await;
}

async fn app_at(now: chrono::DateTime<chrono::Utc>) -> (common::TestApp, MockServer) {
    let (app, server) = synced_app_at(now).await;
    stop_39(&server, common::fixture("stop_39.json")).await;
    (app, server)
}

async fn track(app: &Router, body: Value) -> (StatusCode, Value) {
    send(app, Method::POST, "/api/trackings", Some(body)).await
}

fn body() -> Value {
    json!({"stop": "39", "trip_id": TRIP})
}

#[tokio::test]
async fn tracks_a_departure_and_shows_it() {
    let (app, _server) = app_at(before()).await;

    let (status, detail) = track(&app.router, body()).await;

    assert_eq!(status, StatusCode::CREATED, "{detail}");
    assert_eq!(detail["id"], 1);
    assert_eq!(detail["recurring"], false);
    assert_eq!(detail["phase"], "tracking");
    assert_eq!(detail["line"], "67");
    assert_eq!(detail["scheduled"], "2026-10-29T17:24:00+01:00");
    assert_eq!(
        detail["stop"],
        json!({"id": "gtt:39", "code": "39", "name": "Porta Nuova"})
    );
    assert_eq!(detail["vehicle"], Value::Null);
    assert_eq!(detail["events"], json!([]));
    assert_eq!(detail["planned_reminders"], json!([10, 5]));
    assert_eq!(detail["delay_alert_minutes"], 3);

    let (status, read) = get(&app.router, "/api/trackings/1").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(read["id"], 1);
    assert_eq!(read["phase"], "tracking");
    assert_eq!(read["stop"], detail["stop"]);

    let (status, list) = get(&app.router, "/api/trackings").await;
    assert_eq!(status, StatusCode::OK);
    let rows = list["trackings"].as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["id"], 1);
    assert_eq!(rows[0]["line"], "67");
    assert_eq!(rows[0]["phase"], "tracking");
    assert_eq!(rows[0]["recurring"], false);
    assert_eq!(rows[0]["stop"], detail["stop"]);
    assert_eq!(rows[0]["scheduled"], "2026-10-29T17:24:00+01:00");
}

#[tokio::test]
async fn tracking_the_same_trip_again_replaces_its_settings() {
    let (app, _server) = app_at(before()).await;
    let (status, _) = track(&app.router, body()).await;
    assert_eq!(status, StatusCode::CREATED);

    let mut again = body();
    again["alert_before_minutes"] = json!([3, 15]);
    again["delay_alert_minutes"] = json!(5);
    let (status, detail) = track(&app.router, again).await;

    assert_eq!(status, StatusCode::OK, "{detail}");
    assert_eq!(detail["id"], 1);
    assert_eq!(detail["planned_reminders"], json!([15, 3]));
    assert_eq!(detail["delay_alert_minutes"], 5);
    let (_, list) = get(&app.router, "/api/trackings").await;
    assert_eq!(list["trackings"].as_array().unwrap().len(), 1);

    // Another trip is another tracking.
    let (status, other) = track(&app.router, json!({"stop": "39", "trip_id": OTHER_TRIP})).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(other["id"], 2);
}

#[tokio::test]
async fn list_is_soonest_first() {
    let (app, _server) = app_at(before()).await;
    // The 33 leaves after the 67.
    track(&app.router, json!({"stop": "39", "trip_id": OTHER_TRIP})).await;
    track(&app.router, body()).await;

    let (_, list) = get(&app.router, "/api/trackings").await;

    let lines: Vec<&Value> = list["trackings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| &row["line"])
        .collect();
    assert_eq!(lines, ["67", "33"]);
}

#[tokio::test]
async fn one_offs_are_hidden_from_the_rules() {
    let (app, _server) = app_at(before()).await;
    track(&app.router, body()).await;

    let (status, list) = get(&app.router, "/api/rules").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(list, json!({"rules": []}));

    for (verb, uri) in [
        (Method::GET, "/api/rules/1"),
        (Method::PATCH, "/api/rules/1"),
        (Method::DELETE, "/api/rules/1"),
        (Method::POST, "/api/rules/1/pause"),
        (Method::POST, "/api/rules/1/resume"),
        (Method::POST, "/api/rules/1/test"),
        (Method::GET, "/api/rules/1/runs"),
    ] {
        let body = (verb == Method::PATCH).then(|| json!({"enabled": false}));
        let (status, json) = send(&app.router, verb.clone(), uri, body).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{verb} {uri}: {json}");
        assert_eq!(json["error"]["code"], "rule_not_found");
    }
    // Still there.
    let (status, _) = get(&app.router, "/api/trackings/1").await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn rules_do_not_accept_a_trip() {
    let (app, _server) = app_at(before()).await;
    let (status, json) = send(
        &app.router,
        Method::POST,
        "/api/rules",
        Some(json!({
            "name": "x", "kind": "track", "stop": "39", "days": ["mon"],
            "config": {"line": "67", "scheduled_time": "08:05", "trip_id": TRIP}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{json}");
    assert_eq!(json["error"]["code"], "invalid_request");
}

#[tokio::test]
async fn deleting_a_one_off_removes_it() {
    let (app, _server) = app_at(before()).await;
    track(&app.router, body()).await;

    let (status, _) = send(&app.router, Method::DELETE, "/api/trackings/1", None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, json) = get(&app.router, "/api/trackings/1").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(json["error"]["code"], "tracking_not_found");
    let (_, list) = get(&app.router, "/api/trackings").await;
    assert_eq!(list, json!({"trackings": []}));
    let (status, _) = send(&app.router, Method::DELETE, "/api/trackings/1", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn deleting_a_recurring_tracking_stops_today_only() {
    // Thursday 5 November 2026, 07:50 in Rome: the 08:05 tracking is searching.
    let now = chrono::DateTime::parse_from_rfc3339("2026-11-05T06:50:00Z")
        .unwrap()
        .to_utc();
    let (app, _server) = app_at(now).await;
    let (status, rule) = send(
        &app.router,
        Method::POST,
        "/api/rules",
        Some(json!({
            "name": "Il 67 delle 8:05", "kind": "track", "stop": "39", "days": ["thu"],
            "config": {"line": "67", "scheduled_time": "08:05"}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{rule}");
    let (status, detail) = get(&app.router, "/api/trackings/1").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(detail["recurring"], true);
    assert_eq!(detail["phase"], "waiting");
    assert_eq!(detail["scheduled"], "2026-11-05T08:05:00+01:00");

    let (status, _) = send(&app.router, Method::DELETE, "/api/trackings/1", None).await;

    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, detail) = get(&app.router, "/api/trackings/1").await;
    assert_eq!(detail["phase"], "stopped");
    // The rule goes on.
    let (status, rule) = get(&app.router, "/api/rules/1").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(rule["enabled"], true);
    let (_, list) = get(&app.router, "/api/trackings").await;
    assert_eq!(list, json!({"trackings": []}));
}

#[tokio::test]
async fn refuses_what_cannot_be_tracked() {
    let (app, _server) = app_at(before()).await;

    let (status, json) = track(&app.router, json!({"stop": "39", "trip_id": "gtt:nope"})).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(json["error"]["code"], "trip_not_found");
    let (status, json) = track(&app.router, json!({"stop": "9999", "trip_id": TRIP})).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(json["error"]["code"], "stop_not_found");

    let with = |field: &str, value: Value| {
        let mut request = body();
        request[field] = value;
        request
    };
    for request in [
        with("alert_before_minutes", json!([0])),
        with("alert_before_minutes", json!([61])),
        with("alert_before_minutes", json!([1, 2, 3, 4, 5])),
        with("delay_alert_minutes", json!(0)),
        with("delay_alert_minutes", json!(31)),
        with("color", json!("red")),
        json!({"stop": "39"}),
    ] {
        let (status, json) = track(&app.router, request.clone()).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{request}: {json}");
    }
    // No reminders is allowed.
    let (status, _) = track(&app.router, with("alert_before_minutes", json!([]))).await;
    assert_eq!(status, StatusCode::CREATED);

    for (uri, expected) in [
        ("/api/trackings/9", StatusCode::NOT_FOUND),
        ("/api/trackings/x", StatusCode::BAD_REQUEST),
    ] {
        let (status, _) = get(&app.router, uri).await;
        assert_eq!(status, expected, "{uri}");
    }
    let (status, _) = send(&app.router, Method::DELETE, "/api/trackings/9", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn refuses_a_departed_run() {
    // A minute after the 67 left.
    let (app, _server) = app_at(at(STOP_39_FIRST_DEPARTURE + 60)).await;

    let (status, json) = track(&app.router, body()).await;

    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{json}");
    assert_eq!(json["error"]["code"], "trip_not_trackable");
}

#[tokio::test]
async fn refuses_a_cancelled_run() {
    let (app, server) = synced_app_at(before()).await;
    let mut fixture = common::fixture("stop_39.json");
    fixture["data"]["stop"]["stoptimesWithoutPatterns"][0]["realtimeState"] = json!("CANCELED");
    stop_39(&server, fixture).await;

    let (status, json) = track(&app.router, body()).await;

    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{json}");
    assert_eq!(json["error"]["code"], "trip_not_trackable");
    let (_, list) = get(&app.router, "/api/trackings").await;
    assert_eq!(list, json!({"trackings": []}));
}

#[tokio::test]
async fn a_recurring_run_still_going_after_midnight_is_shown_and_stopped() {
    // Friday 6 November 2026, 00:03 in Rome (Thursday 23:03 UTC): the Thursday 23:55 run is
    // still tracking.
    let now = chrono::DateTime::parse_from_rfc3339("2026-11-05T23:03:00Z")
        .unwrap()
        .to_utc();
    let (app, _server) = app_at(now).await;
    let (status, rule) = send(
        &app.router,
        Method::POST,
        "/api/rules",
        Some(json!({
            "name": "Il 67 delle 23:55", "kind": "track", "stop": "39",
            "days": ["mon", "tue", "wed", "thu", "fri", "sat", "sun"],
            "config": {"line": "67", "scheduled_time": "23:55"}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{rule}");
    app.scheduler
        .insert_track_run(
            1,
            chrono::NaiveDate::from_ymd_opt(2026, 11, 5).unwrap(),
            "tracking",
            chrono::DateTime::parse_from_rfc3339("2026-11-05T22:55:00Z")
                .unwrap()
                .to_utc(),
            &Default::default(),
            now,
        )
        .await
        .unwrap();

    let (_, detail) = get(&app.router, "/api/trackings/1").await;
    assert_eq!(detail["phase"], "tracking");
    let (status, _) = send(&app.router, Method::DELETE, "/api/trackings/1", None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let yesterday = app
        .scheduler
        .get_track_run(1, chrono::NaiveDate::from_ymd_opt(2026, 11, 5).unwrap())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(yesterday.status, "stopped");
    let today = app
        .scheduler
        .get_track_run(1, chrono::NaiveDate::from_ymd_opt(2026, 11, 6).unwrap())
        .await
        .unwrap();
    assert!(today.is_none());
}
