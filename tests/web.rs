mod common;

use std::time::Duration;

use axum::{
    body::Body,
    http::{Request, StatusCode, header},
};
use intime::web::WebAssets;
use rust_embed::RustEmbed;

use common::{NOW, Options, at, get_text, offline_app, request};

const INDEX_MARKER: &str = "<title>fixture web app</title>";

#[tokio::test]
async fn root_serves_the_page_without_caching() {
    let (status, headers, body) = get_text(&offline_app().await, "/").await;

    assert_eq!(status, StatusCode::OK);
    assert!(body.contains(INDEX_MARKER));
    assert_eq!(headers[header::CONTENT_TYPE], "text/html");
    assert_eq!(headers[header::CACHE_CONTROL], "no-cache");
}

#[tokio::test]
async fn app_paths_serve_the_page() {
    let app = offline_app().await;

    for path in [
        "/fermata/39",
        "/cerca?q=porta",
        "/index.html",
        "/health/extra",
    ] {
        let (status, _, body) = get_text(&app, path).await;
        assert_eq!(status, StatusCode::OK, "{path}");
        assert!(body.contains(INDEX_MARKER), "{path}");
    }
}

#[tokio::test]
async fn service_worker_and_manifest_are_never_cached() {
    // A cached service worker would keep the phone on an old version of the app.
    let app = offline_app().await;
    for (path, content_type) in [
        ("/sw.js", "text/javascript"),
        ("/manifest.webmanifest", "application/manifest+json"),
    ] {
        let (status, headers, _) = get_text(&app, path).await;
        assert_eq!(status, StatusCode::OK, "{path}");
        assert_eq!(headers[header::CONTENT_TYPE], content_type, "{path}");
        assert_eq!(headers[header::CACHE_CONTROL], "no-cache", "{path}");
    }
}

#[tokio::test]
async fn hashed_files_are_cached_for_a_year() {
    let (status, headers, body) = get_text(&offline_app().await, "/assets/index-Ab12Cd34.js").await;

    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("fixture"));
    assert_eq!(headers[header::CONTENT_TYPE], "text/javascript");
    assert_eq!(
        headers[header::CACHE_CONTROL],
        "public, max-age=31536000, immutable"
    );
}

#[tokio::test]
async fn missing_hashed_files_and_api_paths_are_not_found() {
    let app = offline_app().await;

    for path in ["/assets/index-Old00000.js", "/api", "/api/nope"] {
        let (status, _, body) = get_text(&app, path).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
        assert!(!body.contains(INDEX_MARKER), "{path}");
    }
}

#[tokio::test]
async fn only_get_is_allowed() {
    let post = Request::post("/fermata/39").body(Body::empty()).unwrap();

    let (status, _, _) = request(&offline_app().await, post).await;

    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
}

/// A folder that does not exist, like `web/dist/` before the first `npm run build`.
#[derive(RustEmbed)]
#[folder = "tests/fixtures/no-web"]
#[allow_missing = true]
struct NoWeb;

#[tokio::test]
async fn without_a_build_the_api_still_works() {
    let web = WebAssets::from_embed::<NoWeb>();
    assert!(!web.is_built());
    let app = common::test_app_with(
        "http://127.0.0.1:9/",
        at(NOW),
        Duration::from_secs(1),
        Options {
            web,
            ..Default::default()
        },
    )
    .await
    .router;

    let (status, _, _) = get_text(&app, "/").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _, _) = get_text(&app, "/api/favorites").await;
    assert_eq!(status, StatusCode::OK);
}
