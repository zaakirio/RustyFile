use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::middleware;
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, head, options, patch, post};
use axum::Router;
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use http_body_util::BodyExt;
use rusqlite::params;
use std::io::Write;

use crate::api::middleware::auth::require_auth;
use crate::error::AppError;
use crate::services::file_ops;
use crate::services::search_index::SearchIndex;
use crate::state::AppState;

const TUS_RESUMABLE: &str = "1.0.0";
const TUS_VERSION: &str = "1.0.0";
const TUS_EXTENSION: &str = "creation,termination";

pub fn routes(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/", post(create_upload))
        .route("/", options(tus_options))
        .route("/{id}", head(query_offset))
        .route("/{id}", patch(append_chunk))
        .route("/{id}", delete(cancel_upload))
        .layer(middleware::from_fn_with_state(state, require_auth))
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

    if client_offset != received_bytes {
        return Err(AppError::UploadConflict);
    }

    let body_bytes = body
        .collect()
        .await
        .map_err(|e| AppError::Internal(format!("Failed to read request body: {e}")))?
        .to_bytes();

    let chunk_len = body_bytes.len() as i64;

    let cache_dir = state.config.cache_dir.clone();
    let tmp = temp_path(&cache_dir, &upload_id);
    let tmp_clone = tmp.clone();
    let buf = body_bytes.to_vec();

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

    if is_complete {
        let dest_dir = if destination.is_empty() {
            state.canonical_root.clone()
        } else {
            file_ops::safe_resolve(&state.canonical_root, &destination)?
        };

        let final_path = dest_dir.join(&filename);

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
            .strip_prefix(&state.canonical_root)
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
    let db = state.db.clone();
    let uid = upload_id.clone();

    let conn = db
        .get()
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;

    let rows_affected = conn
        .interact(move |conn| conn.execute("DELETE FROM uploads WHERE id = ?1", params![uid]))
        .await
        .map_err(|e| AppError::Internal(format!("interact error: {e}")))?
        .map_err(AppError::Database)?;

    if rows_affected == 0 {
        return Err(AppError::UploadNotFound(upload_id));
    }

    let tmp = temp_path(&state.config.cache_dir, &upload_id);
    let _ = tokio::fs::remove_file(&tmp).await;

    let mut resp_headers = HeaderMap::new();
    resp_headers.insert(
        header::HeaderName::from_static("tus-resumable"),
        HeaderValue::from_static(TUS_RESUMABLE),
    );

    Ok((StatusCode::NO_CONTENT, resp_headers).into_response())
}

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

async fn cleanup_expired(db: &deadpool_sqlite::Pool, cache_dir: &str) -> Result<(), AppError> {
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
        let tmp = temp_path(cache_dir, id);
        let _ = tokio::fs::remove_file(&tmp).await;
    }

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
