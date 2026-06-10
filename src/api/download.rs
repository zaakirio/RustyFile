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

pub(crate) fn content_disposition(filename: &str, inline: bool) -> String {
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
    stream_file_response(resolved, &headers, query.inline.unwrap_or(false)).await
}

/// Streams a regular file with ETag/Last-Modified conditionals, Range
/// support, Content-Disposition and download-hardening headers. Shared by the
/// authenticated download endpoint and public share downloads (which always
/// pass `inline_requested = false`).
pub(crate) async fn stream_file_response(
    resolved: std::path::PathBuf,
    headers: &HeaderMap,
    inline_requested: bool,
) -> Result<Response<Body>, AppError> {
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

    let etag = {
        let mut hasher = blake3::Hasher::new();
        hasher.update(&file_size.to_le_bytes());
        hasher.update(&modified.timestamp().to_le_bytes());
        format!("\"{}\"", &hasher.finalize().to_hex()[..32])
    };

    if let Some(if_none_match) = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
    {
        if if_none_match == etag || if_none_match == "*" {
            return Ok(Response::builder()
                .status(StatusCode::NOT_MODIFIED)
                .header(header::ETAG, &etag)
                .body(Body::empty())?);
        }
    }

    if let Some(ims) = headers
        .get(header::IF_MODIFIED_SINCE)
        .and_then(|v| v.to_str().ok())
    {
        if let Ok(ims_time) = chrono::DateTime::parse_from_rfc2822(ims) {
            if modified <= ims_time {
                return Ok(Response::builder()
                    .status(StatusCode::NOT_MODIFIED)
                    .header(header::ETAG, &etag)
                    .body(Body::empty())?);
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

    // HTML/SVG/XML execute scripts when rendered inline, so force a download
    // for those types regardless of ?inline=true.
    const FORCE_ATTACHMENT_MIMES: [&str; 5] = [
        "text/html",
        "application/xhtml+xml",
        "image/svg+xml",
        "text/xml",
        "application/xml",
    ];

    let inline = inline_requested && !FORCE_ATTACHMENT_MIMES.contains(&mime.as_str());
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
            Ok(Response::builder()
                .status(StatusCode::RANGE_NOT_SATISFIABLE)
                .header(header::CONTENT_RANGE, format!("bytes */{file_size}"))
                .body(body)?)
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

    Ok(Response::builder()
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
        .body(body)?)
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

    Ok(Response::builder()
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
        .body(body)?)
}

pub fn routes(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/{*path}", get(download))
        .route_layer(middleware::from_fn_with_state(state, require_auth))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normal_range() {
        let range = parse_range("bytes=0-499", 1000).unwrap();
        assert_eq!(range.start, 0);
        assert_eq!(range.end, 499);
    }

    #[test]
    fn open_ended_range() {
        let range = parse_range("bytes=500-", 1000).unwrap();
        assert_eq!(range.start, 500);
        assert_eq!(range.end, 999);
    }

    #[test]
    fn suffix_range() {
        let range = parse_range("bytes=-200", 1000).unwrap();
        assert_eq!(range.start, 800);
        assert_eq!(range.end, 999);
    }

    #[test]
    fn suffix_longer_than_file_rejected() {
        assert!(parse_range("bytes=-2000", 1000).is_none());
    }

    #[test]
    fn zero_suffix_rejected() {
        assert!(parse_range("bytes=-0", 1000).is_none());
    }

    #[test]
    fn end_clamped_to_file_size() {
        let range = parse_range("bytes=900-5000", 1000).unwrap();
        assert_eq!(range.start, 900);
        assert_eq!(range.end, 999);
    }

    #[test]
    fn start_past_eof_rejected() {
        assert!(parse_range("bytes=1000-", 1000).is_none());
        assert!(parse_range("bytes=1000-1500", 1000).is_none());
    }

    #[test]
    fn inverted_range_rejected() {
        assert!(parse_range("bytes=500-100", 1000).is_none());
    }

    #[test]
    fn invalid_inputs_rejected() {
        assert!(parse_range("items=0-499", 1000).is_none()); // wrong unit
        assert!(parse_range("bytes=abc-def", 1000).is_none()); // non-numeric
        assert!(parse_range("bytes=0-99,200-299", 1000).is_none()); // multipart
        assert!(parse_range("bytes=-", 1000).is_none()); // empty both sides
        assert!(parse_range("bytes=", 1000).is_none()); // no range at all
    }
}
