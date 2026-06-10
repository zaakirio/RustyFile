use std::sync::{Arc, LazyLock};

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::middleware;
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, head, options, patch, post};
use axum::Router;
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use dashmap::DashMap;
use futures_util::StreamExt;
use rusqlite::params;
use tokio::io::AsyncWriteExt;
use tokio_util::sync::CancellationToken;

use crate::api::middleware::auth::require_auth;
use crate::error::AppError;
use crate::services::file_ops;
use crate::services::search_index::SearchIndex;
use crate::state::AppState;

const TUS_RESUMABLE: &str = "1.0.0";
const TUS_VERSION: &str = "1.0.0";
const TUS_EXTENSION: &str = "creation,termination";

/// Per-upload-id locks serializing the check-append-update section of
/// `append_chunk`, so concurrent PATCHes for the same upload cannot race
/// between the offset read, file append, and offset update. Entries are
/// removed on completion/cancel to avoid unbounded growth.
static UPLOAD_LOCKS: LazyLock<DashMap<String, Arc<tokio::sync::Mutex<()>>>> =
    LazyLock::new(DashMap::new);

fn upload_lock(upload_id: &str) -> Arc<tokio::sync::Mutex<()>> {
    UPLOAD_LOCKS
        .entry(upload_id.to_string())
        .or_default()
        .clone()
}

pub fn routes(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/", post(create_upload))
        .route("/", options(tus_options))
        .route("/{id}", head(query_offset))
        .route("/{id}", patch(append_chunk))
        .route("/{id}", delete(cancel_upload))
        .route_layer(middleware::from_fn_with_state(state, require_auth))
}

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

fn temp_path(cache_dir: &str, upload_id: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(cache_dir)
        .join("uploads")
        .join(upload_id)
}

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

