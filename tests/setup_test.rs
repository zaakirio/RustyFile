mod helpers;

use helpers::TestApp;

#[tokio::test]
async fn setup_status_shows_required_initially() {
    let app = TestApp::spawn().await;

    let resp = app
        .client
        .get(app.url("/api/setup/status"))
        .send()
        .await
        .expect("Failed to send setup status request");

    assert_eq!(resp.status(), 200);

    let body: serde_json::Value = resp.json().await.expect("Failed to parse setup status");
    assert_eq!(body["setup_required"], true);
}

#[tokio::test]
async fn create_admin_succeeds() {
    let app = TestApp::spawn().await;

    let body = serde_json::json!({
        "username": "admin",
        "password": "supersecure1",
        "password_confirm": "supersecure1"
    });

    let resp = app
        .client
        .post(app.url("/api/setup/admin"))
        .json(&body)
        .send()
        .await
        .expect("Failed to send create admin request");

    assert_eq!(resp.status(), 201);

    let cookie = resp
        .headers()
        .get(reqwest::header::SET_COOKIE)
        .expect("Response should contain a Set-Cookie header");
    let cookie_str = cookie.to_str().unwrap();
    assert!(
        cookie_str.contains("rustyfile_token="),
        "Cookie should contain rustyfile_token"
    );

    let json: serde_json::Value = resp.json().await.expect("Failed to parse admin response");
    assert!(
        json["user"].is_object(),
        "Response should contain a user object"
    );
    assert_eq!(json["user"]["username"], "admin");
    assert_eq!(json["user"]["role"], "admin");
}

#[tokio::test]
async fn setup_not_required_after_admin_created() {
    let app = TestApp::spawn().await;
    let _ = app.create_admin().await;

    let resp = app
        .client
        .get(app.url("/api/setup/status"))
        .send()
        .await
        .expect("Failed to send setup status request");

    assert_eq!(resp.status(), 200);

    let body: serde_json::Value = resp.json().await.expect("Failed to parse setup status");
    assert_eq!(body["setup_required"], false);
}

#[tokio::test]
async fn create_admin_twice_returns_conflict() {
    let app = TestApp::spawn().await;
    let _ = app.create_admin().await;

    let body = serde_json::json!({
        "username": "admin2",
        "password": "supersecure2",
        "password_confirm": "supersecure2"
    });

    let resp = app
        .client
        .post(app.url("/api/setup/admin"))
        .json(&body)
        .send()
        .await
        .expect("Failed to send second admin request");

    assert_eq!(resp.status(), 409);
}

#[tokio::test]
async fn create_admin_rejects_short_password() {
    let app = TestApp::spawn().await;

    let body = serde_json::json!({
        "username": "admin",
        "password": "short",
        "password_confirm": "short"
    });

    let resp = app
        .client
        .post(app.url("/api/setup/admin"))
        .json(&body)
        .send()
        .await
        .expect("Failed to send short password request");

    assert_eq!(resp.status(), 400);

    let json: serde_json::Value = resp.json().await.expect("Failed to parse error response");
    let error_msg = json["error"].as_str().unwrap_or("");
    assert!(
        error_msg.contains("at least"),
        "Error should mention minimum length, got: {error_msg}"
    );
}

#[tokio::test]
async fn create_admin_rejects_mismatched_passwords() {
    let app = TestApp::spawn().await;

    let body = serde_json::json!({
        "username": "admin",
        "password": "supersecure1",
        "password_confirm": "different123"
    });

    let resp = app
        .client
        .post(app.url("/api/setup/admin"))
        .json(&body)
        .send()
        .await
        .expect("Failed to send mismatched password request");

    assert_eq!(resp.status(), 400);

    let json: serde_json::Value = resp.json().await.expect("Failed to parse error response");
    let error_msg = json["error"].as_str().unwrap_or("");
    assert!(
        error_msg.contains("do not match"),
        "Error should mention password mismatch, got: {error_msg}"
    );
}
