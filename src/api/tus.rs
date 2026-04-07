use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::middleware;
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, head, options, patch, post};
use axum::Router;
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use rusqlite::params;
use std::io::Write;
use tokio_util::io::StreamReader;

use crate::api::middleware::auth::require_auth;
use crate::error::AppError;
use crate::services::file_ops;
use crate::state::AppState;

const TUS_RESUMABLE: &str = "1.0.0";
const TUS_VERSION: &str = "1.0.0";
const TUS_EXTENSION: &str = "creation,termination";

/// Build TUS protocol routes.
pub fn routes(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/", post(create_upload))
        .route("/", options(tus_options))
        .route("/{id}", head(query_offset))
        .route("/{id}", patch(append_chunk))
        .route("/{id}", delete(cancel_upload))
        .layer(middleware::from_fn_with_state(state, require_auth))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Parse the TUS Upload-Metadata header.
///
/// Format: `key1 base64value1,key2 base64value2`
fn parse_upload_metadata(header_value: &str) -> Vec<(String, String)> {
    header_value
        .split(',')
        .filter_map(|pair| {
            let pair = pair.trim();
            let mut parts = pair.splitn(2, ' ');
            let key = parts.next()?.trim().to_string();
            let b64_value = parts.next().unwrap_or("").trim();
            let value = if b64_value.is_empty() {
                String::new()
            } else {
                STANDARD
                    .decode(b64_value)
                    .ok()
                    .and_then(|bytes| String::from_utf8(bytes).ok())
                    .unwrap_or_default()
            };
            Some((key, value))
        })
        .collect()
}

/// Return the temp file path for a given upload id.
fn temp_path(cache_dir: &str, upload_id: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(cache_dir)
        .join("uploads")
        .join(upload_id)
}

/// Common TUS response headers.
fn tus_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::HeaderName::from_static("tus-resumable"),
        HeaderValue::from_static(TUS_RESUMABLE),
    );
    headers
}

// ---------------------------------------------------------------------------
// OPTIONS /api/tus/ -- TUS Discovery
// ---------------------------------------------------------------------------

async fn tus_options() -> impl IntoResponse {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::HeaderName::from_static("tus-resumable"),
        HeaderValue::from_static(TUS_RESUMABLE),
    );
    headers.insert(
        header::HeaderName::from_static("tus-version"),
        HeaderValue::from_static(TUS_VERSION),
    );
    headers.insert(
        header::HeaderName::from_static("tus-extension"),
        HeaderValue::from_static(TUS_EXTENSION),
    );
    (StatusCode::NO_CONTENT, headers)
}

// ---------------------------------------------------------------------------
// POST /api/tus/ -- Create Upload
// ---------------------------------------------------------------------------