async fn create_upload(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::Extension(user): axum::Extension<crate::db::user_repo::User>,
) -> Result<Response, AppError> {
    let total_bytes: i64 = headers
        .get("upload-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok())
        .ok_or_else(|| AppError::BadRequest("Missing or invalid Upload-Length header".into()))?;

    if total_bytes < 0 {
        return Err(AppError::BadRequest(
            "Upload-Length must be non-negative".into(),
        ));
    }

    let max_upload = state.config.max_upload_bytes as i64;
    if total_bytes > max_upload {
        return Err(AppError::BadRequest(format!(
            "Upload-Length {total_bytes} exceeds maximum allowed size {max_upload}"
        )));
    }

    let metadata_str = headers
        .get("upload-metadata")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    let mut metadata = parse_upload_metadata(metadata_str);

    let raw_filename = metadata
        .iter()
        .position(|(k, _)| k == "filename")
        .map(|i| metadata.swap_remove(i).1)
        .ok_or_else(|| AppError::BadRequest("Upload-Metadata must include 'filename'".into()))?;

    let filename = file_ops::sanitize_filename(&raw_filename)?;

    file_ops::check_blocked_extension(&filename, &state.blocked_extensions)?;

    let destination = metadata
        .iter()
        .position(|(k, _)| k == "destination")
        .map(|i| metadata.swap_remove(i).1)
        .unwrap_or_default();

    if !destination.is_empty() {
        file_ops::safe_resolve(&state.canonical_root, &destination)?;
    }

    let upload_id = uuid::Uuid::new_v4().to_string();
    let cache_dir = state.config.cache_dir.clone();
    let expiry_hours = state.config.tus_expiry_hours;

    let tmp = temp_path(&cache_dir, &upload_id);
    tokio::fs::create_dir_all(tmp.parent().unwrap())
        .await
        .map_err(AppError::Io)?;
    tokio::fs::File::create(&tmp).await.map_err(AppError::Io)?;

    let expires_at = chrono::Utc::now() + chrono::Duration::hours(expiry_hours as i64);
    let expires_str = expires_at.to_rfc3339();

    let uid = upload_id.clone();
    let user_id = user.id;
    let exp_str = expires_str.clone();

    crate::db::interact(&state.db, move |conn| {
        conn.execute(
            "INSERT INTO uploads (id, filename, destination, total_bytes, received_bytes, created_by, expires_at, completed)
             VALUES (?1, ?2, ?3, ?4, 0, ?5, ?6, 0)",
            params![uid, filename, destination, total_bytes, user_id, exp_str],
        )?;
        Ok(())
    })
    .await?;

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

async fn query_offset(
    State(state): State<AppState>,
    Path(upload_id): Path<String>,
) -> Result<Response, AppError> {
    let uid = upload_id.clone();

    let (received_bytes, total_bytes): (i64, i64) = crate::db::interact(&state.db, move |conn| {
        conn.query_row(
            "SELECT received_bytes, total_bytes FROM uploads WHERE id = ?1",
            params![uid],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
    })
    .await
    .map_err(|e| match e {
        AppError::Database(rusqlite::Error::QueryReturnedNoRows) => {
            AppError::UploadNotFound(upload_id)
        }
        other => other,
    })?;

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
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));

    Ok((StatusCode::OK, headers).into_response())
}

async fn append_chunk(
    State(state): State<AppState>,
    Path(upload_id): Path<String>,
    headers: HeaderMap,
    body: Body,
) -> Result<Response, AppError> {
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if content_type != "application/offset+octet-stream" {
        return Err(AppError::BadRequest(
            "Content-Type must be application/offset+octet-stream".into(),
        ));
    }

    let client_offset: i64 = headers
        .get("upload-offset")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok())
        .ok_or_else(|| AppError::BadRequest("Missing or invalid Upload-Offset header".into()))?;

    // Serialize the offset check, file append, and offset update per upload-id
    // so concurrent PATCHes cannot interleave and corrupt the temp file.
    let lock = upload_lock(&upload_id);
    let _guard = lock.lock().await;

    let uid = upload_id.clone();

    let (received_bytes, total_bytes, filename, destination): (i64, i64, String, String) =
        crate::db::interact(&state.db, move |conn| {
            conn.query_row(
                "SELECT received_bytes, total_bytes, filename, destination FROM uploads WHERE id = ?1 AND completed = 0",
                params![uid],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
        })
        .await
        .map_err(|e| match e {
            AppError::Database(rusqlite::Error::QueryReturnedNoRows) => {
                AppError::UploadNotFound(upload_id.clone())
            }
            other => other,
        })?;

    if client_offset != received_bytes {
        return Err(AppError::UploadConflict);
    }

    let cache_dir = state.config.cache_dir.clone();
    let tmp = temp_path(&cache_dir, &upload_id);

    // Stream the chunk to disk instead of buffering it in RAM. On any failure
    // (body error, IO error, or the chunk overflowing the declared
    // Upload-Length) truncate back to the recorded offset so the temp file
    // stays consistent with the DB state.
    let chunk_len = {
        let mut file = tokio::fs::OpenOptions::new()
            .append(true)
            .open(&tmp)
            .await
            .map_err(AppError::Io)?;

        let max_chunk = total_bytes - received_bytes;
        let mut written: i64 = 0;
        let mut stream = body.into_data_stream();
        let mut failure: Option<AppError> = None;

        while let Some(chunk) = stream.next().await {
            let bytes = match chunk {
                Ok(bytes) => bytes,
                Err(e) => {
                    failure = Some(AppError::BadRequest(format!(
                        "Failed to read request body: {e}"
                    )));
                    break;
                }
            };

            written += bytes.len() as i64;
            if written > max_chunk {
                failure = Some(AppError::BadRequest(format!(
                    "Chunk exceeds declared Upload-Length ({total_bytes})"
                )));
                break;
            }

            if let Err(e) = file.write_all(&bytes).await {
                failure = Some(AppError::Io(e));
                break;
            }
        }

        if failure.is_none() {
            if let Err(e) = file.flush().await {
                failure = Some(AppError::Io(e));
            }
        }

        if let Some(err) = failure {
            let _ = file.set_len(received_bytes as u64).await;
            let _ = file.sync_all().await;
            return Err(err);
        }

        file.sync_all().await.map_err(AppError::Io)?;
        written
    };

    let new_offset = received_bytes + chunk_len;

    let is_complete = new_offset >= total_bytes;

    {
        let uid = upload_id.clone();
        crate::db::interact(&state.db, move |conn| {
            conn.execute(
                "UPDATE uploads SET received_bytes = ?1, completed = ?2 WHERE id = ?3",
                params![new_offset, if is_complete { 1 } else { 0 }, uid],
            )?;
            Ok(())
        })
        .await?;
    }

    if is_complete {
        UPLOAD_LOCKS.remove(&upload_id);

        let dest_dir = if destination.is_empty() {
            (*state.canonical_root).clone()
        } else {
            file_ops::safe_resolve(&state.canonical_root, &destination)?
        };

        let final_path = dest_dir.join(&filename);

        if !final_path.starts_with(&*state.canonical_root) {
            let _ = tokio::fs::remove_file(&tmp).await;
            return Err(AppError::Forbidden(
                "Upload destination escapes root directory".into(),
            ));
        }

        if let Some(parent) = final_path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(AppError::Io)?;
        }

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

        let cache_key = dest_dir.to_string_lossy().to_string();
        state.dir_cache.invalidate(&cache_key).await;

        let indexer = state.search_indexer.clone();
        let idx_path = final_path
            .strip_prefix(&*state.canonical_root)
            .map_err(|_| AppError::BadRequest("resolved upload path escaped root".into()))?
            .iter()
            .map(|component| component.to_string_lossy())
            .collect::<Vec<_>>()
            .join("/");
        tokio::spawn(async move {
            let _ = indexer.upsert(&idx_path).await;
        });
    }

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

async fn cancel_upload(
    State(state): State<AppState>,
    Path(upload_id): Path<String>,
) -> Result<Response, AppError> {
    let uid = upload_id.clone();

    let rows_affected = crate::db::interact(&state.db, move |conn| {
        conn.execute("DELETE FROM uploads WHERE id = ?1", params![uid])
    })
    .await?;

    if rows_affected == 0 {
        return Err(AppError::UploadNotFound(upload_id));
    }

    UPLOAD_LOCKS.remove(&upload_id);

    let tmp = temp_path(&state.config.cache_dir, &upload_id);
    let _ = tokio::fs::remove_file(&tmp).await;

    let mut resp_headers = HeaderMap::new();
    resp_headers.insert(
        header::HeaderName::from_static("tus-resumable"),
        HeaderValue::from_static(TUS_RESUMABLE),
    );

    Ok((StatusCode::NO_CONTENT, resp_headers).into_response())
}

pub fn spawn_cleanup_task(
    db: deadpool_sqlite::Pool,
    cache_dir: String,
    shutdown: CancellationToken,
) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(5 * 60));
        loop {
            tokio::select! {
                _ = shutdown.cancelled() => {
                    tracing::info!("TUS cleanup task shutting down");
                    break;
                }
                _ = interval.tick() => {
                    if let Err(e) = cleanup_expired(&db, &cache_dir).await {
                        tracing::warn!("TUS cleanup error: {e}");
                    }
                }
            }
        }
    });
}

