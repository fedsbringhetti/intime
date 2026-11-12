mod common;

use std::time::Duration;

use axum::http::StatusCode;
use intime::stops::sync::{SyncError, sync_once};
use serde_json::{Value, json};
use wiremock::{Mock, ResponseTemplate, matchers::method};

use common::{NOW, at, get, synced_app};

const TIMEOUT: Duration = Duration::from_secs(2);

fn ids(json: &Value) -> Vec<&str> {
    json["stops"]
        .as_array()
        .unwrap()
        .iter()
        .map(|stop| stop["id"].as_str().unwrap())
        .collect()
}

#[tokio::test]
async fn endpoints_return_503_before_the_first_sync() {
    let app = common::test_app("http://127.0.0.1:9/", at(NOW), TIMEOUT).await;

    for uri in [
        "/api/stops?q=porta",
        "/api/stops/nearby?lat=45.06&lon=7.67",
        "/api/stops/39",
    ] {
        let (status, json) = get(&app.router, uri).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{uri}");
        assert_eq!(json["error"]["code"], "index_not_ready", "{uri}");
    }
}

#[tokio::test]
async fn sync_records_time_and_count() {
    let (app, _server) = synced_app().await;

    let state = app.stops.last_sync().await.unwrap().unwrap();
    assert_eq!(state.at, at(NOW));
    assert_eq!(state.count, 44);
}

#[tokio::test]
async fn sync_replaces_the_index_without_duplicates() {
    let (app, _server) = synced_app().await;
    let uri = "/api/stops?q=porta%20nuova&feeds=all&limit=100";
    let (_, before) = get(&app.router, uri).await;

    sync_once(&app.stops, &app.upstream, at(NOW + 60))
        .await
        .unwrap();

    let (_, after) = get(&app.router, uri).await;
    assert!(!ids(&before).is_empty());
    assert_eq!(ids(&after), ids(&before));
    assert_eq!(
        app.stops.last_sync().await.unwrap().unwrap().at,
        at(NOW + 60)
    );
}

#[tokio::test]
async fn empty_upstream_response_keeps_the_current_index() {
    let (app, server) = synced_app().await;
    server.reset().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "data": { "stops": [] } })))
        .mount(&server)
        .await;

    let err = sync_once(&app.stops, &app.upstream, at(NOW + 60))
        .await
        .unwrap_err();
    assert!(matches!(err, SyncError::Empty));
    assert_eq!(app.stops.last_sync().await.unwrap().unwrap().count, 44);
    let (status, _) = get(&app.router, "/api/stops/39").await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn search_by_name_puts_exact_names_first() {
    let (app, _server) = synced_app().await;

    let (status, json) = get(&app.router, "/api/stops?q=porta%20nuova").await;

    assert_eq!(status, StatusCode::OK);
    // Names equal to the query, then names containing it, then matches in the description
    // only (e.g. ARSENALE, described as "STAZIONE PORTA NUOVA").
    assert_eq!(
        ids(&json),
        [
            "gtt:39", "gtt:40", "gtt:252", "gtt:253", "gtt:3492", "gtt:8224", "gtt:1682",
            "gtt:2010", "gtt:3582", "gtt:1415"
        ]
    );
    assert_eq!(
        json["stops"][0],
        json!({
            "id": "gtt:39",
            "feed": "gtt",
            "code": "39",
            "name": "PORTA NUOVA",
            "description": json["stops"][0]["description"],
            "lat": 45.06256,
            "lon": 7.67898,
            "mode": "BUS",
            "lines": json["stops"][0]["lines"],
        })
    );
}

#[tokio::test]
async fn search_by_code_comes_first_and_respects_feeds() {
    let (app, _server) = synced_app().await;

    let (_, json) = get(&app.router, "/api/stops?q=39").await;
    assert_eq!(ids(&json)[0], "gtt:39");
    assert!(ids(&json).iter().all(|id| id.starts_with("gtt:")));

    let (_, json) = get(&app.router, "/api/stops?q=39&feeds=all").await;
    assert_eq!(&ids(&json)[..2], ["gtt:39", "4:39"]);

    let (_, json) = get(&app.router, "/api/stops?q=gtt:40").await;
    assert_eq!(ids(&json)[0], "gtt:40");
}

#[tokio::test]
async fn search_filters_by_feed_and_limits_results() {
    let (app, _server) = synced_app().await;

    let (_, json) = get(&app.router, "/api/stops?q=porta%20nuova&feeds=2").await;
    assert_eq!(ids(&json), ["2:830000219"]);

    let (_, json) = get(&app.router, "/api/stops?q=porta%20nuova&feeds=gtt,3").await;
    assert!(ids(&json).contains(&"3:000253"));
    assert!(!ids(&json).contains(&"2:830000219"));

    let (_, json) = get(&app.router, "/api/stops?q=porta%20nuova&limit=2").await;
    assert_eq!(ids(&json), ["gtt:39", "gtt:40"]);
}

