//! Authenticated share-link management: create, list and delete share links.
//!
//! Shares come in two kinds: `download` (anonymous recipients fetch a file
//! or a directory-as-ZIP) and `drop` (anonymous recipients upload into a
//! directory). The anonymous endpoints live in [`crate::api::public_shares`].

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::middleware;
use axum::routing::{delete, get};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use crate::api::middleware::auth::require_auth;
use crate::db::{share_repo, user_repo};
use crate::error::AppError;
use crate::services::file_ops;
use crate::state::AppState;

/// Expired shares 404 immediately (lazy check); this daily sweep just keeps
/// dead rows from accumulating.
const CLEANUP_INTERVAL_SECS: u64 = 24 * 60 * 60;

/// Sanity cap on share expiry (10 years), guarding the unix-seconds math.
const MAX_EXPIRES_IN_HOURS: u64 = 24 * 365 * 10;

#[derive(Debug, Deserialize)]
struct CreateShareRequest {
    path: String,
    kind: String,
    password: Option<String>,
    expires_in_hours: Option<u64>,
}

/// Public-safe view of a share row: the password hash is never serialized,
/// only a `has_password` flag.
#[derive(Debug, Serialize)]
pub(crate) struct ShareView {
    token: String,
    path: String,
    kind: String,
    has_password: bool,
    expires_at: Option<i64>,
    created_at: i64,
    download_count: i64,
    /// Whether the shared path currently exists on disk.
    exists: bool,
}

#[derive(Debug, Serialize)]
struct ShareListResponse {
    shares: Vec<ShareView>,
}

async fn share_view(state: &AppState, share: share_repo::Share) -> ShareView {
    let exists = match file_ops::safe_resolve(&state.canonical_root, &share.path) {
        Ok(resolved) => tokio::fs::symlink_metadata(&resolved).await.is_ok(),
        Err(_) => false,
    };

    ShareView {
        has_password: share.has_password(),
        token: share.token,
        path: share.path,
        kind: share.kind,
        expires_at: share.expires_at,
        created_at: share.created_at,
        download_count: share.download_count,
        exists,
    }
}

async fn create_share(
    State(state): State<AppState>,
    axum::Extension(_user): axum::Extension<user_repo::User>,
    Json(body): Json<CreateShareRequest>,
) -> Result<(StatusCode, Json<ShareView>), AppError> {
    if body.kind != "download" && body.kind != "drop" {
        return Err(AppError::BadRequest(
            "kind must be 'download' or 'drop'".into(),
        ));
    }

    let resolved = file_ops::safe_resolve(&state.canonical_root, &body.path)?;
    let metadata = tokio::fs::metadata(&resolved)
        .await
        .map_err(|_| AppError::NotFound("Path not found".into()))?;

    if body.kind == "drop" && !metadata.is_dir() {
        return Err(AppError::BadRequest(
            "Drop shares require an existing directory".into(),
        ));
    }

    // Store the canonical root-relative path (input may contain `.` segments
    // or redundant separators).
    let stored_path = resolved
        .strip_prefix(&*state.canonical_root)
        .map_err(|_| AppError::Forbidden("Path escapes root directory".into()))?
        .iter()
        .map(|c| c.to_string_lossy())
        .collect::<Vec<_>>()
        .join("/");

    let password_hash = match body.password.as_deref().filter(|p| !p.is_empty()) {
        Some(password) => {
            if password.len() > state.config.max_password_length {
                return Err(AppError::BadRequest(format!(
                    "Password exceeds maximum length of {} characters",
                    state.config.max_password_length
                )));
            }
            Some(user_repo::hash_password(password)?)
        }
        None => None,
    };

    let expires_at = match body.expires_in_hours {
        Some(0) => {
            return Err(AppError::BadRequest(
                "expires_in_hours must be at least 1".into(),
            ))
        }
        Some(hours) if hours > MAX_EXPIRES_IN_HOURS => {
            return Err(AppError::BadRequest(format!(
                "expires_in_hours must be at most {MAX_EXPIRES_IN_HOURS}"
            )))
        }
        Some(hours) => Some(chrono::Utc::now().timestamp() + (hours * 3600) as i64),
        None => None,
    };

    let share = share_repo::create(
        &state.db,
        &stored_path,
        &body.kind,
        password_hash,
        expires_at,
    )
    .await?;

    tracing::info!(path = %share.path, kind = %share.kind, "Share link created");

    Ok((StatusCode::CREATED, Json(share_view(&state, share).await)))
}

async fn list_shares(
    State(state): State<AppState>,
    axum::Extension(_user): axum::Extension<user_repo::User>,
) -> Result<Json<ShareListResponse>, AppError> {
    let rows = share_repo::list(&state.db).await?;

    let mut shares = Vec::with_capacity(rows.len());
    for share in rows {
        shares.push(share_view(&state, share).await);
    }

    Ok(Json(ShareListResponse { shares }))
}

async fn remove_share(
    State(state): State<AppState>,
    axum::Extension(_user): axum::Extension<user_repo::User>,
    Path(token): Path<String>,
) -> Result<StatusCode, AppError> {
    if share_repo::delete(&state.db, &token).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(AppError::NotFound("Share not found".into()))
    }
}

pub fn routes(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/", get(list_shares).post(create_share))
        .route("/{token}", delete(remove_share))
        .route_layer(middleware::from_fn_with_state(state, require_auth))
}

/// Daily sweep deleting expired share rows (lazy 404s already hide them).
pub fn spawn_cleanup_task(db: deadpool_sqlite::Pool, shutdown: CancellationToken) {
    tokio::spawn(async move {
        let mut interval =
            tokio::time::interval(std::time::Duration::from_secs(CLEANUP_INTERVAL_SECS));
        loop {
            tokio::select! {
                _ = shutdown.cancelled() => {
                    tracing::info!("Share cleanup task shutting down");
                    break;
                }
                _ = interval.tick() => {
                    match share_repo::delete_expired(&db).await {
                        Ok(0) => {}
                        Ok(count) => tracing::info!(count, "Removed expired share links"),
                        Err(e) => tracing::warn!("Share cleanup error: {e}"),
                    }
                }
            }
        }
    });
}
