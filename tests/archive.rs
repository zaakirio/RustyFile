mod helpers;

use std::io::{Cursor, Read, Write};

use helpers::TestApp;
use zip::write::SimpleFileOptions;
use zip::{ZipArchive, ZipWriter};

/// Reads a response body into an in-memory ZIP archive.
fn open_zip(bytes: Vec<u8>) -> ZipArchive<Cursor<Vec<u8>>> {
    ZipArchive::new(Cursor::new(bytes)).expect("Response body is not a valid ZIP archive")
}

fn zip_entry_names(archive: &mut ZipArchive<Cursor<Vec<u8>>>) -> Vec<String> {
    (0..archive.len())
        .map(|i| archive.by_index(i).unwrap().name().to_string())
        .collect()
}

fn read_zip_entry(archive: &mut ZipArchive<Cursor<Vec<u8>>>, name: &str) -> Vec<u8> {
    let mut entry = archive
        .by_name(name)
        .unwrap_or_else(|_| panic!("Entry {name} missing from archive"));
    let mut buf = Vec::new();
    entry.read_to_end(&mut buf).expect("Failed to read entry");
    buf
}

#[tokio::test]
async fn zip_download_of_multi_file_selection() {
    let app = TestApp::spawn().await;
    let token = app.create_admin().await;

    app.write_file("alpha.txt", b"alpha contents");
    app.write_file("beta.txt", b"beta contents");

    let resp = app
        .client
        .post(app.url("/api/archive/download"))
        .bearer_auth(&token)
        .json(&serde_json::json!({ "paths": ["alpha.txt", "beta.txt"] }))
        .send()
        .await
        .expect("Failed to send download request");

    assert_eq!(resp.status(), 200);
    assert_eq!(
        resp.headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok()),
        Some("application/zip")
    );
    let disposition = resp
        .headers()
        .get("content-disposition")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    assert!(
        disposition.contains("attachment") && disposition.contains("rustyfile-2-items.zip"),
        "Unexpected Content-Disposition: {disposition}"
    );

    let bytes = resp.bytes().await.expect("Failed to read body").to_vec();
    let mut archive = open_zip(bytes);

    assert_eq!(read_zip_entry(&mut archive, "alpha.txt"), b"alpha contents");
    assert_eq!(read_zip_entry(&mut archive, "beta.txt"), b"beta contents");
}

