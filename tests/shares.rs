mod helpers;

use std::io::{Cursor, Read};

use helpers::TestApp;
use zip::ZipArchive;

// ── Helpers ────────────────────────────────────────────────────────────────

async fn create_share(
    app: &TestApp,
    token: &str,
    body: serde_json::Value,
) -> (reqwest::StatusCode, serde_json::Value) {
    let resp = app
        .client
        .post(app.url("/api/shares"))
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
        .expect("Failed to send create-share request");
    let status = resp.status();
    let json = resp.json().await.unwrap_or(serde_json::Value::Null);
    (status, json)
}

async fn create_share_ok(app: &TestApp, token: &str, body: serde_json::Value) -> String {
    let (status, json) = create_share(app, token, body).await;
    assert_eq!(status, 201, "create share failed: {json}");
    json["token"]
        .as_str()
        .expect("share response missing token")
        .to_string()
}

/// Forces a share's expiry into the past directly in the DB.
async fn expire_share(app: &TestApp, share_token: &str) {
    let conn = app.db.get().await.expect("DB conn");
    let token = share_token.to_string();
    conn.interact(move |conn| {
        let rows = conn
            .execute(
                "UPDATE shares SET expires_at = 1 WHERE token = ?1",
                rusqlite::params![token],
            )
            .expect("expire UPDATE failed");
        assert_eq!(rows, 1, "share row to expire not found");
    })
    .await
    .expect("interact failed");
}

// ── Management API ─────────────────────────────────────────────────────────

