//! Archive endpoints: download a selection as a streamed ZIP, and extract
//! an archive on the server.
//!
//! Registered under the SLOW route group (300s timeout): zipping a large
//! directory or inflating a big archive can legitimately take minutes.

use std::collections::HashSet;
use std::io::Write;

use axum::body::{Body, Bytes};
use axum::extract::{Extension, FromRequest, Query, State};
use axum::http::{header, Request, Response, StatusCode};
use axum::middleware;
use axum::routing::post;
use axum::{Form, Json, Router};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::api::download::content_disposition;
use crate::api::middleware::auth::require_auth;
use crate::db::user_repo;
use crate::error::AppError;
use crate::services::archive::{self, dedupe_entry_name, ArchiveFormat, ZipSource};
use crate::services::file_ops;
use crate::state::AppState;

/// Upper bound on the number of *selected* paths in one download request
/// (each may still be a directory that expands to many files).
const MAX_DOWNLOAD_SELECTION: usize = 10_000;

/// Chunk size for bridging the blocking zip writer to the response body.
const ZIP_STREAM_CHUNK_BYTES: usize = 64 * 1024;

/// Backpressure: at most this many chunks buffered between the blocking zip
/// writer and the HTTP response.
const ZIP_STREAM_CHANNEL_DEPTH: usize = 8;

#[derive(Debug, Deserialize)]
struct DownloadBody {
    paths: Vec<String>,
}

/// HTML forms cannot post JSON, so the same endpoint also accepts a
/// urlencoded form whose `paths` field is a JSON-encoded string array. This
/// lets the frontend submit a real `<form>` and have the browser stream the
/// download natively (no blob buffering in JS).
#[derive(Debug, Deserialize)]
struct DownloadForm {
    paths: String,
}

#[derive(Debug, Deserialize)]
struct DownloadQuery {
    path: String,
}

#[derive(Debug, Deserialize)]
struct ExtractBody {
    path: String,
    dest: Option<String>,
}

#[derive(Debug, Serialize)]
struct ExtractResponse {
    message: String,
    dest: String,
    entries: usize,
}

/// `std::io::Write` adapter that ships chunks from the blocking zip writer
/// into an mpsc channel bridged to the response body. `blocking_send` is
/// safe here because the writer only ever runs inside `spawn_blocking`; it
/// also provides backpressure against slow clients.
struct ChannelWriter {
    tx: mpsc::Sender<Result<Bytes, std::io::Error>>,
    buf: Vec<u8>,
}

impl ChannelWriter {
    fn new(tx: mpsc::Sender<Result<Bytes, std::io::Error>>) -> Self {
        Self {
            tx,
            buf: Vec::with_capacity(ZIP_STREAM_CHUNK_BYTES),
        }
    }

    fn send_buf(&mut self) -> std::io::Result<()> {
        if self.buf.is_empty() {
            return Ok(());
        }
        let chunk = Bytes::from(std::mem::replace(
            &mut self.buf,
            Vec::with_capacity(ZIP_STREAM_CHUNK_BYTES),
        ));
        self.tx
            .blocking_send(Ok(chunk))
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::BrokenPipe, "client disconnected"))
    }
}

impl Write for ChannelWriter {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        self.buf.extend_from_slice(data);
        if self.buf.len() >= ZIP_STREAM_CHUNK_BYTES {
            self.send_buf()?;
        }
        Ok(data.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.send_buf()
    }
}

/// Resolves each selected path through the jail and streams a ZIP of the
/// selection. Shared by the POST (multi-select) and GET (single directory)
/// handlers, and by public share downloads of directories.
pub(crate) async fn stream_zip_response(
    state: &AppState,
    user_paths: Vec<String>,
) -> Result<Response<Body>, AppError> {
    if user_paths.is_empty() {
        return Err(AppError::BadRequest("No paths selected".into()));
    }
    if user_paths.len() > MAX_DOWNLOAD_SELECTION {
        return Err(AppError::BadRequest(format!(
            "Too many items selected (limit: {MAX_DOWNLOAD_SELECTION})"
        )));
    }

    let mut sources = Vec::with_capacity(user_paths.len());
    let mut used_names: HashSet<String> = HashSet::new();

    for user_path in &user_paths {
        // Every path goes through the same jail as all other file access.
        let resolved = file_ops::safe_resolve(&state.canonical_root, user_path)?;

        if tokio::fs::symlink_metadata(&resolved).await.is_err() {
            return Err(AppError::NotFound(format!("Not found: {user_path}")));
        }

        let base_name = resolved
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "root".into());

        sources.push(ZipSource {
            abs_path: resolved,
            entry_name: dedupe_entry_name(&base_name, &mut used_names),
        });
    }

    let zip_name = if sources.len() == 1 {
        format!("{}.zip", sources[0].entry_name)
    } else {
        format!("rustyfile-{}-items.zip", sources.len())
    };

    let (tx, rx) = mpsc::channel::<Result<Bytes, std::io::Error>>(ZIP_STREAM_CHANNEL_DEPTH);

    tokio::task::spawn_blocking(move || {
        let writer = ChannelWriter::new(tx.clone());
        if let Err(err) = archive::write_zip(writer, &sources) {
            tracing::warn!("ZIP download aborted: {err}");
            // Erroring the body stream truncates the download instead of
            // silently producing a corrupt archive.
            let _ = tx.blocking_send(Err(std::io::Error::other(err.to_string())));
        }
    });

    let stream = futures_util::stream::unfold(rx, |mut rx| async move {
        rx.recv().await.map(|chunk| (chunk, rx))
    });

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/zip")
        .header(
            header::CONTENT_DISPOSITION,
            content_disposition(&zip_name, false),
        )
        .header(header::CACHE_CONTROL, "no-store")
        .header("X-Content-Type-Options", "nosniff")
        .body(Body::from_stream(stream))?)
}

