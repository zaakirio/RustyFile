mod helpers;

use helpers::TestApp;

#[tokio::test]
async fn health_check_returns_ok() {
    let app = TestApp::spawn().await;

    let resp = app
        .client
        .get(app.url("/api/health"))
        .send()
        .await
        .expect("Failed to send health request");

    assert_eq!(resp.status(), 200);

    let body: serde_json::Value = resp.json().await.expect("Failed to parse health response");
    assert_eq!(body["status"], "ok");
}