#[tokio::test]
async fn search_understands_abbreviations_and_saints() {
    let (app, _server) = synced_app().await;

    let (_, short) = get(&app.router, "/api/stops?q=c.so%20vittorio&feeds=all").await;
    let (_, long) = get(&app.router, "/api/stops?q=corso%20vittorio&feeds=all").await;
    assert_eq!(ids(&short), ids(&long));
    assert!(ids(&long).contains(&"3:000035"), "Cso Vittorio Emanuele");
    assert!(
        ids(&long).contains(&"4:26062"),
        "Corso Vittorio Emanuele II"
    );

    let (_, json) = get(&app.router, "/api/stops?q=santa%20anna&feeds=all").await;
    assert!(ids(&json).contains(&"gtt:1460"), "SANT'ANNA N.79");
    assert!(ids(&json).contains(&"4:23022"), "Via S.Anna 67");
}

#[tokio::test]
async fn search_matches_partial_and_short_words() {
    let (app, _server) = synced_app().await;

    let (_, json) = get(&app.router, "/api/stops?q=nuov").await;
    assert!(ids(&json).contains(&"gtt:39"));

    let (_, json) = get(&app.router, "/api/stops?q=porta%20n").await;
    assert!(ids(&json).contains(&"gtt:39"));

    let (_, json) = get(&app.router, "/api/stops?q=xyzzy").await;
    assert!(ids(&json).is_empty());
}

#[tokio::test]
async fn nearby_returns_closest_stops_first() {
    let (app, _server) = synced_app().await;

    let (status, json) = get(
        &app.router,
        "/api/stops/nearby?lat=45.06256&lon=7.67898&radius=150",
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    let stops = json["stops"].as_array().unwrap();
    assert_eq!(stops[0]["id"], "gtt:39");
    assert_eq!(stops[0]["distance_m"], 0);
    let distances: Vec<u64> = stops
        .iter()
        .map(|s| s["distance_m"].as_u64().unwrap())
        .collect();
    assert!(distances.is_sorted());
    assert!(distances.iter().all(|&d| d <= 150));
    assert!(stops.iter().all(|s| s["feed"] == "gtt"));

    let (_, json) = get(
        &app.router,
        "/api/stops/nearby?lat=45.06256&lon=7.67898&radius=400&feeds=all&limit=100",
    )
    .await;
    assert!(
        json["stops"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["feed"] != "gtt")
    );

    let (_, json) = get(
        &app.router,
        "/api/stops/nearby?lat=45.06256&lon=7.67898&limit=3",
    )
    .await;
    assert_eq!(json["stops"].as_array().unwrap().len(), 3);
}

#[tokio::test]
async fn details_include_naturally_sorted_lines() {
    let (app, _server) = synced_app().await;

    let (status, json) = get(&app.router, "/api/stops/39").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["id"], "gtt:39");
    let names: Vec<&str> = json["lines"]
        .as_array()
        .unwrap()
        .iter()
        .map(|line| line["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "6", "9", "9B", "33", "35", "52", "61", "64", "67", "68", "68+", "M1S"
        ]
    );
}

#[tokio::test]
async fn details_for_metro_and_unknown_stops() {
    let (app, _server) = synced_app().await;

    let (_, json) = get(&app.router, "/api/stops/gtt:8224").await;
    assert_eq!(json["code"], "8224");
    assert_eq!(json["mode"], "SUBWAY");

    let (status, json) = get(&app.router, "/api/stops/99999").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(json["error"]["code"], "stop_not_found");
}

#[tokio::test]
async fn invalid_parameters_return_400() {
    let (app, _server) = synced_app().await;

    for uri in [
        "/api/stops",
        "/api/stops?q=",
        "/api/stops?q=%20%20",
        "/api/stops?q=porta&feeds=gtt;2",
        "/api/stops?q=porta&limit=0",
        "/api/stops?q=porta&limit=abc",
        "/api/stops/nearby?lon=7.67",
        "/api/stops/nearby?lat=45.06",
        "/api/stops/nearby?lat=91&lon=7.67",
        "/api/stops/nearby?lat=45.06&lon=7.67&radius=5000",
        "/api/stops/nearby?lat=NaN&lon=7.67",
        "/api/stops/a%20b",
    ] {
        let (status, json) = get(&app.router, uri).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{uri}");
        assert_eq!(json["error"]["code"], "invalid_request", "{uri}");
    }
}
