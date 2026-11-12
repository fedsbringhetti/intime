//! GTT's GTFS static archive: download, conditional checks, failures.

use std::io::{Cursor, Write};

use chrono::{DateTime, Utc};
use intime::{
    db,
    gtt::static_data::{CheckOutcome, GttStatic, StaticError},
    upstream::UpstreamError,
};
use wiremock::{
    Mock, MockServer, Request, ResponseTemplate,
    matchers::{method, path},
};

const ARCHIVE_PATH: &str = "/open_data/gtt_gtfs.zip";
const LAST_MODIFIED: &str = "Sun, 01 Nov 2026 02:26:59 GMT";
/// 2026-11-04 04:10 +01:00.
const NOW: i64 = 1_793_761_800;

/// A zip like GTT's, with the real rows of `fixtures/gtt/stops.txt` (26 stops, 2 stations).
fn archive(stops: &str) -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    for (name, text) in [("agency.txt", "agency_id\nU\n"), ("stops.txt", stops)] {
        zip.start_file(name, options).unwrap();
        zip.write_all(text.as_bytes()).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

fn recorded_stops() -> String {
    std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/gtt/stops.txt"
    ))
    .unwrap()
}

fn at(timestamp: i64) -> DateTime<Utc> {
    DateTime::from_timestamp(timestamp, 0).unwrap()
}

async fn gtt_static(server: &MockServer) -> GttStatic {
    let pool = db::connect_in_memory().await.unwrap();
    let url = format!("{}{ARCHIVE_PATH}", server.uri()).parse().unwrap();
    GttStatic::new(pool, url).unwrap()
}

fn archive_response(stops: &str) -> ResponseTemplate {
    ResponseTemplate::new(200)
        .insert_header("Last-Modified", LAST_MODIFIED)
        .set_body_bytes(archive(stops))
}

#[tokio::test]
async fn downloads_then_asks_only_for_changes() {
    let server = MockServer::start().await;
    // Asked with the stored version, GTT answers 304; otherwise the archive.
    Mock::given(method("GET"))
        .and(path(ARCHIVE_PATH))
        // A closure, not `header(...)`: wiremock splits header values at commas.
        .and(|request: &Request| {
            request
                .headers
                .get("If-Modified-Since")
                .is_some_and(|value| value == LAST_MODIFIED)
        })
        .respond_with(ResponseTemplate::new(304))
        .with_priority(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(ARCHIVE_PATH))
        .respond_with(archive_response(&recorded_stops()))
        .mount(&server)
        .await;
    let gtt_static = gtt_static(&server).await;
    assert_eq!(gtt_static.state().await.unwrap(), None);
    assert!(gtt_static.stop_ids().await.unwrap().is_empty());

    let outcome = gtt_static.check(at(NOW)).await.unwrap();
    assert_eq!(outcome, CheckOutcome::Updated(26));
    let ids = gtt_static.stop_ids().await.unwrap();
    assert_eq!(ids.len(), 26);
    assert_eq!(ids["2843"], "gtt:39");
    assert_eq!(ids["3634"], "gtt:8224", "metro stops have codes too");
    assert!(!ids.contains_key("116589"), "stations are skipped");
    let state = gtt_static.state().await.unwrap().unwrap();
    assert_eq!(state.checked_at, at(NOW));
    assert_eq!(state.version, Some(at(1_793_500_019)));
    assert_eq!(state.stops, 26);

    let a_day_later = at(NOW + 86_400);
    let outcome = gtt_static.check(a_day_later).await.unwrap();
    assert_eq!(outcome, CheckOutcome::Unchanged);
    let state = gtt_static.state().await.unwrap().unwrap();
    assert_eq!(state.checked_at, a_day_later);
    assert_eq!(state.version, Some(at(1_793_500_019)));
    assert_eq!(gtt_static.stop_ids().await.unwrap().len(), 26);
    assert_eq!(server.received_requests().await.unwrap().len(), 2);
    assert_eq!(gtt_static.health().snapshot().ok, Some(true));
}

#[tokio::test]
async fn failures_keep_the_stored_stops() {
    let server = MockServer::start().await;
    let gtt_static = gtt_static(&server).await;
    Mock::given(method("GET"))
        .respond_with(archive_response(&recorded_stops()))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    gtt_static.check(at(NOW)).await.unwrap();

    // An archive whose stops.txt has only the header is refused.
    Mock::given(method("GET"))
        .respond_with(archive_response(recorded_stops().lines().next().unwrap()))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    let err = gtt_static.check(at(NOW + 60)).await.unwrap_err();
    assert!(matches!(err, StaticError::Empty), "{err}");

    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;
    let err = gtt_static.check(at(NOW + 120)).await.unwrap_err();
    assert!(
        matches!(err, StaticError::Download(UpstreamError::Status(503))),
        "{err}"
    );

    assert_eq!(gtt_static.stop_ids().await.unwrap().len(), 26);
    let state = gtt_static.state().await.unwrap().unwrap();
    assert_eq!(state.checked_at, at(NOW), "only successful checks count");
    let health = gtt_static.health().snapshot();
    assert_eq!(health.ok, Some(false));
    assert_eq!(health.consecutive_failures, 2);
}
