mod helpers;

use helpers::TestApp;
use rustyfile::services::SearchIndex;

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

// ── Full-text content search (FTS5) ─────────────────────────────────────

/// Snippet markers emitted by the backend (see search_index::SNIPPET_START/END).
const SNIPPET_START: char = '\u{e000}';
const SNIPPET_END: char = '\u{e001}';

#[tokio::test]
async fn content_search_finds_phrase_with_snippet() {
    let app = TestApp::spawn().await;
    let token = app.create_admin().await;
    app.write_file(
        "notes/ideas.md",
        b"# Ideas\n\nThe quick zebra jumped over the lazy firewall today.\n",
    );
    app.write_file("notes/other.md", b"nothing relevant here\n");
    app.reindex().await;

    let resp = app
        .client
        .get(app.url("/api/fs/search?q=zebra%20firewall&scope=content"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    let results = body["results"].as_array().unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["path"], "notes/ideas.md");

    // Snippet present, contains the term, and carries highlight markers.
    let snippet = results[0]["snippet"].as_str().expect("snippet missing");
    assert!(snippet.contains("zebra"), "snippet was: {snippet}");
    assert!(
        snippet.contains(SNIPPET_START),
        "no start marker: {snippet}"
    );
    assert!(snippet.contains(SNIPPET_END), "no end marker: {snippet}");
}

#[tokio::test]
async fn names_mode_does_not_match_content() {
    let app = TestApp::spawn().await;
    let token = app.create_admin().await;
    app.write_file("plain.md", b"contains the word xylophone in the body\n");
    app.reindex().await;

    // Default scope (names): body text must not match.
    let resp = app
        .client
        .get(app.url("/api/fs/search?q=xylophone"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["total"], 0);
    assert!(body["results"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn both_scope_matches_names_and_content() {
    let app = TestApp::spawn().await;
    let token = app.create_admin().await;
    app.write_file("xylophone.txt", b"unrelated body\n");
    app.write_file("body-match.md", b"a xylophone solo\n");
    app.reindex().await;

    let resp = app
        .client
        .get(app.url("/api/fs/search?q=xylophone&scope=both"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["total"], 2);
}

#[tokio::test]
async fn content_search_fts_injection_is_graceful() {
    let app = TestApp::spawn().await;
    let token = app.create_admin().await;
    app.write_file("doc.md", b"foo bar baz\n");
    app.reindex().await;

    for raw in [
        "\"foo\" OR bar*",
        "\"unbalanced",
        "(foo AND",
        "NEAR(foo bar)",
        "content:foo",
    ] {
        for scope in ["content", "both"] {
            let resp = app
                .client
                .get(app.url("/api/fs/search"))
                .query(&[("q", raw), ("scope", scope)])
                .bearer_auth(&token)
                .send()
                .await
                .unwrap();
            assert_eq!(
                resp.status(),
                200,
                "raw query {raw:?} in scope {scope} did not return 200"
            );
        }
    }
}

#[tokio::test]
async fn oversized_file_content_skipped_but_name_indexed() {
    let app = TestApp::spawn().await;
    let token = app.create_admin().await;
    // Over the 1 MiB content cap; term sits at the start so it would match
    // if the content had been indexed.
    let mut big = b"elephantine content marker\n".to_vec();
    big.resize(1024 * 1024 + 1024, b'a');
    app.write_file("huge-notes.txt", &big);
    app.reindex().await;

    // Content search: no match (content skipped).
    let resp = app
        .client
        .get(app.url("/api/fs/search?q=elephantine&scope=content"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["total"], 0);

    // Name search: file is still in the index.
    let resp = app
        .client
        .get(app.url("/api/fs/search?q=huge-notes"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["total"], 1);
}

#[tokio::test]
async fn binary_file_content_skipped() {
    let app = TestApp::spawn().await;
    let token = app.create_admin().await;
    // Text-like extension but NUL bytes in the first chunk = binary sniff.
    app.write_file("fake-text.txt", b"searchable-token\x00\x00binary tail");
    app.reindex().await;

    let resp = app
        .client
        .get(app.url("/api/fs/search?q=searchable-token&scope=content"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["total"], 0);
}

#[tokio::test]
async fn deleted_file_removed_from_content_results() {
    let app = TestApp::spawn().await;
    let token = app.create_admin().await;
    app.write_file("doomed.md", b"the ephemeral pangolin\n");
    app.reindex().await;

    let resp = app
        .client
        .get(app.url("/api/fs/search?q=pangolin&scope=content"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["total"], 1);

    // Delete on disk and via the index-maintenance path the watcher uses.
    std::fs::remove_file(app.root_dir.path().join("doomed.md")).unwrap();
    app.search_indexer.remove("doomed.md").await.unwrap();

    let resp = app
        .client
        .get(app.url("/api/fs/search?q=pangolin&scope=content"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["total"], 0);

    // A full reindex must also stay clean (no stale FTS rows resurface).
    app.reindex().await;
    let resp = app
        .client
        .get(app.url("/api/fs/search?q=pangolin&scope=content"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["total"], 0);
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
