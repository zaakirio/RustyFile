mod helpers;

use helpers::TestApp;

#[tokio::test]
async fn login_with_valid_credentials() {
    let app = TestApp::spawn().await;
    let _ = app.create_admin().await;

    let body = serde_json::json!({
        "username": "admin",
        "password": "supersecure1"
    });

    let resp = app
        .client
        .post(app.url("/api/auth/login"))
        .json(&body)
        .send()
        .await
        .expect("Failed to send login request");

    assert_eq!(resp.status(), 200);

    let cookie = resp
        .headers()
        .get(reqwest::header::SET_COOKIE)
        .expect("Response should contain a Set-Cookie header");
    let cookie_str = cookie.to_str().unwrap();
    assert!(
        cookie_str.contains("rustyfile_token="),
        "Cookie should contain rustyfile_token"
    );

    let json: serde_json::Value = resp.json().await.expect("Failed to parse login response");
    assert!(
        json["user"].is_object(),
        "Response should contain a user object"
    );
    assert_eq!(json["user"]["username"], "admin");
}

#[tokio::test]
async fn login_with_wrong_password() {
    let app = TestApp::spawn().await;
    let _ = app.create_admin().await;

    let body = serde_json::json!({
        "username": "admin",
        "password": "wrongpassword"
    });

    let resp = app
        .client
        .post(app.url("/api/auth/login"))
        .json(&body)
        .send()
        .await
        .expect("Failed to send login request");

    assert_eq!(resp.status(), 401);
}

#[tokio::test]
async fn protected_route_without_token() {
    let app = TestApp::spawn().await;
    let _ = app.create_admin().await;

    let resp = app
        .client
        .get(app.url("/api/fs"))
        .send()
        .await
        .expect("Failed to send unauthenticated request");

    assert_eq!(resp.status(), 401);
}

#[tokio::test]
async fn refresh_returns_new_token() {
    let app = TestApp::spawn().await;
    let token = app.create_admin().await;

    let resp = app
        .client
        .post(app.url("/api/auth/refresh"))
        .bearer_auth(&token)
        .send()
        .await
        .expect("Failed to send refresh request");

    assert_eq!(resp.status(), 200);

    let cookie = resp
        .headers()
        .get(reqwest::header::SET_COOKIE)
        .expect("Refresh response should contain a Set-Cookie header");
    let cookie_str = cookie.to_str().unwrap();
    let new_token = cookie_str
        .split(';')
        .next()
        .unwrap()
        .strip_prefix("rustyfile_token=")
        .expect("Cookie should start with rustyfile_token=");
    assert!(!new_token.is_empty(), "New token should not be empty");
}
