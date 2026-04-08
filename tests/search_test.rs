mod helpers;

use helpers::TestApp;

#[tokio::test]
async fn search_requires_auth() {
    let app = TestApp::spawn().await;
    let resp = app
        .client
        .get(app.url("/api/fs/search?q=test"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
}

#[tokio::test]
async fn search_requires_query_param() {
    let app = TestApp::spawn().await;
    let token = app.create_admin().await;
    let resp = app
        .client
        .get(app.url("/api/fs/search?q="))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
}

#[tokio::test]
async fn search_finds_files_by_name() {
    let app = TestApp::spawn().await;
    let token = app.create_admin().await;
    app.write_file("readme.txt", b"hello");
    app.write_file("docs/readme.md", b"world");
    app.write_file("other.log", b"data");
    app.reindex().await;

    let resp = app
        .client
        .get(app.url("/api/fs/search?q=readme"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    let results = body["results"].as_array().unwrap();
    assert_eq!(results.len(), 2);
    assert_eq!(body["total"], 2);
}

#[tokio::test]
async fn search_scoped_to_directory() {
    let app = TestApp::spawn().await;
    let token = app.create_admin().await;
    app.write_file("root.txt", b"root");
    app.write_file("sub/nested.txt", b"nested");
    app.write_file("sub/deep/file.txt", b"deep");
    app.reindex().await;

    let resp = app
        .client
        .get(app.url("/api/fs/search?q=txt&path=sub"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    let results = body["results"].as_array().unwrap();
    // Only nested.txt and deep/file.txt, not root.txt
    assert_eq!(results.len(), 2);
}

#[tokio::test]
async fn search_pagination() {
    let app = TestApp::spawn().await;
    let token = app.create_admin().await;
    for i in 0..5 {
        app.write_file(&format!("file{i}.txt"), b"data");
    }
    app.reindex().await;

    let resp = app
        .client
        .get(app.url("/api/fs/search?q=file&limit=2&offset=0"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["results"].as_array().unwrap().len(), 2);
    assert_eq!(body["total"], 5);

    // Page 2
    let resp = app
        .client
        .get(app.url("/api/fs/search?q=file&limit=2&offset=2"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["results"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn search_filters_by_directory_type() {
    let app = TestApp::spawn().await;
    let token = app.create_admin().await;
    app.write_file("mydir/.keep", b"");
    app.write_file("myfile.txt", b"data");
    app.reindex().await;

    // Search for directories only — "mydir" should match
    let resp = app
        .client
        .get(app.url("/api/fs/search?q=my&type=dir"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    let results = body["results"].as_array().unwrap();
    assert!(results.iter().all(|r| r["is_dir"].as_bool().unwrap()));
}
