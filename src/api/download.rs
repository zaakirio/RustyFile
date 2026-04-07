use axum::extract::{Extension, Path, Query, State};
use axum::http::{header, HeaderMap, Response, StatusCode};
use axum::middleware;
use axum::routing::get;
use axum::{body::Body, Router};
use serde::Deserialize;
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio_util::io::ReaderStream;

use crate::api::middleware::auth::require_auth;
use crate::db::user_repo;
use crate::error::AppError;
use crate::services::file_ops;
use crate::state::AppState;

// ---------------------------------------------------------------------------
// Query parameters
// ---------------------------------------------------------------------------

/// Fix 14: Allow `?inline=true` to display in-browser instead of downloading.
#[derive(Debug, Deserialize)]
struct DownloadQuery {
    inline: Option<bool>,
}

// ---------------------------------------------------------------------------
// Range parsing
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
struct ByteRange {
    start: u64,
    end: u64, // inclusive
}

/// Parse an HTTP Range header value (e.g. "bytes=0-999", "bytes=500-", "bytes=-500").
///
/// Returns `None` for malformed or unsupported range specifications.
/// Only single-range requests are supported.
fn parse_range(header: &str, file_size: u64) -> Option<ByteRange> {
    let range_str = header.strip_prefix("bytes=")?;

    // We only support a single range (no comma-separated multipart).
    if range_str.contains(',') {
        return None;
    }

    let (start_str, end_str) = range_str.split_once('-')?;

    if start_str.is_empty() {
        // Suffix range: "-500" means last 500 bytes.
        let suffix_len: u64 = end_str.parse().ok()?;
        if suffix_len == 0 || suffix_len > file_size {
            return None;
        }
        let start = file_size - suffix_len;
        Some(ByteRange {
            start,
            end: file_size - 1,
        })
    } else if end_str.is_empty() {
        // Open-ended range: "500-" means from 500 to end.
        let start: u64 = start_str.parse().ok()?;
        if start >= file_size {
            return None;
        }
        Some(ByteRange {
            start,
            end: file_size - 1,
        })
    } else {
        // Explicit range: "0-999".
        let start: u64 = start_str.parse().ok()?;
        let mut end: u64 = end_str.parse().ok()?;

        if start >= file_size || start > end {
            return None;
        }

        // Clamp end to file boundary.
        if end >= file_size {
            end = file_size - 1;
        }

        Some(ByteRange { start, end })
    }
}

// ---------------------------------------------------------------------------
// Content-Disposition helper
// ---------------------------------------------------------------------------

/// Build a Content-Disposition header value with both ASCII and UTF-8 filename
/// parameters (RFC 5987). The `inline` parameter controls whether the browser
/// should display the content in-page or prompt a download.
fn content_disposition(filename: &str, inline: bool) -> String {
    let disposition_type = if inline { "inline" } else { "attachment" };

    // Build an ASCII-safe version by replacing non-ASCII bytes with underscores.
    let ascii_name: String = filename
        .chars()
        .map(|c| if c.is_ascii() && c != '"' { c } else { '_' })
        .collect();

    // Percent-encode for the filename* parameter (RFC 5987).
    let encoded: String = filename
        .bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b == b'.' || b == b'-' || b == b'_' {
                String::from(b as char)
            } else {
                format!("%{b:02X}")
            }
        })
        .collect();

    format!("{disposition_type}; filename=\"{ascii_name}\"; filename*=UTF-8''{encoded}")
}

// ---------------------------------------------------------------------------
// Handler
// ---------------------------------------------------------------------------

