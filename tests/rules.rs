mod common;

use axum::{
    Router,
    http::{Method, StatusCode},
};
use chrono::{DateTime, Utc};
use serde_json::{Value, json};

use common::{get, offline_app, send, synced_app_at};

/// The fake clock, from an RFC 3339 time.
fn at(text: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(text).unwrap().to_utc()
}

/// Thursday 5 November 2026, 07:00 in Rome.
const THURSDAY_MORNING: &str = "2026-11-05T06:00:00Z";

fn summary() -> Value {
    json!({
        "name": " Casa → lavoro ",
        "kind": "summary",
        "stop": "39",
        "days": ["fri", "mon", "tue", "wed", "thu", "mon"],
        "config": {"time": "07:40", "lines": ["9b", "67"]}
    })
}

fn tracking() -> Value {
    json!({
        "name": "Il 67 delle 8:05",
        "kind": "track",
        "stop": "gtt:39",
        "days": ["sat"],
        "config": {"line": "67", "scheduled_time": "08:05", "alert_before_minutes": [5, 15]}
    })
}

async fn create(app: &Router, body: Value) -> (StatusCode, Value) {
    send(app, Method::POST, "/api/rules", Some(body)).await
}

async fn patch(app: &Router, id: i64, body: Value) -> (StatusCode, Value) {
    send(app, Method::PATCH, &format!("/api/rules/{id}"), Some(body)).await
}

async fn pause(app: &Router, id: i64, body: Option<Value>) -> (StatusCode, Value) {
    send(app, Method::POST, &format!("/api/rules/{id}/pause"), body).await
}

#[tokio::test]
async fn creates_a_summary_with_its_defaults_and_today() {
    let (app, _server) = synced_app_at(at(THURSDAY_MORNING)).await;

    let (status, rule) = create(&app.router, summary()).await;

    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(rule["id"], 1);
    assert_eq!(rule["name"], "Casa → lavoro");
    assert_eq!(rule["kind"], "summary");
    assert_eq!(rule["enabled"], true);
    assert_eq!(rule["stop_id"], "gtt:39");
    assert_eq!(rule["stop"]["name"], "PORTA NUOVA");
    assert_eq!(rule["missing"], false);
    assert_eq!(rule["days"], json!(["mon", "tue", "wed", "thu", "fri"]));
    assert_eq!(rule["date"], Value::Null);
    assert_eq!(rule["skip_holidays"], true);
    assert_eq!(rule["paused_until"], Value::Null);
    // Lines take the index's spelling; the missing numbers get their defaults, stored.
    assert_eq!(
        rule["config"],
        json!({"time": "07:40", "lines": ["9B", "67"], "window_minutes": 30, "max_departures": 6})
    );
    assert_eq!(
        rule["today"],
        json!({"status": "due", "at": "2026-11-05T07:40:00+01:00", "run": null})
    );
    assert_eq!(rule["next"], "2026-11-05T07:40:00+01:00");
    assert_eq!(rule["created_at"], "2026-11-05T06:00:00Z");
    assert_eq!(rule["updated_at"], "2026-11-05T06:00:00Z");

    let (status, read) = get(&app.router, "/api/rules/1").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(read, rule);
    let (status, list) = get(&app.router, "/api/rules").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(list, json!({"rules": [rule]}));
}

#[tokio::test]
async fn creates_a_tracking_rule() {
    let (app, _server) = synced_app_at(at(THURSDAY_MORNING)).await;

    let (status, rule) = create(&app.router, tracking()).await;

    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(rule["kind"], "track");
    assert_eq!(
        rule["config"],
        json!({
            "line": "67", "scheduled_time": "08:05", "tolerance_minutes": 2,
            "watch_from_minutes": 30, "alert_before_minutes": [15, 5], "delay_alert_minutes": 3
        })
    );
    assert_eq!(
        rule["today"],
        json!({"status": "not_scheduled", "run": null})
    );
    assert_eq!(rule["next"], "2026-11-07T08:05:00+01:00");
}

