//! GTT's alerts: reading, linking to stops, in the departures, `/api/alerts`, `/api/status`.

mod common;

use std::{
    io::{Cursor, Write},
    time::Duration,
};

use axum::http::StatusCode;
use common::{Options, TestApp, at, fixture, get};
use intime::stops::sync::sync_once;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{body_string_contains, method, path},
};

/// When `fixtures/gtt/stop_39.json` was recorded (2026-11-04 10:46:30 +01:00). The alerts in
/// `fixtures/gtt/alerts.pb` were recorded the same day at 16:03:58; all 119 were already active.
const NOW: i64 = 1_793_785_590;
const ALERTS_PATH: &str = "/das_gtfsrt/alerts.aspx";
const ARCHIVE_PATH: &str = "/open_data/gtt_gtfs.zip";

fn file(name: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/tests/fixtures/gtt/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

/// GTT's archive with the real rows of `fixtures/gtt/stops.txt`: 26 of the stops around Porta
/// Nuova in `fixtures/all_stops.json`.
fn archive() -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    zip.start_file("stops.txt", options).unwrap();
    zip.write_all(&file("stops.txt")).unwrap();
    zip.finish().unwrap().into_inner()
}

/// Fake OpenTripPlanner (all stops, departures of stop 39) and fake GTT (alerts, archive).
async fn servers() -> (MockServer, MockServer) {
    let otp = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_string_contains("AllStops"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("all_stops.json")))
        .mount(&otp)
        .await;
    Mock::given(method("POST"))
        .and(body_string_contains("StopDepartures"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("gtt/stop_39.json")))
        .mount(&otp)
        .await;
    let gtt = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(ALERTS_PATH))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(file("alerts.pb")))
        .mount(&gtt)
        .await;
    Mock::given(method("GET"))
        .and(path(ARCHIVE_PATH))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("Last-Modified", "Sun, 01 Nov 2026 02:26:59 GMT")
                .set_body_bytes(archive()),
        )
        .mount(&gtt)
        .await;
    (otp, gtt)
}

/// The app with GTT's data on, the stop index synced and, when `with_static`, the static data
/// downloaded; the alerts are read once.
async fn app(otp: &MockServer, gtt: &MockServer, with_static: bool) -> TestApp {
    let options = Options {
        gtt_url: Some(format!("{}/das_gtfsrt", gtt.uri())),
        gtt_gtfs_url: with_static.then(|| format!("{}{ARCHIVE_PATH}", gtt.uri())),
        ..Options::default()
    };
    let app = common::test_app_with(&otp.uri(), at(NOW), Duration::from_secs(2), options).await;
    sync_once(&app.stops, &app.upstream, at(NOW)).await.unwrap();
    if let Some(gtt_static) = &app.gtt_static {
        gtt_static.check(at(NOW)).await.unwrap();
    }
    let alerts = app.alerts.as_ref().unwrap();
    assert_eq!(alerts.refresh(at(NOW)).await.unwrap(), 119);
    app
}

fn titles(json: &serde_json::Value) -> Vec<&str> {
    json["alerts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|alert| alert["title"].as_str().unwrap())
        .collect()
}

#[tokio::test]
async fn status_reports_alerts_and_static_data() {
    let (otp, gtt) = servers().await;
    let app = app(&otp, &gtt, true).await;

    let (status, json) = get(&app.router, "/api/status").await;
    assert_eq!(status, StatusCode::OK);
    let alerts = &json["gtt"]["alerts"];
    assert_eq!(alerts["feed"]["ok"], true);
    assert_eq!(alerts["read"]["alerts"], 119);
    // The fixture's static data knows 26 stops: most stops named by alerts are unknown.
    assert_eq!(alerts["read"]["unknown_stops"], 3_971);
    let static_data = &json["gtt"]["static_data"];
    assert_eq!(static_data["download"]["ok"], true);
    assert_eq!(static_data["data"]["stops"], 26);
    assert_eq!(static_data["data"]["version"], "2026-11-01T02:26:59Z");
}

#[tokio::test]
async fn a_failed_read_keeps_the_last_alerts() {
    let (otp, gtt) = servers().await;
    let app = app(&otp, &gtt, true).await;
    gtt.reset().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&gtt)
        .await;

    let alerts = app.alerts.as_ref().unwrap();
    assert!(alerts.refresh(at(NOW + 300)).await.is_err());
    let (_, json) = get(&app.router, "/api/alerts").await;
    assert_eq!(json["alerts"].as_array().unwrap().len(), 119);
    assert_eq!(json["read_at"], "2026-11-04T10:46:30+01:00");
    let (_, json) = get(&app.router, "/api/status").await;
    assert_eq!(json["gtt"]["alerts"]["feed"]["ok"], false);
    assert_eq!(
        json["gtt"]["alerts"]["feed"]["last_error"],
        "upstream returned HTTP 503"
    );
}