/// `POST /api/archive/download` — body is either JSON `{"paths": [...]}` or
/// a urlencoded form with a JSON-encoded `paths` field (native browser
/// streaming via `<form>` submission).
async fn download_post(
    State(state): State<AppState>,
    Extension(_user): Extension<user_repo::User>,
    request: Request<Body>,
) -> Result<Response<Body>, AppError> {
    let is_json = request
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|ct| ct.starts_with("application/json"))
        .unwrap_or(false);

    let paths: Vec<String> = if is_json {
        let Json(body) = Json::<DownloadBody>::from_request(request, &state)
            .await
            .map_err(|e| AppError::BadRequest(format!("Invalid request body: {e}")))?;
        body.paths
    } else {
        let Form(form) = Form::<DownloadForm>::from_request(request, &state)
            .await
            .map_err(|e| AppError::BadRequest(format!("Invalid request body: {e}")))?;
        serde_json::from_str(&form.paths)
            .map_err(|_| AppError::BadRequest("paths must be a JSON string array".into()))?
    };

    stream_zip_response(&state, paths).await
}

/// `GET /api/archive/download?path=<dir>` — single-item variant, handy for
/// plain links to download a directory.
async fn download_get(
    State(state): State<AppState>,
    Query(query): Query<DownloadQuery>,
    Extension(_user): Extension<user_repo::User>,
) -> Result<Response<Body>, AppError> {
    stream_zip_response(&state, vec![query.path]).await
}

/// `POST /api/archive/extract` — extracts a `.zip` / `.tar.gz` / `.tgz`
/// archive on the server. Default destination is a sibling directory named
/// after the archive. Never overwrites: any conflict aborts with 409.
async fn extract(
    State(state): State<AppState>,
    Extension(_user): Extension<user_repo::User>,
    Json(body): Json<ExtractBody>,
) -> Result<Json<ExtractResponse>, AppError> {
    let archive_path = file_ops::safe_resolve(&state.canonical_root, &body.path)?;

    let metadata = tokio::fs::metadata(&archive_path)
        .await
        .map_err(|_| AppError::NotFound("Archive not found".into()))?;
    if !metadata.is_file() {
        return Err(AppError::BadRequest("Not a file".into()));
    }

    let file_name = archive_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();

    let format = ArchiveFormat::from_name(&file_name).ok_or_else(|| {
        AppError::BadRequest("Unsupported archive type (expected .zip, .tar.gz or .tgz)".into())
    })?;

    // Default destination: sibling directory named after the archive.
    let dest_rel = match &body.dest {
        Some(dest) => dest.clone(),
        None => {
            let stem = format.strip_extension(&file_name);
            match body.path.rsplit_once('/') {
                Some((parent, _)) => format!("{parent}/{stem}"),
                None => stem,
            }
        }
    };

    let dest = file_ops::safe_resolve(&state.canonical_root, &dest_rel)?;

    if let Ok(meta) = tokio::fs::metadata(&dest).await {
        if !meta.is_dir() {
            return Err(AppError::Conflict(format!(
                "Destination '{dest_rel}' exists and is not a directory"
            )));
        }
    }

    let report = {
        let archive_path = archive_path.clone();
        let dest = dest.clone();
        tokio::task::spawn_blocking(move || archive::extract_archive(format, &archive_path, &dest))
            .await
            .map_err(|e| AppError::Internal(format!("extract task panicked: {e}")))??
    };

    // The filesystem watcher also picks these up (search index + SSE), but
    // invalidate the listing cache immediately like other mutation handlers.
    state.dir_cache.invalidate(&dest.to_string_lossy()).await;
    if let Some(parent) = dest.parent() {
        state.dir_cache.invalidate(&parent.to_string_lossy()).await;
    }

    let indexer = state.search_indexer.clone();
    let idx_path = dest_rel.clone();
    tokio::spawn(async move {
        use crate::services::SearchIndex;
        let _ = indexer.upsert(&idx_path).await;
    });

    Ok(Json(ExtractResponse {
        message: format!("Extracted {} entries to {dest_rel}", report.entries),
        dest: dest_rel,
        entries: report.entries,
    }))
}

pub fn routes(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/download", post(download_post).get(download_get))
        .route("/extract", post(extract))
        .route_layer(middleware::from_fn_with_state(state, require_auth))
}