#[tokio::test]
async fn refuses_invalid_rules() {
    let (app, _server) = synced_app_at(at(THURSDAY_MORNING)).await;
    let with = |field: &str, value: Value| {
        let mut body = summary();
        body[field] = value;
        body
    };
    let invalid = [
        with("kind", json!("leave_now")),
        with("name", json!("  ")),
        with("days", json!([])),
        with("days", json!(["Mon"])),
        with("stop", json!("gtt:39:1")),
        // A one-off rule comes from /api/trackings (decision AJ).
        with("date", json!("2026-11-05")),
        with("config", json!({"time": "7:40"})),
        with("config", json!({"time": "07:40", "lines": ["4"]})),
        with("config", json!({"line": "67", "scheduled_time": "08:05"})),
        with("config", json!({"time": "07:40", "color": "red"})),
        json!({"name": "x", "kind": "summary", "stop": "39", "days": ["mon"]}),
    ];
    for body in invalid {
        let (status, json) = create(&app.router, body.clone()).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert_eq!(json["error"]["code"], "invalid_request", "{body}");
    }

    let (status, json) = create(&app.router, with("stop", json!("999999"))).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(json["error"]["code"], "stop_not_found");

    let (_, list) = get(&app.router, "/api/rules").await;
    assert_eq!(list, json!({"rules": []}));
}

#[tokio::test]
async fn needs_the_stop_index() {
    let app = offline_app().await;
    let (status, json) = create(&app, summary()).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(json["error"]["code"], "index_not_ready");
}