#[tokio::test]
async fn departures_carry_the_alerts_of_the_stop() {
    let (otp, gtt) = servers().await;
    let app = app(&otp, &gtt, true).await;

    // The line filter does not filter alerts: the web app filters on the data it has.
    let (status, json) = get(&app.router, "/api/stops/39/departures?lines=4").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        titles(&json),
        [
            "linea 67 e 67F deviate e limitate in piazza Marconi ( Moncalieri ).",
            "Linea 35 deviata in direzione via Amendola (Nichelino).",
            "Linea 67 Festivo",
            "Linea 68+ deviata in direzione Porta Nuova.",
            "Linea 68 deviata in entrambe le direzioni.",
            "Linea 6 deviata in direzione piazza Carlo Felice.",
            "Linea 9 deviata in entrambe le direzioni.",
            "Linea 35 capolinea arretrato via Artom.",
        ]
    );
    let detour = &json["alerts"][6];
    assert_eq!(detour["id"], "6aa2a2bc22bb480c377bee29");
    assert_eq!(
        detour["lines"],
        serde_json::json!([{"id": "gtt:9U", "name": "9", "mode": "TRAM"}])
    );
    assert_eq!(detour["url"], "http://www.gtt.to.it/cms/percorari/urbano");
    assert!(
        detour["description"]
            .as_str()
            .unwrap()
            .starts_with("Da lunedì 14 settembre 2026 e sino a nuove comunicazioni.\nDirezione")
    );
    // Summer time in September: the period keeps its own offset.
    assert_eq!(detour["start"], "2026-09-15T05:00:00+02:00");
    assert_eq!(detour["end"], "2027-09-15T05:00:00+02:00");

    // Line 9 has another alert, about a stop suspended elsewhere: not shown here.
    let (_, json) = get(&app.router, "/api/alerts?line=9").await;
    assert_eq!(
        titles(&json),
        [
            "Linea 9 deviata in entrambe le direzioni.",
            "Fermata 558 \"Palagiustizia\" temporaneamente sospesa",
        ]
    );
}

#[tokio::test]
async fn alerts_by_stop_and_line() {
    let (otp, gtt) = servers().await;
    let app = app(&otp, &gtt, true).await;

    let (status, json) = get(&app.router, "/api/alerts?stop=39").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["alerts"].as_array().unwrap().len(), 8);
    assert_eq!(json["generated_at"], "2026-11-04T10:46:30+01:00");

    let (_, json) = get(&app.router, "/api/alerts?stop=gtt:39&line=68%2B").await;
    assert_eq!(
        titles(&json),
        ["Linea 68+ deviata in direzione Porta Nuova."]
    );

    // A metro stop: its alert names line M1 and no stop.
    let (_, json) = get(&app.router, "/api/alerts?stop=8224").await;
    assert_eq!(titles(&json), ["Situazione aggiornata ascensori metro."]);

    let (_, json) = get(&app.router, "/api/alerts?stop=1415").await;
    assert_eq!(json["alerts"], serde_json::json!([]));

    let (_, json) = get(&app.router, "/api/alerts").await;
    assert_eq!(json["alerts"].as_array().unwrap().len(), 119);

    let (status, json) = get(&app.router, "/api/alerts?stop=39%20OR%201").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(json["error"]["code"], "invalid_request");
    let (status, json) = get(&app.router, "/api/alerts?stop=999999").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(json["error"]["code"], "stop_not_found");
}

#[tokio::test]
async fn without_static_data_alerts_are_linked_by_line() {
    let (otp, gtt) = servers().await;
    let app = app(&otp, &gtt, false).await;

    let (_, json) = get(&app.router, "/api/status").await;
    assert_eq!(json["gtt"]["static_data"], serde_json::Value::Null);
    // Every alert on a line serving stop 39 is shown, stop alerts of other stops too.
    let (_, json) = get(&app.router, "/api/alerts?stop=39").await;
    let titles = titles(&json);
    assert!(titles.len() > 8, "{titles:?}");
    assert!(titles.contains(&"Fermata 558 \"Palagiustizia\" temporaneamente sospesa"));
}

#[tokio::test]
async fn without_gtt_there_are_no_alerts() {
    let (otp, _gtt) = servers().await;
    let app = common::test_app(&otp.uri(), at(NOW), Duration::from_secs(2)).await;

    let (status, json) = get(&app.router, "/api/stops/39/departures").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["alerts"], serde_json::json!([]));
    let (status, json) = get(&app.router, "/api/alerts").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["alerts"], serde_json::json!([]));
    assert_eq!(json["read_at"], serde_json::Value::Null);
}