async fn create_upload(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::Extension(user): axum::Extension<crate::db::user_repo::User>,
) -> Result<Response, AppError> {
    // Read Upload-Length header (required).
    let total_bytes: i64 = headers
        .get("upload-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok())
        .ok_or_else(|| AppError::BadRequest("Missing or invalid Upload-Length header".into()))?;

    if total_bytes < 0 {
        return Err(AppError::BadRequest("Upload-Length must be non-negative".into()));
    }

    // Parse Upload-Metadata to extract destination and filename.
    let metadata_str = headers
        .get("upload-metadata")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    let metadata = parse_upload_metadata(metadata_str);

    let filename = metadata
        .iter()
        .find(|(k, _)| k == "filename")
        .map(|(_, v)| v.clone())
        .ok_or_else(|| AppError::BadRequest("Upload-Metadata must include 'filename'".into()))?;

    let destination = metadata
        .iter()
        .find(|(k, _)| k == "destination")
        .map(|(_, v)| v.clone())
        .unwrap_or_default();

    // Validate destination path.
    if !destination.is_empty() {
        file_ops::safe_resolve(&state.canonical_root, &destination)?;
    }

    let upload_id = uuid::Uuid::new_v4().to_string();
    let cache_dir = state.config.cache_dir.clone();
    let expiry_hours = state.config.tus_expiry_hours;

    // Create temp file.
    let tmp = temp_path(&cache_dir, &upload_id);
    tokio::fs::create_dir_all(tmp.parent().unwrap())
        .await
        .map_err(AppError::Io)?;
    tokio::fs::File::create(&tmp)
        .await
        .map_err(AppError::Io)?;

    // Compute expiry.
    let expires_at = chrono::Utc::now()
        + chrono::Duration::hours(expiry_hours as i64);
    let expires_str = expires_at.to_rfc3339();

    // Insert into SQLite.
    let db = state.db.clone();
    let uid = upload_id.clone();
    let fname = filename.clone();
    let dest = destination.clone();
    let user_id = user.id;
    let exp_str = expires_str.clone();

    let conn = db
        .get()
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;

    conn.interact(move |conn| {
        conn.execute(
            "INSERT INTO uploads (id, filename, destination, total_bytes, received_bytes, created_by, expires_at, completed)
             VALUES (?1, ?2, ?3, ?4, 0, ?5, ?6, 0)",
            params![uid, fname, dest, total_bytes, user_id, exp_str],
        )?;
        Ok::<_, rusqlite::Error>(())
    })
    .await
    .map_err(|e| AppError::Internal(format!("interact error: {e}")))?
    .map_err(AppError::Database)?;

    // Build response.
    let location = format!("/api/tus/{upload_id}");
    let mut resp_headers = HeaderMap::new();
    resp_headers.insert(
        header::LOCATION,
        HeaderValue::from_str(&location)
            .map_err(|_| AppError::Internal("Invalid location header".into()))?,
    );
    resp_headers.insert(
        header::HeaderName::from_static("tus-resumable"),
        HeaderValue::from_static(TUS_RESUMABLE),
    );
    resp_headers.insert(
        header::HeaderName::from_static("upload-expires"),
        HeaderValue::from_str(&expires_str)
            .map_err(|_| AppError::Internal("Invalid expires header".into()))?,
    );

    Ok((StatusCode::CREATED, resp_headers).into_response())
}

// ---------------------------------------------------------------------------
// HEAD /api/tus/:id -- Query Offset
// ---------------------------------------------------------------------------

async fn query_offset(
    State(state): State<AppState>,
    Path(upload_id): Path<String>,
) -> Result<Response, AppError> {
    let db = state.db.clone();
    let uid = upload_id.clone();

    let conn = db
        .get()
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;

    let (received_bytes, total_bytes): (i64, i64) = conn
        .interact(move |conn| {
            conn.query_row(
                "SELECT received_bytes, total_bytes FROM uploads WHERE id = ?1",
                params![uid],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
        })
        .await
        .map_err(|e| AppError::Internal(format!("interact error: {e}")))?
        .map_err(|_| AppError::UploadNotFound(upload_id))?;

    let mut headers = HeaderMap::new();
    headers.insert(
        header::HeaderName::from_static("upload-offset"),
        HeaderValue::from_str(&received_bytes.to_string()).unwrap(),
    );
    headers.insert(
        header::HeaderName::from_static("upload-length"),
        HeaderValue::from_str(&total_bytes.to_string()).unwrap(),
    );
    headers.insert(
        header::HeaderName::from_static("tus-resumable"),
        HeaderValue::from_static(TUS_RESUMABLE),
    );
    // Prevent caching of offset queries.
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-store"),
    );

    Ok((StatusCode::OK, headers).into_response())
}

// ---------------------------------------------------------------------------
// PATCH /api/tus/:id -- Append Chunk
// ---------------------------------------------------------------------------

async fn append_chunk(
    State(state): State<AppState>,
    Path(upload_id): Path<String>,
    headers: HeaderMap,
    body: Body,
) -> Result<Response, AppError> {
    // Validate Content-Type.
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if content_type != "application/offset+octet-stream" {
        return Err(AppError::BadRequest(
            "Content-Type must be application/offset+octet-stream".into(),
        ));
    }

    // Read Upload-Offset from client.
    let client_offset: i64 = headers
        .get("upload-offset")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok())
        .ok_or_else(|| AppError::BadRequest("Missing or invalid Upload-Offset header".into()))?;

    // Get current server state.
    let db = state.db.clone();
    let uid = upload_id.clone();

    let conn = db
        .get()
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;

    let (received_bytes, total_bytes, filename, destination): (i64, i64, String, String) = conn
        .interact(move |conn| {
            conn.query_row(
                "SELECT received_bytes, total_bytes, filename, destination FROM uploads WHERE id = ?1 AND completed = 0",
                params![uid],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
        })
        .await
        .map_err(|e| AppError::Internal(format!("interact error: {e}")))?
        .map_err(|_| AppError::UploadNotFound(upload_id.clone()))?;

    // Validate offset matches.
    if client_offset != received_bytes {
        return Err(AppError::UploadConflict);
    }

    // Stream body to temp file.
    let cache_dir = state.config.cache_dir.clone();
    let tmp = temp_path(&cache_dir, &upload_id);

    // Read body into bytes, then append using spawn_blocking for fsync.
    use futures_util::TryStreamExt;
    let stream = body.into_data_stream();
    let byte_stream = stream.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e));
    let mut reader = StreamReader::new(byte_stream);

    // Read all chunks into a buffer.
    let mut buf = Vec::new();
    tokio::io::copy(&mut reader, &mut buf)
        .await
        .map_err(AppError::Io)?;

    let chunk_len = buf.len() as i64;
    let tmp_clone = tmp.clone();

    // Append to file with fsync on a blocking thread.
    tokio::task::spawn_blocking(move || -> Result<(), AppError> {
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&tmp_clone)
            .map_err(AppError::Io)?;
        file.write_all(&buf).map_err(AppError::Io)?;
        file.sync_all().map_err(AppError::Io)?;
        Ok(())
    })
    .await
    .map_err(|e| AppError::Internal(format!("spawn_blocking error: {e}")))??;

    let new_offset = received_bytes + chunk_len;

    // Update SQLite.
    let db = state.db.clone();
    let uid = upload_id.clone();
    let is_complete = new_offset >= total_bytes;

    let conn = db
        .get()
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;

    conn.interact(move |conn| {
        conn.execute(
            "UPDATE uploads SET received_bytes = ?1, completed = ?2 WHERE id = ?3",
            params![new_offset, if is_complete { 1 } else { 0 }, uid],
        )?;
        Ok::<_, rusqlite::Error>(())
    })
    .await
    .map_err(|e| AppError::Internal(format!("interact error: {e}")))?
    .map_err(AppError::Database)?;

    // If upload is complete, move to final destination.
    if is_complete {
        let dest_dir = if destination.is_empty() {
            state.canonical_root.clone()
        } else {
            file_ops::safe_resolve(&state.canonical_root, &destination)?
        };

        let final_path = dest_dir.join(&filename);

        // Ensure parent directory exists.
        if let Some(parent) = final_path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(AppError::Io)?;
        }

        // Atomic rename from temp to final path.
        tokio::fs::rename(&tmp, &final_path)
            .await
            .map_err(AppError::Io)?;

        tracing::info!(
            upload_id = %upload_id,
            filename = %filename,
            destination = %destination,
            bytes = total_bytes,
            "TUS upload completed"
        );

        // Invalidate directory cache for the destination.
        let cache_key = dest_dir.to_string_lossy().to_string();
        state.dir_cache.invalidate(&cache_key).await;
    }

    // Build response.
    let mut resp_headers = HeaderMap::new();
    resp_headers.insert(
        header::HeaderName::from_static("upload-offset"),
        HeaderValue::from_str(&new_offset.to_string()).unwrap(),
    );
    resp_headers.insert(
        header::HeaderName::from_static("tus-resumable"),
        HeaderValue::from_static(TUS_RESUMABLE),
    );

    Ok((StatusCode::NO_CONTENT, resp_headers).into_response())
}

