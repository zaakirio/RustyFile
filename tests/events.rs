mod helpers;

use std::time::Duration;

use futures_util::StreamExt;
use helpers::TestApp;

/// The watcher debounces events (500ms) and platform notification latency
/// varies, so allow plenty of time for the event to arrive.
const EVENT_TIMEOUT: Duration = Duration::from_secs(15);

#[tokio::test]
async fn events_rejects_unauthenticated() {
    let app = TestApp::spawn().await;

    let resp = app
        .client
        .get(app.url("/api/events"))
        .send()
        .await
        .expect("Request failed");

    assert_eq!(resp.status(), 401);
}

#[tokio::test]
async fn events_streams_dir_changed_on_file_creation() {
    let app = TestApp::spawn().await;
    let token = app.create_admin().await;

    let resp = app
        .client
        .get(app.url("/api/events"))
        .header("Cookie", format!("rustyfile_token={token}"))
        .send()
        .await
        .expect("Request failed");

    assert_eq!(resp.status(), 200);
    let content_type = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .expect("Missing Content-Type")
        .to_str()
        .unwrap()
        .to_string();
    assert!(
        content_type.starts_with("text/event-stream"),
        "Unexpected Content-Type: {content_type}"
    );

    let mut stream = resp.bytes_stream();

    // Give the platform watcher a moment to become ready, then trigger a
    // filesystem change inside the watched root.
    tokio::time::sleep(Duration::from_millis(500)).await;
    app.write_file("watched-dir/hello.txt", b"hello live updates");

    // Read SSE frames until the dir_changed event for "watched-dir" shows up
    // (keep-alive comments and other events may arrive first).
    let deadline = tokio::time::Instant::now() + EVENT_TIMEOUT;
    let mut received = String::new();
    let mut found = false;

    while tokio::time::Instant::now() < deadline {
        let remaining = deadline - tokio::time::Instant::now();
        match tokio::time::timeout(remaining, stream.next()).await {
            Ok(Some(Ok(chunk))) => {
                received.push_str(&String::from_utf8_lossy(&chunk));
                if received.contains(r#""type":"dir_changed""#)
                    && received.contains(r#""path":"watched-dir""#)
                {
                    found = true;
                    break;
                }
            }
            Ok(Some(Err(e))) => panic!("SSE stream errored: {e}"),
            Ok(None) => panic!("SSE stream ended unexpectedly. Received so far: {received}"),
            Err(_) => break, // timeout
        }
    }

    assert!(
        found,
        "Did not receive dir_changed event for watched-dir within {EVENT_TIMEOUT:?}. Received: {received}"
    );
}
