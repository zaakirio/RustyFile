mod helpers;

use helpers::TestApp;

/// Create a test file with deterministic content of a known size.
fn seed_download_file(app: &TestApp) {
    // 2000 bytes of repeating ASCII pattern -- enough to exercise range requests.
    let content: Vec<u8> = (0..2000u16).map(|i| (i % 256) as u8).collect();
    app.write_file("large.bin", &content);
}

#[tokio::test]
async fn download_full_file() {
    let app = TestApp::spawn().await;
    let token = app.create_admin().await;
    seed_download_file(&app);

    let resp = app
        .client
        .get(app.url("/api/fs/download/large.bin"))
        .bearer_auth(&token)
        .send()
        .await
        .expect("Failed to send download request");

    assert_eq!(resp.status(), 200);

    let accept_ranges = resp
        .headers()
        .get("accept-ranges")
        .expect("Should have Accept-Ranges header")
        .to_str()
        .unwrap();
    assert_eq!(accept_ranges, "bytes");

    let body = resp.bytes().await.expect("Failed to read body");
    assert_eq!(body.len(), 2000, "Full download should be 2000 bytes");
}

#[tokio::test]
async fn download_range_request() {
    let app = TestApp::spawn().await;
    let token = app.create_admin().await;
    seed_download_file(&app);

    let resp = app
        .client
        .get(app.url("/api/fs/download/large.bin"))
        .bearer_auth(&token)
        .header("Range", "bytes=0-999")
        .send()
        .await
        .expect("Failed to send range request");

    assert_eq!(resp.status(), 206);

    let content_range = resp
        .headers()
        .get("content-range")
        .expect("Should have Content-Range header")
        .to_str()
        .unwrap();
    assert!(
        content_range.starts_with("bytes 0-999/2000"),
        "Content-Range should indicate bytes 0-999/2000, got: {content_range}"
    );

    let body = resp.bytes().await.expect("Failed to read body");
    assert_eq!(body.len(), 1000, "Range 0-999 should return 1000 bytes");
}

#[tokio::test]
async fn download_suffix_range() {
    let app = TestApp::spawn().await;
    let token = app.create_admin().await;
    seed_download_file(&app);

    let resp = app
        .client
        .get(app.url("/api/fs/download/large.bin"))
        .bearer_auth(&token)
        .header("Range", "bytes=-500")
        .send()
        .await
        .expect("Failed to send suffix range request");

    assert_eq!(resp.status(), 206);

    let content_range = resp
        .headers()
        .get("content-range")
        .expect("Should have Content-Range header")
        .to_str()
        .unwrap();
    assert!(
        content_range.contains("1500-1999/2000"),
        "Suffix range should cover last 500 bytes, got: {content_range}"
    );

    let body = resp.bytes().await.expect("Failed to read body");
    assert_eq!(body.len(), 500, "Suffix range should return 500 bytes");
}

#[tokio::test]
async fn download_open_ended_range() {
    let app = TestApp::spawn().await;
    let token = app.create_admin().await;
    seed_download_file(&app);

    let resp = app
        .client
        .get(app.url("/api/fs/download/large.bin"))
        .bearer_auth(&token)
        .header("Range", "bytes=500-")
        .send()
        .await
        .expect("Failed to send open-ended range request");

    assert_eq!(resp.status(), 206);

    let content_range = resp
        .headers()
        .get("content-range")
        .expect("Should have Content-Range header")
        .to_str()
        .unwrap();
    assert!(
        content_range.contains("500-1999/2000"),
        "Open-ended range should cover 500-1999, got: {content_range}"
    );

    let body = resp.bytes().await.expect("Failed to read body");
    assert_eq!(
        body.len(),
        1500,
        "Open-ended range from 500 should return 1500 bytes"
    );
}

#[tokio::test]
async fn download_without_auth() {
    let app = TestApp::spawn().await;
    let _ = app.create_admin().await;
    seed_download_file(&app);

    let resp = app
        .client
        .get(app.url("/api/fs/download/large.bin"))
        .send()
        .await
        .expect("Failed to send unauthenticated download request");

    assert_eq!(resp.status(), 401);
}
