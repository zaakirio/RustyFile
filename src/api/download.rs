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

#[derive(Debug, Deserialize)]
struct DownloadQuery {
    inline: Option<bool>,
}

#[derive(Debug, Clone, Copy)]
struct ByteRange {
    start: u64,
    end: u64, // inclusive
}

/// Only single-range requests are supported.
fn parse_range(header: &str, file_size: u64) -> Option<ByteRange> {
    let range_str = header.strip_prefix("bytes=")?;

    if range_str.contains(',') {
        return None;
    }

    let (start_str, end_str) = range_str.split_once('-')?;

    if start_str.is_empty() {
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
        let start: u64 = start_str.parse().ok()?;
        if start >= file_size {
            return None;
        }
        Some(ByteRange {
            start,
            end: file_size - 1,
        })
    } else {
        let start: u64 = start_str.parse().ok()?;
        let mut end: u64 = end_str.parse().ok()?;

        if start >= file_size || start > end {
            return None;
        }

        if end >= file_size {
            end = file_size - 1;
        }

        Some(ByteRange { start, end })
    }
}

/// RFC 5987 Content-Disposition with both ASCII and UTF-8 filename parameters.
fn content_disposition(filename: &str, inline: bool) -> String {
    let disposition_type = if inline { "inline" } else { "attachment" };

    let ascii_name: String = filename
        .chars()
        .map(|c| {
            if c.is_ascii() && !matches!(c, '"' | ';' | '\\' | ',') {
                c
            } else {
                '_'
            }
        })
        .collect();

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

async fn download(
    State(state): State<AppState>,
    Path(user_path): Path<String>,
    Query(query): Query<DownloadQuery>,
    headers: HeaderMap,
    Extension(_user): Extension<user_repo::User>,
) -> Result<Response<Body>, AppError> {
    let resolved = file_ops::safe_resolve(&state.canonical_root, &user_path)?;

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

    let etag = format!("\"{:x}-{:x}\"", file_size, modified.timestamp());

    if let Some(if_none_match) = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
    {
        if if_none_match == etag || if_none_match == "*" {
            return Response::builder()
                .status(StatusCode::NOT_MODIFIED)
                .header(header::ETAG, &etag)
                .body(Body::empty())
                .map_err(|e| AppError::Internal(e.to_string()));
        }
    }

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

    let mime = mime_guess::from_path(&resolved)
        .first_or_octet_stream()
        .to_string();

    let filename = resolved
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "download".into());

    let inline = query.inline.unwrap_or(false);
    let disposition = content_disposition(&filename, inline);

    let range_header = headers.get(header::RANGE).and_then(|v| v.to_str().ok());

    match range_header.and_then(|h| parse_range(h, file_size)) {
        Some(range) => {
            serve_partial(
                resolved,
                file_size,
                range,
                &mime,
                &disposition,
                &last_modified,
                &etag,
            )
            .await
        }
        None if range_header.is_some() => {
            let body = Body::empty();
            Response::builder()
                .status(StatusCode::RANGE_NOT_SATISFIABLE)
                .header(header::CONTENT_RANGE, format!("bytes */{file_size}"))
                .body(body)
                .map_err(|e| AppError::Internal(e.to_string()))
        }
        None => {
            serve_full(
                resolved,
                file_size,
                &mime,
                &disposition,
                &last_modified,
                &etag,
            )
            .await
        }
    }
}

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

    file.seek(std::io::SeekFrom::Start(range.start))
        .await
        .map_err(AppError::Io)?;

    let chunk_size = range.end - range.start + 1;

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

pub fn routes(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/{*path}", get(download))
        .route_layer(middleware::from_fn_with_state(state, require_auth))
}