/// GET /*path -- Download a file with HTTP Range request support.
async fn download(
    State(state): State<AppState>,
    Path(user_path): Path<String>,
    Query(query): Query<DownloadQuery>,
    headers: HeaderMap,
    Extension(_user): Extension<user_repo::User>,
) -> Result<Response<Body>, AppError> {
    let resolved = file_ops::safe_resolve(&state.canonical_root, &user_path)?;

    // Ensure target is a file.
    let metadata = tokio::fs::metadata(&resolved)
        .await
        .map_err(|_| AppError::NotFound("File not found".into()))?;

    if metadata.is_dir() {
        return Err(AppError::BadRequest("Cannot download a directory".into()));
    }

    let file_size = metadata.len();

    let modified: chrono::DateTime<chrono::Utc> = metadata
        .modified()
        .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
        .into();
    let last_modified = modified.format("%a, %d %b %Y %H:%M:%S GMT").to_string();

    // Fix 5: Generate ETag from file size + last modified timestamp.
    let etag = {
        let mut hasher = blake3::Hasher::new();
        hasher.update(&file_size.to_le_bytes());
        hasher.update(&modified.timestamp().to_le_bytes());
        format!("\"{}\"", &hasher.finalize().to_hex()[..32])
    };

    // Fix 5: Check If-None-Match -- return 304 if ETag matches.
    if let Some(if_none_match) = headers.get(header::IF_NONE_MATCH).and_then(|v| v.to_str().ok())
    {
        if if_none_match == etag || if_none_match == "*" {
            return Response::builder()
                .status(StatusCode::NOT_MODIFIED)
                .header(header::ETAG, &etag)
                .body(Body::empty())
                .map_err(|e| AppError::Internal(e.to_string()));
        }
    }

    // Fix 5: Check If-Modified-Since -- return 304 if not newer.
    if let Some(ims) = headers
        .get(header::IF_MODIFIED_SINCE)
        .and_then(|v| v.to_str().ok())
    {
        if let Ok(ims_time) = chrono::DateTime::parse_from_rfc2822(ims) {
            if modified <= ims_time {
                return Response::builder()
                    .status(StatusCode::NOT_MODIFIED)
                    .header(header::ETAG, &etag)
                    .body(Body::empty())
                    .map_err(|e| AppError::Internal(e.to_string()));
            }
        }
    }

    // Determine MIME type.
    let mime = mime_guess::from_path(&resolved)
        .first_or_octet_stream()
        .to_string();

    let filename = resolved
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "download".into());

    let inline = query.inline.unwrap_or(false);
    let disposition = content_disposition(&filename, inline);

    // Check for Range header.
    let range_header = headers
        .get(header::RANGE)
        .and_then(|v| v.to_str().ok());

    match range_header.and_then(|h| parse_range(h, file_size)) {
        Some(range) => {
            serve_partial(resolved, file_size, range, &mime, &disposition, &last_modified, &etag)
                .await
        }
        None if range_header.is_some() => {
            // Range header present but unparseable -- return 416.
            let body = Body::empty();
            Response::builder()
                .status(StatusCode::RANGE_NOT_SATISFIABLE)
                .header(header::CONTENT_RANGE, format!("bytes */{file_size}"))
                .body(body)
                .map_err(|e| AppError::Internal(e.to_string()))
        }
        None => serve_full(resolved, file_size, &mime, &disposition, &last_modified, &etag).await,
    }
}

/// Serve the complete file (200 OK).
async fn serve_full(
    path: std::path::PathBuf,
    file_size: u64,
    mime: &str,
    disposition: &str,
    last_modified: &str,
    etag: &str,
) -> Result<Response<Body>, AppError> {
    let file = tokio::fs::File::open(&path).await.map_err(AppError::Io)?;
    let stream = ReaderStream::new(file);
    let body = Body::from_stream(stream);

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, mime)
        .header(header::CONTENT_LENGTH, file_size)
        .header(header::ACCEPT_RANGES, "bytes")
        .header(header::CONTENT_DISPOSITION, disposition)
        .header(header::CACHE_CONTROL, "private")
        .header(header::LAST_MODIFIED, last_modified)
        .header(header::ETAG, etag)
        .header("Content-Security-Policy", "script-src 'none';")
        .header("X-Content-Type-Options", "nosniff")
        .body(body)
        .map_err(|e| AppError::Internal(e.to_string()))
}

/// Serve a byte range of the file (206 Partial Content).
async fn serve_partial(
    path: std::path::PathBuf,
    file_size: u64,
    range: ByteRange,
    mime: &str,
    disposition: &str,
    last_modified: &str,
    etag: &str,
) -> Result<Response<Body>, AppError> {
    let mut file = tokio::fs::File::open(&path).await.map_err(AppError::Io)?;

    // Seek to the start of the range.
    file.seek(std::io::SeekFrom::Start(range.start))
        .await
        .map_err(AppError::Io)?;

    let chunk_size = range.end - range.start + 1;

    // Limit the reader to only the requested byte range.
    let limited = file.take(chunk_size);
    let stream = ReaderStream::new(limited);
    let body = Body::from_stream(stream);

    let content_range = format!("bytes {}-{}/{file_size}", range.start, range.end);

    Response::builder()
        .status(StatusCode::PARTIAL_CONTENT)
        .header(header::CONTENT_TYPE, mime)
        .header(header::CONTENT_LENGTH, chunk_size)
        .header(header::CONTENT_RANGE, content_range)
        .header(header::ACCEPT_RANGES, "bytes")
        .header(header::CONTENT_DISPOSITION, disposition)
        .header(header::CACHE_CONTROL, "private")
        .header(header::LAST_MODIFIED, last_modified)
        .header(header::ETAG, etag)
        .header("Content-Security-Policy", "script-src 'none';")
        .header("X-Content-Type-Options", "nosniff")
        .body(body)
        .map_err(|e| AppError::Internal(e.to_string()))
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

pub fn routes(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/{*path}", get(download))
        .route_layer(middleware::from_fn_with_state(state, require_auth))
}
