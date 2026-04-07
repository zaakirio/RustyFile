mod helpers;

use helpers::TestApp;

#[tokio::test]
async fn root_serves_index_html() {
    let app = TestApp::spawn().await;

    let resp = app
        .client
        .get(app.url("/"))
        .send()
        .await
        .expect("Failed to send request");

    assert_eq!(resp.status(), 200);
    let content_type = resp
        .headers()
        .get("content-type")
        .expect("Missing content-type")
        .to_str()
        .unwrap();
    assert!(
        content_type.contains("text/html"),
        "Expected text/html, got {content_type}"
    );

    let body = resp.text().await.unwrap();
    assert!(body.contains("<!DOCTYPE html>") || body.contains("<!doctype html>"));
}

#[tokio::test]
async fn spa_route_serves_index_html() {
    let app = TestApp::spawn().await;

    // React Router paths like /browse/foo should return index.html
    let resp = app
        .client
        .get(app.url("/browse/some/path"))
        .send()
        .await
        .expect("Failed to send request");

    assert_eq!(resp.status(), 200);
    let content_type = resp
        .headers()
        .get("content-type")
        .expect("Missing content-type")
        .to_str()
        .unwrap();
    assert!(
        content_type.contains("text/html"),
        "Expected text/html for SPA route, got {content_type}"
    );
}

#[tokio::test]
async fn spa_route_with_dot_in_middle_segment_serves_index_html() {
    let app = TestApp::spawn().await;

    // Paths like /browse/v1.2/detail have a dot in a middle segment, not a
    // file extension in the last segment.  They are SPA routes and must fall
    // back to index.html rather than returning 404.
    let resp = app
        .client
        .get(app.url("/browse/v1.2/detail"))
        .send()
        .await
        .expect("Failed to send request");

    assert_eq!(resp.status(), 200);
    let content_type = resp
        .headers()
        .get("content-type")
        .expect("Missing content-type")
        .to_str()
        .unwrap();
    assert!(
        content_type.contains("text/html"),
        "Expected text/html for SPA route with dot in middle segment, got {content_type}"
    );
}

#[tokio::test]
async fn missing_asset_returns_404() {
    let app = TestApp::spawn().await;

    let resp = app
        .client
        .get(app.url("/nonexistent.js"))
        .send()
        .await
        .expect("Failed to send request");

    assert_eq!(resp.status(), 404);
}

#[tokio::test]
async fn api_routes_still_work() {
    let app = TestApp::spawn().await;

    let resp = app
        .client
        .get(app.url("/api/health"))
        .send()
        .await
        .expect("Failed to send request");

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["status"], "ok");
}