#[tokio::test]
async fn create_list_delete_share() {
    let app = TestApp::spawn().await;
    let token = app.create_admin().await;
    app.write_file("report.pdf", b"pdf bytes");

    let (status, created) = create_share(
        &app,
        &token,
        serde_json::json!({ "path": "report.pdf", "kind": "download", "expires_in_hours": 24 }),
    )
    .await;
    assert_eq!(status, 201);
    let share_token = created["token"].as_str().unwrap().to_string();
    assert_eq!(created["path"], "report.pdf");
    assert_eq!(created["kind"], "download");
    assert_eq!(created["has_password"], false);
    assert_eq!(created["exists"], true);
    assert!(created["expires_at"].as_i64().is_some());
    assert!(
        created.get("password_hash").is_none(),
        "hash must never be serialized"
    );

    // Token is 32 bytes base64url-no-pad.
    assert_eq!(share_token.len(), 43);

    // List includes it (and never leaks password hashes).
    let resp = app
        .client
        .get(app.url("/api/shares"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let list: serde_json::Value = resp.json().await.unwrap();
    let shares = list["shares"].as_array().unwrap();
    assert_eq!(shares.len(), 1);
    assert_eq!(shares[0]["token"], share_token.as_str());
    assert!(shares[0].get("password_hash").is_none());

    // Delete.
    let resp = app
        .client
        .delete(app.url(&format!("/api/shares/{share_token}")))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 204);

    // Deleting again 404s, and the list is empty.
    let resp = app
        .client
        .delete(app.url(&format!("/api/shares/{share_token}")))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);

    let list: serde_json::Value = app
        .client
        .get(app.url("/api/shares"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list["shares"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn share_management_requires_auth() {
    let app = TestApp::spawn().await;
    let token = app.create_admin().await;
    app.write_file("file.txt", b"x");

    let resp = app
        .client
        .post(app.url("/api/shares"))
        .json(&serde_json::json!({ "path": "file.txt", "kind": "download" }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);

    let resp = app.client.get(app.url("/api/shares")).send().await.unwrap();
    assert_eq!(resp.status(), 401);

    let resp = app
        .client
        .delete(app.url("/api/shares/sometoken"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);

    // Sanity: kind and path are validated for authed requests.
    let (status, _) = create_share(
        &app,
        &token,
        serde_json::json!({ "path": "file.txt", "kind": "view" }),
    )
    .await;
    assert_eq!(status, 400);

    let (status, _) = create_share(
        &app,
        &token,
        serde_json::json!({ "path": "missing.txt", "kind": "download" }),
    )
    .await;
    assert_eq!(status, 404);

    // Drop shares require an existing directory, not a file.
    let (status, _) = create_share(
        &app,
        &token,
        serde_json::json!({ "path": "file.txt", "kind": "drop" }),
    )
    .await;
    assert_eq!(status, 400);
}

// ── Public metadata ────────────────────────────────────────────────────────

#[tokio::test]
async fn public_metadata_for_valid_token() {
    let app = TestApp::spawn().await;
    let token = app.create_admin().await;
    app.write_file("docs/manual.txt", b"hello manual");

    let share_token = create_share_ok(
        &app,
        &token,
        serde_json::json!({ "path": "docs/manual.txt", "kind": "download" }),
    )
    .await;

    // No auth required.
    let resp = app
        .client
        .get(app.url(&format!("/api/public/shares/{share_token}")))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let meta: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(meta["name"], "manual.txt");
    assert_eq!(meta["kind"], "download");
    assert_eq!(meta["is_dir"], false);
    assert_eq!(meta["size"], 12);
    assert_eq!(meta["has_password"], false);
}

#[tokio::test]
async fn bogus_and_expired_tokens_are_identical_404s() {
    let app = TestApp::spawn().await;
    let token = app.create_admin().await;
    app.write_file("secret.txt", b"secret");

    // Bogus token.
    let resp = app
        .client
        .get(app.url("/api/public/shares/AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);

    // Expired token: must behave exactly like a missing one.
    let share_token = create_share_ok(
        &app,
        &token,
        serde_json::json!({ "path": "secret.txt", "kind": "download", "expires_in_hours": 1 }),
    )
    .await;
    expire_share(&app, &share_token).await;

    for path in [
        format!("/api/public/shares/{share_token}"),
        format!("/api/public/shares/{share_token}/download"),
    ] {
        let resp = app.client.get(app.url(&path)).send().await.unwrap();
        assert_eq!(resp.status(), 404, "expired share leaked via {path}");
    }
}

// ── Password flow ──────────────────────────────────────────────────────────

#[tokio::test]
async fn password_flow_verify_and_download() {
    let app = TestApp::spawn().await;
    let token = app.create_admin().await;
    app.write_file("payload.bin", b"top secret payload");

    let share_token = create_share_ok(
        &app,
        &token,
        serde_json::json!({
            "path": "payload.bin",
            "kind": "download",
            "password": "hunter2hunter2"
        }),
    )
    .await;

    let base = format!("/api/public/shares/{share_token}");

    // Metadata without password: only name + has_password (no size/type leak).
    let resp = app.client.get(app.url(&base)).send().await.unwrap();
    assert_eq!(resp.status(), 200);
    let meta: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(meta["has_password"], true);
    assert_eq!(meta["name"], "payload.bin");
    assert!(meta.get("size").is_none(), "size leaked: {meta}");
    assert!(meta.get("kind").is_none(), "kind leaked: {meta}");
    assert!(meta.get("is_dir").is_none(), "is_dir leaked: {meta}");

    // Wrong password header -> 401 (metadata and download).
    let resp = app
        .client
        .get(app.url(&base))
        .header("X-Share-Password", "wrong")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);

    let resp = app
        .client
        .get(app.url(&format!("{base}/download")))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);

    // Wrong password on verify -> 401.
    let resp = app
        .client
        .post(app.url(&format!("{base}/verify")))
        .json(&serde_json::json!({ "password": "wrong" }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);

    // Right password on verify -> short-lived download token.
    let resp = app
        .client
        .post(app.url(&format!("{base}/verify")))
        .json(&serde_json::json!({ "password": "hunter2hunter2" }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    let download_token = body["download_token"].as_str().unwrap().to_string();

    // Download via ?t= (no headers needed — plain browser navigation).
    let resp = app
        .client
        .get(app.url(&format!("{base}/download?t={download_token}")))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.bytes().await.unwrap().as_ref(), b"top secret payload");

    // A garbage download token is rejected.
    let resp = app
        .client
        .get(app.url(&format!("{base}/download?t=garbage")))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);

    // The password header also works directly on the download endpoint.
    let resp = app
        .client
        .get(app.url(&format!("{base}/download")))
        .header("X-Share-Password", "hunter2hunter2")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
}

#[tokio::test]
async fn download_token_is_share_scoped() {
    let app = TestApp::spawn().await;
    let token = app.create_admin().await;
    app.write_file("a.txt", b"file a");
    app.write_file("b.txt", b"file b");

    let share_a = create_share_ok(
        &app,
        &token,
        serde_json::json!({ "path": "a.txt", "kind": "download", "password": "password-aaa" }),
    )
    .await;
    let share_b = create_share_ok(
        &app,
        &token,
        serde_json::json!({ "path": "b.txt", "kind": "download", "password": "password-bbb" }),
    )
    .await;

    let resp = app
        .client
        .post(app.url(&format!("/api/public/shares/{share_a}/verify")))
        .json(&serde_json::json!({ "password": "password-aaa" }))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = resp.json().await.unwrap();
    let token_a = body["download_token"].as_str().unwrap();

    // Share A's download token must not unlock share B.
    let resp = app
        .client
        .get(app.url(&format!(
            "/api/public/shares/{share_b}/download?t={token_a}"
        )))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
}

// ── Downloads ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn file_download_round_trip() {
    let app = TestApp::spawn().await;
    let token = app.create_admin().await;
    let content = b"round trip contents \xF0\x9F\xA6\x80";
    app.write_file("dir/crab.txt", content);

    let share_token = create_share_ok(
        &app,
        &token,
        serde_json::json!({ "path": "dir/crab.txt", "kind": "download" }),
    )
    .await;

    let resp = app
        .client
        .get(app.url(&format!("/api/public/shares/{share_token}/download")))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    let disposition = resp
        .headers()
        .get("content-disposition")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    assert!(
        disposition.contains("attachment") && disposition.contains("crab.txt"),
        "Unexpected Content-Disposition: {disposition}"
    );
    assert_eq!(
        resp.headers()
            .get("x-content-type-options")
            .and_then(|v| v.to_str().ok()),
        Some("nosniff")
    );

    assert_eq!(resp.bytes().await.unwrap().as_ref(), content);

    // Download count incremented (best-effort, so poll briefly).
    let mut counted = false;
    for _ in 0..50 {
        let list: serde_json::Value = app
            .client
            .get(app.url("/api/shares"))
            .bearer_auth(&token)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        if list["shares"][0]["download_count"].as_i64() == Some(1) {
            counted = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert!(counted, "download_count was not incremented");
}

#[tokio::test]
async fn directory_download_is_a_valid_zip() {
    let app = TestApp::spawn().await;
    let token = app.create_admin().await;
    app.write_file("project/readme.md", b"# readme");
    app.write_file("project/src/main.rs", b"fn main() {}");

    let share_token = create_share_ok(
        &app,
        &token,
        serde_json::json!({ "path": "project", "kind": "download" }),
    )
    .await;

    let resp = app
        .client
        .get(app.url(&format!("/api/public/shares/{share_token}/download")))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(
        resp.headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok()),
        Some("application/zip")
    );

    let bytes = resp.bytes().await.unwrap().to_vec();
    let mut archive =
        ZipArchive::new(Cursor::new(bytes)).expect("Response body is not a valid ZIP");

    let mut entry = archive
        .by_name("project/src/main.rs")
        .expect("entry missing");
    let mut buf = Vec::new();
    entry.read_to_end(&mut buf).unwrap();
    assert_eq!(buf, b"fn main() {}");
}

// ── Drop uploads ───────────────────────────────────────────────────────────

fn multipart_file(name: &str, bytes: &[u8]) -> reqwest::multipart::Form {
    reqwest::multipart::Form::new().part(
        "file",
        reqwest::multipart::Part::bytes(bytes.to_vec()).file_name(name.to_string()),
    )
}

#[tokio::test]
async fn drop_upload_lands_in_directory_and_never_overwrites() {
    let app = TestApp::spawn().await;
    let token = app.create_admin().await;
    std::fs::create_dir_all(app.root_dir.path().join("inbox")).unwrap();

    let share_token = create_share_ok(
        &app,
        &token,
        serde_json::json!({ "path": "inbox", "kind": "drop" }),
    )
    .await;

    let upload_url = app.url(&format!("/api/public/shares/{share_token}/upload"));

    // First upload (no auth).
    let resp = app
        .client
        .post(&upload_url)
        .multipart(multipart_file("notes.txt", b"first"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["files"][0]["name"], "notes.txt");

    // Second upload with the same name gets a " (1)" suffix.
    let resp = app
        .client
        .post(&upload_url)
        .multipart(multipart_file("notes.txt", b"second"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["files"][0]["name"], "notes (1).txt");

    assert_eq!(
        std::fs::read(app.root_dir.path().join("inbox/notes.txt")).unwrap(),
        b"first"
    );
    assert_eq!(
        std::fs::read(app.root_dir.path().join("inbox/notes (1).txt")).unwrap(),
        b"second"
    );
}

#[tokio::test]
async fn non_drop_share_rejects_upload() {
    let app = TestApp::spawn().await;
    let token = app.create_admin().await;
    app.write_file("readonly.txt", b"data");

    let share_token = create_share_ok(
        &app,
        &token,
        serde_json::json!({ "path": "readonly.txt", "kind": "download" }),
    )
    .await;

    let resp = app
        .client
        .post(app.url(&format!("/api/public/shares/{share_token}/upload")))
        .multipart(multipart_file("evil.txt", b"nope"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 403);
}

#[tokio::test]
async fn drop_upload_traversal_filename_is_neutralized() {
    let app = TestApp::spawn().await;
    let token = app.create_admin().await;
    std::fs::create_dir_all(app.root_dir.path().join("inbox")).unwrap();

    let share_token = create_share_ok(
        &app,
        &token,
        serde_json::json!({ "path": "inbox", "kind": "drop" }),
    )
    .await;

    let resp = app
        .client
        .post(app.url(&format!("/api/public/shares/{share_token}/upload")))
        .multipart(multipart_file("../../escape.txt", b"jailbreak"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);
    let body: serde_json::Value = resp.json().await.unwrap();
    // Traversal collapses to the bare filename inside the drop directory.
    assert_eq!(body["files"][0]["name"], "escape.txt");

    assert!(app.root_dir.path().join("inbox/escape.txt").exists());
    assert!(!app.root_dir.path().join("escape.txt").exists());
    assert!(!app
        .root_dir
        .path()
        .parent()
        .unwrap()
        .join("escape.txt")
        .exists());
}

#[tokio::test]
async fn password_protected_drop_upload() {
    let app = TestApp::spawn().await;
    let token = app.create_admin().await;
    std::fs::create_dir_all(app.root_dir.path().join("dropbox")).unwrap();

    let share_token = create_share_ok(
        &app,
        &token,
        serde_json::json!({ "path": "dropbox", "kind": "drop", "password": "drop-secret-1" }),
    )
    .await;

    let upload_url = app.url(&format!("/api/public/shares/{share_token}/upload"));

    // No password -> 401.
    let resp = app
        .client
        .post(&upload_url)
        .multipart(multipart_file("a.txt", b"x"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);

    // With the password header -> accepted.
    let resp = app
        .client
        .post(&upload_url)
        .header("X-Share-Password", "drop-secret-1")
        .multipart(multipart_file("a.txt", b"x"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);
    assert!(app.root_dir.path().join("dropbox/a.txt").exists());
}