// ---------------------------------------------------------------------------
// DELETE /api/tus/:id -- Cancel Upload
// ---------------------------------------------------------------------------

async fn cancel_upload(
    State(state): State<AppState>,
    Path(upload_id): Path<String>,
) -> Result<Response, AppError> {
    // Delete from DB.
    let db = state.db.clone();
    let uid = upload_id.clone();

    let conn = db
        .get()
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;

    let rows_affected = conn
        .interact(move |conn| {
            conn.execute("DELETE FROM uploads WHERE id = ?1", params![uid])
        })
        .await
        .map_err(|e| AppError::Internal(format!("interact error: {e}")))?
        .map_err(AppError::Database)?;

    if rows_affected == 0 {
        return Err(AppError::UploadNotFound(upload_id));
    }

    // Remove temp file (best effort).
    let tmp = temp_path(&state.config.cache_dir, &upload_id);
    let _ = tokio::fs::remove_file(&tmp).await;

    let mut resp_headers = HeaderMap::new();
    resp_headers.insert(
        header::HeaderName::from_static("tus-resumable"),
        HeaderValue::from_static(TUS_RESUMABLE),
    );

    Ok((StatusCode::NO_CONTENT, resp_headers).into_response())
}

// ---------------------------------------------------------------------------
// Background cleanup: remove expired incomplete uploads
// ---------------------------------------------------------------------------

/// Spawn a background task that periodically cleans up expired TUS uploads.
pub fn spawn_cleanup_task(db: deadpool_sqlite::Pool, cache_dir: String) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(5 * 60));
        loop {
            interval.tick().await;

            if let Err(e) = cleanup_expired(&db, &cache_dir).await {
                tracing::warn!("TUS cleanup error: {e}");
            }
        }
    });
}

async fn cleanup_expired(
    db: &deadpool_sqlite::Pool,
    cache_dir: &str,
) -> Result<(), AppError> {
    let conn = db
        .get()
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;

    let now = chrono::Utc::now().to_rfc3339();

    let expired_ids: Vec<String> = conn
        .interact(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT id FROM uploads WHERE completed = 0 AND expires_at IS NOT NULL AND expires_at < ?1",
            )?;
            let ids = stmt
                .query_map(params![now], |row| row.get::<_, String>(0))?
                .filter_map(|r| r.ok())
                .collect();
            Ok::<_, rusqlite::Error>(ids)
        })
        .await
        .map_err(|e| AppError::Internal(format!("interact error: {e}")))?
        .map_err(AppError::Database)?;

    if expired_ids.is_empty() {
        return Ok(());
    }

    tracing::info!(count = expired_ids.len(), "Cleaning up expired TUS uploads");

    for id in &expired_ids {
        // Remove temp file.
        let tmp = temp_path(cache_dir, id);
        let _ = tokio::fs::remove_file(&tmp).await;
    }

    // Delete from DB.
    let ids = expired_ids.clone();
    let conn = db
        .get()
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;

    conn.interact(move |conn| {
        for id in &ids {
            conn.execute("DELETE FROM uploads WHERE id = ?1", params![id])?;
        }
        Ok::<_, rusqlite::Error>(())
    })
    .await
    .map_err(|e| AppError::Internal(format!("interact error: {e}")))?
    .map_err(AppError::Database)?;

    Ok(())
}