#[tokio::test]
async fn updates_a_rule() {
    let (app, _server) = synced_app_at(at(THURSDAY_MORNING)).await;
    create(&app.router, summary()).await;

    let (status, rule) = patch(
        &app.router,
        1,
        json!({"name": "Andata", "days": ["sat", "sun"], "skip_holidays": false}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(rule["name"], "Andata");
    assert_eq!(rule["days"], json!(["sat", "sun"]));
    assert_eq!(rule["skip_holidays"], false);
    assert_eq!(rule["config"]["lines"], json!(["9B", "67"]));
    assert_eq!(
        rule["today"],
        json!({"status": "not_scheduled", "run": null})
    );

    // `config` is replaced whole, with defaults again for what is missing.
    let (status, rule) = patch(
        &app.router,
        1,
        json!({"kind": "summary", "config": {"time": "18:10", "max_departures": 3}}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        rule["config"],
        json!({"time": "18:10", "lines": [], "window_minutes": 30, "max_departures": 3})
    );

    // Another stop: the lines must serve it.
    let (status, _) = patch(
        &app.router,
        1,
        json!({"stop": "8224", "config": {"time": "18:10", "lines": ["9"]}}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, rule) = patch(
        &app.router,
        1,
        json!({"stop": "8224", "config": {"time": "18:10", "lines": ["m1"]}}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(rule["stop_id"], "gtt:8224");
    assert_eq!(rule["config"]["lines"], json!(["M1"]));

    let (status, json) = patch(&app.router, 1, json!({"kind": "track"})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(json["error"]["code"], "invalid_request");
    let (status, _) = patch(&app.router, 1, json!({"date": "2026-11-05"})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let (status, json) = patch(&app.router, 2, json!({"name": "x"})).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(json["error"]["code"], "rule_not_found");
}

#[tokio::test]
async fn pauses_skips_today_and_resumes() {
    let (app, _server) = synced_app_at(at(THURSDAY_MORNING)).await;
    create(&app.router, summary()).await;

    // "Salta oggi": paused until today, included.
    let (status, rule) = pause(&app.router, 1, Some(json!({"until": "2026-11-05"}))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(rule["paused_until"], "2026-11-05");
    assert_eq!(
        rule["today"],
        json!({"status": "paused", "until": "2026-11-05", "run": null})
    );
    assert_eq!(rule["next"], "2026-11-06T07:40:00+01:00");

    // "In pausa fino al 16 novembre": back on Tuesday 17.
    let (_, rule) = pause(&app.router, 1, Some(json!({"until": "2026-11-16"}))).await;
    assert_eq!(rule["next"], "2026-11-17T07:40:00+01:00");

    // Without a date: turned off.
    let (status, rule) = pause(&app.router, 1, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(rule["enabled"], false);
    assert_eq!(rule["paused_until"], Value::Null);
    assert_eq!(rule["today"], json!({"status": "disabled", "run": null}));
    assert_eq!(rule["next"], Value::Null);

    // A pause with a date turns it back on.
    let (_, rule) = pause(&app.router, 1, Some(json!({"until": "2026-11-05"}))).await;
    assert_eq!(rule["enabled"], true);

    let (status, rule) = send(&app.router, Method::POST, "/api/rules/1/resume", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(rule["enabled"], true);
    assert_eq!(rule["paused_until"], Value::Null);
    assert_eq!(
        rule["today"],
        json!({"status": "due", "at": "2026-11-05T07:40:00+01:00", "run": null})
    );

    for until in ["2026-11-04", "2027-11-06", "16/11/2026"] {
        let (status, _) = pause(&app.router, 1, Some(json!({"until": until}))).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{until}");
    }
    let (status, _) = pause(&app.router, 1, Some(json!({"from": "2026-11-05"}))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, json) = pause(&app.router, 9, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(json["error"]["code"], "rule_not_found");
}

#[tokio::test]
async fn skips_holidays() {
    // Tuesday 8 December 2026, Immacolata, 07:00 in Rome.
    let (app, _server) = synced_app_at(at("2026-12-08T06:00:00Z")).await;
    let (_, rule) = create(&app.router, summary()).await;
    assert_eq!(
        rule["today"],
        json!({"status": "holiday", "holiday": "Immacolata", "run": null})
    );
    assert_eq!(rule["next"], "2026-12-09T07:40:00+01:00");

    let (_, rule) = patch(&app.router, 1, json!({"skip_holidays": false})).await;
    assert_eq!(
        rule["today"],
        json!({"status": "due", "at": "2026-12-08T07:40:00+01:00", "run": null})
    );
}

#[tokio::test]
async fn today_is_the_day_in_rome() {
    // 23:30 UTC on Thursday is 00:30 on Friday in Rome.
    let (app, _server) = synced_app_at(at("2026-11-05T23:30:00Z")).await;
    let mut body = summary();
    body["days"] = json!(["fri"]);
    let (_, rule) = create(&app.router, body).await;
    assert_eq!(
        rule["today"],
        json!({"status": "due", "at": "2026-11-06T07:40:00+01:00", "run": null})
    );
}

#[tokio::test]
async fn deletes_a_rule() {
    let (app, _server) = synced_app_at(at(THURSDAY_MORNING)).await;
    create(&app.router, summary()).await;

    let (status, _) = send(&app.router, Method::DELETE, "/api/rules/1", None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, json) = get(&app.router, "/api/rules/1").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(json["error"]["code"], "rule_not_found");
    let (status, _) = send(&app.router, Method::DELETE, "/api/rules/1", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = get(&app.router, "/api/rules/abc").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

/// Makes the fake upstream answer departure queries for stop 39, ahead of the catch-all
/// answer of `synced_app`.
async fn upstream_with_stop_39(
    server: &wiremock::MockServer,
    response: wiremock::ResponseTemplate,
) {
    use wiremock::{
        Mock,
        matchers::{body_partial_json, method},
    };
    Mock::given(method("POST"))
        .and(body_partial_json(
            json!({ "variables": { "id": "gtt:39" } }),
        ))
        .respond_with(response)
        .with_priority(1)
        .mount(server)
        .await;
}

#[tokio::test]
async fn tests_a_summary_now_and_logs_it_without_a_run() {
    use wiremock::ResponseTemplate;
    let (app, server) = synced_app_at(at(THURSDAY_MORNING)).await;
    let stop_39 = ResponseTemplate::new(200).set_body_json(common::fixture("stop_39.json"));
    upstream_with_stop_39(&server, stop_39).await;
    create(&app.router, summary()).await;
    // Turned off: the test ignores it.
    pause(&app.router, 1, None).await;

    let (status, sent) = send(&app.router, Method::POST, "/api/rules/1/test", None).await;

    assert_eq!(status, StatusCode::OK, "{sent}");
    assert_eq!(sent["channel"], "log");
    assert_eq!(sent["sent"], 1);
    assert_eq!(sent["removed"], 0);
    assert_eq!(sent["failed"], 0);
    assert!(!sent["title"].as_str().unwrap().is_empty());
    assert!(sent["body"].is_string());

    let (_, runs) = get(&app.router, "/api/rules/1/runs").await;
    assert_eq!(runs, json!({"runs": []}));
    let (_, rule) = get(&app.router, "/api/rules/1").await;
    assert_eq!(rule["today"]["run"], Value::Null);
    let (status, log) = get(&app.router, "/api/notifications").await;
    assert_eq!(status, StatusCode::OK);
    let log = log["notifications"].as_array().unwrap();
    assert_eq!(log.len(), 1);
    assert_eq!(log[0]["kind"], "summary");
    assert_eq!(log[0]["rule_id"], 1);
    assert_eq!(log[0]["rule_name"], "Casa → lavoro");
    assert_eq!(log[0]["title"], sent["title"]);
    assert_eq!(log[0]["body"], sent["body"]);
    assert_eq!(log[0]["channel"], "log");
    assert_eq!(log[0]["sent_at"], "2026-11-05T06:00:00Z");
}

#[tokio::test]
async fn test_refuses_tracking_rules_unknown_rules_and_upstream_failures() {
    use wiremock::ResponseTemplate;
    let (app, server) = synced_app_at(at(THURSDAY_MORNING)).await;
    create(&app.router, tracking()).await;
    create(&app.router, summary()).await;

    let (status, body) = send(&app.router, Method::POST, "/api/rules/1/test", None).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["error"]["code"], "not_a_summary");
    let (status, body) = send(&app.router, Method::POST, "/api/rules/9/test", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"]["code"], "rule_not_found");
    let (status, _) = send(&app.router, Method::POST, "/api/rules/abc/test", None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    upstream_with_stop_39(&server, ResponseTemplate::new(500)).await;
    let (status, body) = send(&app.router, Method::POST, "/api/rules/2/test", None).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY, "{body}");
    assert_eq!(body["error"]["code"], "upstream_error");
    let (_, log) = get(&app.router, "/api/notifications").await;
    assert_eq!(log, json!({"notifications": []}));
}

#[tokio::test]
async fn lists_runs_and_shows_today_run() {
    let (app, _server) = synced_app_at(at(THURSDAY_MORNING)).await;
    create(&app.router, summary()).await;
    let now = at(THURSDAY_MORNING);
    let due = at("2026-11-05T06:40:00Z");
    // 35 days: only the last 30 come back, newest first.
    for day in 0..35 {
        let date =
            chrono::NaiveDate::from_ymd_opt(2026, 11, 5).unwrap() - chrono::TimeDelta::days(day);
        app.scheduler
            .insert_run(1, date, "pending", due, now)
            .await
            .unwrap();
    }
    app.scheduler
        .set_run_status(
            1,
            chrono::NaiveDate::from_ymd_opt(2026, 11, 4).unwrap(),
            "sent",
            now,
        )
        .await
        .unwrap();

    let (status, runs) = get(&app.router, "/api/rules/1/runs").await;
    assert_eq!(status, StatusCode::OK);
    let runs = runs["runs"].as_array().unwrap();
    assert_eq!(runs.len(), 30);
    assert_eq!(runs[0]["date"], "2026-11-05");
    assert_eq!(runs[0]["status"], "pending");
    assert_eq!(runs[1]["status"], "sent");
    assert_eq!(runs[0]["rule_id"], 1);
    assert!(runs[0].get("due_at").is_some() && runs[0].get("updated_at").is_some());

    // `pending` is passed through as is.
    let (_, rule) = get(&app.router, "/api/rules/1").await;
    assert_eq!(rule["today"]["status"], "due");
    assert_eq!(
        rule["today"]["run"],
        json!({"status": "pending", "updated_at": "2026-11-05T06:00:00Z"})
    );

    let (status, body) = get(&app.router, "/api/rules/9/runs").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"]["code"], "rule_not_found");
}

#[tokio::test]
async fn notifications_are_limited_and_newest_first() {
    let (app, _server) = synced_app_at(at(THURSDAY_MORNING)).await;
    for (minute, title) in [(0, "a"), (2, "c"), (1, "b")] {
        let record = intime::scheduler::store::NewNotification {
            rule_id: None,
            kind: "alert".to_owned(),
            title: title.to_owned(),
            body: String::new(),
            url: None,
            channel: "log".to_owned(),
            report: Default::default(),
            sent_at: at(THURSDAY_MORNING) + chrono::TimeDelta::minutes(minute),
        };
        app.scheduler.log_notification(&record).await.unwrap();
    }

    let (_, log) = get(&app.router, "/api/notifications?limit=2").await;
    let titles: Vec<_> = log["notifications"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["title"].as_str().unwrap())
        .collect();
    assert_eq!(titles, ["c", "b"]);
    assert_eq!(log["notifications"][0]["rule_name"], Value::Null);
    for bad in ["limit=0", "limit=201", "limit=x", "other=1"] {
        let (status, _) = get(&app.router, &format!("/api/notifications?{bad}")).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}");
    }
    let (status, _) = get(&app.router, "/api/notifications?limit=200").await;
    assert_eq!(status, StatusCode::OK);
}