async fn cleanup_expired(db: &deadpool_sqlite::Pool, cache_dir: &str) -> Result<(), AppError> {
    let now = chrono::Utc::now().to_rfc3339();

    let expired_ids: Vec<String> = crate::db::interact(db, move |conn| {
        let mut stmt = conn.prepare(
            "SELECT id FROM uploads WHERE completed = 0 AND expires_at IS NOT NULL AND expires_at < ?1",
        )?;
        let ids = stmt
            .query_map(params![now], |row| row.get::<_, String>(0))?
            .filter_map(|r| r.ok())
            .collect();
        Ok(ids)
    })
    .await?;

    if expired_ids.is_empty() {
        return Ok(());
    }

    tracing::info!(count = expired_ids.len(), "Cleaning up expired TUS uploads");

    for id in &expired_ids {
        let tmp = temp_path(cache_dir, id);
        let _ = tokio::fs::remove_file(&tmp).await;
    }

    crate::db::interact(db, move |conn| {
        for id in &expired_ids {
            conn.execute("DELETE FROM uploads WHERE id = ?1", params![id])?;
        }
        Ok(())
    })
    .await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_valid_base64_pairs() {
        // "filename" -> "hello.txt", "destination" -> "docs/sub"
        let header = "filename aGVsbG8udHh0, destination ZG9jcy9zdWI=";
        let parsed = parse_upload_metadata(header);
        assert_eq!(
            parsed,
            vec![
                ("filename".to_string(), "hello.txt".to_string()),
                ("destination".to_string(), "docs/sub".to_string()),
            ]
        );
    }

    #[test]
    fn key_without_value_yields_empty_string() {
        let parsed = parse_upload_metadata("is_partial");
        assert_eq!(parsed, vec![("is_partial".to_string(), String::new())]);
    }

    #[test]
    fn malformed_base64_yields_empty_value() {
        let parsed = parse_upload_metadata("filename !!!not-base64!!!");
        assert_eq!(parsed, vec![("filename".to_string(), String::new())]);
    }

    #[test]
    fn non_utf8_payload_yields_empty_value() {
        // 0xFF 0xFE is not valid UTF-8.
        let header = format!("filename {}", STANDARD.encode([0xFF, 0xFE]));
        let parsed = parse_upload_metadata(&header);
        assert_eq!(parsed, vec![("filename".to_string(), String::new())]);
    }

    #[test]
    fn empty_header_yields_single_empty_key() {
        // Whole-header split keeps one empty segment; callers look keys up by
        // name, so an empty key is harmless.
        let parsed = parse_upload_metadata("");
        assert_eq!(parsed, vec![(String::new(), String::new())]);
    }
}