/// The frontend submits a real HTML form (so the browser streams the
/// download natively); the endpoint must accept the urlencoded variant too.
#[tokio::test]
async fn zip_download_accepts_html_form_posts() {
    let app = TestApp::spawn().await;
    let token = app.create_admin().await;

    app.write_file("one.txt", b"1");
    app.write_file("two.txt", b"2");

    let resp = app
        .client
        .post(app.url("/api/archive/download"))
        .bearer_auth(&token)
        .form(&[("paths", r#"["one.txt","two.txt"]"#)])
        .send()
        .await
        .expect("Failed to send form download request");

    assert_eq!(resp.status(), 200);
    let bytes = resp.bytes().await.expect("Failed to read body").to_vec();
    let mut archive = open_zip(bytes);
    let names = zip_entry_names(&mut archive);
    assert!(names.contains(&"one.txt".to_string()), "{names:?}");
    assert!(names.contains(&"two.txt".to_string()), "{names:?}");
}

#[tokio::test]
async fn zip_download_of_directory_includes_nested_files_and_skips_symlinks() {
    let app = TestApp::spawn().await;
    let token = app.create_admin().await;

    app.write_file("docs/readme.md", b"# readme");
    app.write_file("docs/nested/deep.txt", b"deep file");

    // Symlink inside the directory must be skipped (house rule).
    #[cfg(unix)]
    std::os::unix::fs::symlink(
        app.root_dir.path().join("docs/readme.md"),
        app.root_dir.path().join("docs/link.md"),
    )
    .expect("Failed to create symlink");

    // GET variant: single-directory download via query parameter.
    let resp = app
        .client
        .get(app.url("/api/archive/download?path=docs"))
        .bearer_auth(&token)
        .send()
        .await
        .expect("Failed to send download request");

    assert_eq!(resp.status(), 200);
    let disposition = resp
        .headers()
        .get("content-disposition")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    assert!(
        disposition.contains("docs.zip"),
        "Unexpected Content-Disposition: {disposition}"
    );

    let bytes = resp.bytes().await.expect("Failed to read body").to_vec();
    let mut archive = open_zip(bytes);
    let names = zip_entry_names(&mut archive);

    assert!(names.contains(&"docs/readme.md".to_string()), "{names:?}");
    assert!(
        names.contains(&"docs/nested/deep.txt".to_string()),
        "{names:?}"
    );
    assert!(
        !names.iter().any(|n| n.contains("link.md")),
        "Symlink was not skipped: {names:?}"
    );

    assert_eq!(
        read_zip_entry(&mut archive, "docs/nested/deep.txt"),
        b"deep file"
    );
}

#[tokio::test]
async fn extract_round_trips_a_zip() {
    let app = TestApp::spawn().await;
    let token = app.create_admin().await;

    // Build a zip on disk inside the root.
    let zip_path = app.root_dir.path().join("bundle.zip");
    {
        let file = std::fs::File::create(&zip_path).expect("Failed to create zip");
        let mut writer = ZipWriter::new(file);
        writer
            .start_file("hello.txt", SimpleFileOptions::default())
            .unwrap();
        writer.write_all(b"hello from zip").unwrap();
        writer
            .add_directory("sub/", SimpleFileOptions::default())
            .unwrap();
        writer
            .start_file("sub/inner.txt", SimpleFileOptions::default())
            .unwrap();
        writer.write_all(b"inner file").unwrap();
        writer.finish().unwrap();
    }

    let resp = app
        .client
        .post(app.url("/api/archive/extract"))
        .bearer_auth(&token)
        .json(&serde_json::json!({ "path": "bundle.zip" }))
        .send()
        .await
        .expect("Failed to send extract request");

    assert_eq!(resp.status(), 200);
    let json: serde_json::Value = resp.json().await.expect("Failed to parse response");
    assert_eq!(json["dest"], "bundle");

    // Default destination is a sibling directory named after the archive.
    let dest = app.root_dir.path().join("bundle");
    assert_eq!(
        std::fs::read(dest.join("hello.txt")).expect("hello.txt missing"),
        b"hello from zip"
    );
    assert_eq!(
        std::fs::read(dest.join("sub/inner.txt")).expect("sub/inner.txt missing"),
        b"inner file"
    );

    // No staging leftovers.
    let leftovers: Vec<_> = std::fs::read_dir(&dest)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with(".rustyfile_extract_"))
        .collect();
    assert!(leftovers.is_empty(), "Staging dir leaked: {leftovers:?}");
}

#[tokio::test]
async fn extract_rejects_zip_slip_entries() {
    let app = TestApp::spawn().await;
    let token = app.create_admin().await;

    let zip_path = app.root_dir.path().join("evil.zip");
    {
        let file = std::fs::File::create(&zip_path).expect("Failed to create zip");
        let mut writer = ZipWriter::new(file);
        // start_file takes the entry name verbatim (unlike
        // start_file_from_path, which sanitizes), so the traversal survives.
        writer
            .start_file("../escape.txt", SimpleFileOptions::default())
            .unwrap();
        writer.write_all(b"escaped!").unwrap();
        writer.finish().unwrap();
    }

    let resp = app
        .client
        .post(app.url("/api/archive/extract"))
        .bearer_auth(&token)
        .json(&serde_json::json!({ "path": "evil.zip" }))
        .send()
        .await
        .expect("Failed to send extract request");

    assert_eq!(resp.status(), 400, "Zip-slip entry must be rejected");

    // Nothing landed outside the root...
    let outside = app.root_dir.path().parent().unwrap().join("escape.txt");
    assert!(!outside.exists(), "Zip-slip escaped the root directory");
    // ...and nothing landed inside the destination either (abort + cleanup).
    assert!(
        !app.root_dir.path().join("evil").exists(),
        "Partial extraction was not cleaned up"
    );
}

#[tokio::test]
async fn extract_conflicts_with_existing_files_return_409() {
    let app = TestApp::spawn().await;
    let token = app.create_admin().await;

    let zip_path = app.root_dir.path().join("pack.zip");
    {
        let file = std::fs::File::create(&zip_path).expect("Failed to create zip");
        let mut writer = ZipWriter::new(file);
        writer
            .start_file("taken.txt", SimpleFileOptions::default())
            .unwrap();
        writer.write_all(b"new contents").unwrap();
        writer.finish().unwrap();
    }

    // Destination already contains a file with the same name.
    app.write_file("pack/taken.txt", b"original");

    let resp = app
        .client
        .post(app.url("/api/archive/extract"))
        .bearer_auth(&token)
        .json(&serde_json::json!({ "path": "pack.zip" }))
        .send()
        .await
        .expect("Failed to send extract request");

    assert_eq!(resp.status(), 409, "Conflicting extraction must 409");

    // Existing file untouched.
    assert_eq!(
        std::fs::read(app.root_dir.path().join("pack/taken.txt")).unwrap(),
        b"original"
    );
}

#[tokio::test]
async fn extract_round_trips_a_tar_gz() {
    let app = TestApp::spawn().await;
    let token = app.create_admin().await;

    let tar_path = app.root_dir.path().join("bundle.tar.gz");
    {
        let file = std::fs::File::create(&tar_path).expect("Failed to create tar.gz");
        let encoder = flate2::write::GzEncoder::new(file, flate2::Compression::default());
        let mut builder = tar::Builder::new(encoder);

        let data = b"from tar";
        let mut header = tar::Header::new_gnu();
        header.set_size(data.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        builder
            .append_data(&mut header, "tarred/file.txt", data.as_slice())
            .unwrap();
        builder.into_inner().unwrap().finish().unwrap();
    }

    let resp = app
        .client
        .post(app.url("/api/archive/extract"))
        .bearer_auth(&token)
        .json(&serde_json::json!({ "path": "bundle.tar.gz", "dest": "unpacked" }))
        .send()
        .await
        .expect("Failed to send extract request");

    assert_eq!(resp.status(), 200);
    assert_eq!(
        std::fs::read(app.root_dir.path().join("unpacked/tarred/file.txt")).unwrap(),
        b"from tar"
    );
}

#[tokio::test]
async fn unauthenticated_archive_requests_are_rejected() {
    let app = TestApp::spawn().await;
    // Admin exists, but these requests carry no token.
    let _token = app.create_admin().await;
    app.write_file("secret.txt", b"secret");

    let download = app
        .client
        .post(app.url("/api/archive/download"))
        .json(&serde_json::json!({ "paths": ["secret.txt"] }))
        .send()
        .await
        .expect("Failed to send download request");
    assert_eq!(download.status(), 401);

    let download_get = app
        .client
        .get(app.url("/api/archive/download?path=secret.txt"))
        .send()
        .await
        .expect("Failed to send download request");
    assert_eq!(download_get.status(), 401);

    let extract = app
        .client
        .post(app.url("/api/archive/extract"))
        .json(&serde_json::json!({ "path": "secret.txt" }))
        .send()
        .await
        .expect("Failed to send extract request");
    assert_eq!(extract.status(), 401);
}
