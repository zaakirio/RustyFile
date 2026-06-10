//! Anonymous share-link endpoints (no auth middleware).
//!
//! Security model:
//! - Tokens are 256-bit random values; knowing one grants access.
//! - Expired and missing tokens are indistinguishable (both 404) so the
//!   endpoint is not an oracle for expired links.
//! - Password-protected shares verify an `X-Share-Password` header with
//!   argon2 on every request. Because browsers cannot attach headers to a
//!   plain download navigation, `POST /{token}/verify` exchanges the
//!   password for a short-lived signed download token accepted as `?t=` on
//!   the download URL (the password itself never appears in a URL).
//! - All endpoints sit behind the per-IP rate limiter (password brute force
//!   is the main threat).
//!
//! Registered under the SLOW route group: downloads stream whole files or
//! zip entire directories, and drop uploads read large bodies.

use argon2::{Argon2, PasswordHash, PasswordVerifier};
use axum::body::Body;
use axum::extract::{Multipart, Path, Query, State};
use axum::http::{HeaderMap, Response, StatusCode};
use axum::middleware;
use axum::routing::{get, post};
use axum::{Json, Router};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;

use crate::db::share_repo;
use crate::error::AppError;
use crate::services::file_ops;
use crate::services::SearchIndex;
use crate::state::AppState;

/// Download tokens are single-purpose and short-lived: just long enough for
/// the browser to start the download after the password exchange.
const DOWNLOAD_TOKEN_TTL_SECS: u64 = 5 * 60;
/// Distinct audience so a share download token can never pass as a session
/// JWT (and vice versa) despite sharing the signing secret.
const DOWNLOAD_TOKEN_AUDIENCE: &str = "rustyfile-share-download";

/// Bound on the " (n)" suffix probe when avoiding overwrites.
const MAX_NAME_DEDUPE_ATTEMPTS: u32 = 10_000;

#[derive(Debug, Serialize, Deserialize)]
struct DownloadTokenClaims {
    /// The share token this download token is valid for.
    sub: String,
    exp: u64,
    iat: u64,
    iss: String,
    aud: String,
}

fn create_download_token(share_token: &str, secret: &[u8]) -> Result<String, AppError> {
    let now = chrono::Utc::now().timestamp() as u64;
    let claims = DownloadTokenClaims {
        sub: share_token.to_string(),
        exp: now + DOWNLOAD_TOKEN_TTL_SECS,
        iat: now,
        iss: "rustyfile".to_string(),
        aud: DOWNLOAD_TOKEN_AUDIENCE.to_string(),
    };

    encode(
        &Header::new(jsonwebtoken::Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(secret),
    )
    .map_err(|e| AppError::Internal(format!("Download token creation error: {e}")))
}

/// Returns the share token the download token was issued for.
fn validate_download_token(token: &str, secret: &[u8]) -> Result<String, AppError> {
    let mut validation = Validation::new(jsonwebtoken::Algorithm::HS256);
    validation.set_issuer(&["rustyfile"]);
    validation.set_audience(&[DOWNLOAD_TOKEN_AUDIENCE]);

    let data = decode::<DownloadTokenClaims>(token, &DecodingKey::from_secret(secret), &validation)
        .map_err(|_| AppError::Unauthorized("Invalid or expired download token".into()))?;

    Ok(data.claims.sub)
}

fn share_not_found() -> AppError {
    // Expired and missing tokens are deliberately identical.
    AppError::NotFound("Share not found".into())
}

/// Loads a live (existing, unexpired) share or 404s.
async fn load_share(state: &AppState, token: &str) -> Result<share_repo::Share, AppError> {
    share_repo::find_valid_by_token(&state.db, token)
        .await?
        .ok_or_else(share_not_found)
}

fn verify_password(share: &share_repo::Share, password: &str) -> Result<(), AppError> {
    let Some(hash) = &share.password_hash else {
        return Ok(());
    };

    let parsed = PasswordHash::new(hash)
        .map_err(|e| AppError::Internal(format!("Share password hash parse error: {e}")))?;

    Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .map_err(|_| AppError::Unauthorized("Invalid share password".into()))
}

fn password_header(state: &AppState, headers: &HeaderMap) -> Result<Option<String>, AppError> {
    let Some(value) = headers.get("x-share-password") else {
        return Ok(None);
    };
    let password = value
        .to_str()
        .map_err(|_| AppError::BadRequest("Invalid X-Share-Password header".into()))?;
    if password.len() > state.config.max_password_length {
        // Argon2 DoS guard, same as login.
        return Err(AppError::Unauthorized("Invalid share password".into()));
    }
    Ok(Some(password.to_string()))
}

/// Grants access to a protected share via either the password header or a
/// previously issued download token (`?t=`).
fn authorize(
    state: &AppState,
    share: &share_repo::Share,
    headers: &HeaderMap,
    download_token: Option<&str>,
) -> Result<(), AppError> {
    if !share.has_password() {
        return Ok(());
    }

    if let Some(password) = password_header(state, headers)? {
        return verify_password(share, &password);
    }

    if let Some(token) = download_token {
        let issued_for = validate_download_token(token, &state.jwt_secret)?;
        if issued_for == share.token {
            return Ok(());
        }
        return Err(AppError::Unauthorized(
            "Invalid or expired download token".into(),
        ));
    }

    Err(AppError::Unauthorized("Share password required".into()))
}

#[derive(Debug, Serialize)]
struct ShareMetadata {
    name: String,
    has_password: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    is_dir: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    size: Option<u64>,
}

/// `GET /api/public/shares/{token}` — share metadata.
///
/// Without a valid password (for protected shares) only `{name,
/// has_password}` is returned, so size/type never leak to someone who only
/// has the link.
async fn metadata(
    State(state): State<AppState>,
    Path(token): Path<String>,
    headers: HeaderMap,
) -> Result<Json<ShareMetadata>, AppError> {
    let share = load_share(&state, &token).await?;

    let resolved = file_ops::safe_resolve(&state.canonical_root, &share.path)?;
    let name = resolved
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "root".into());

    if share.has_password() {
        match password_header(&state, &headers)? {
            // Wrong password is a hard 401 so clients can distinguish it
            // from "no password given yet".
            Some(password) => verify_password(&share, &password)?,
            None => {
                return Ok(Json(ShareMetadata {
                    name,
                    has_password: true,
                    kind: None,
                    is_dir: None,
                    size: None,
                }))
            }
        }
    }

    let fs_meta = tokio::fs::metadata(&resolved)
        .await
        .map_err(|_| share_not_found())?;

    Ok(Json(ShareMetadata {
        name,
        has_password: share.has_password(),
        kind: Some(share.kind.clone()),
        is_dir: Some(fs_meta.is_dir()),
        size: if fs_meta.is_dir() {
            None
        } else {
            Some(fs_meta.len())
        },
    }))
}

#[derive(Debug, Deserialize)]
struct VerifyRequest {
    password: String,
}

#[derive(Debug, Serialize)]
struct VerifyResponse {
    download_token: String,
}

/// `POST /api/public/shares/{token}/verify` — exchanges the share password
/// for a short-lived signed download token, so the password never has to be
/// placed in a URL.
async fn verify(
    State(state): State<AppState>,
    Path(token): Path<String>,
    Json(body): Json<VerifyRequest>,
) -> Result<Json<VerifyResponse>, AppError> {
    let share = load_share(&state, &token).await?;

    if body.password.len() > state.config.max_password_length {
        return Err(AppError::Unauthorized("Invalid share password".into()));
    }
    verify_password(&share, &body.password)?;

    Ok(Json(VerifyResponse {
        download_token: create_download_token(&share.token, &state.jwt_secret)?,
    }))
}

#[derive(Debug, Deserialize)]
struct DownloadQuery {
    /// Short-lived download token from `/verify` (password-protected shares).
    t: Option<String>,
}

/// `GET /api/public/shares/{token}/download` — streams the shared file
/// (Range/ETag supported) or the shared directory as a ZIP.
async fn download(
    State(state): State<AppState>,
    Path(token): Path<String>,
    Query(query): Query<DownloadQuery>,
    headers: HeaderMap,
) -> Result<Response<Body>, AppError> {
    let share = load_share(&state, &token).await?;
    authorize(&state, &share, &headers, query.t.as_deref())?;

    let resolved = file_ops::safe_resolve(&state.canonical_root, &share.path)?;
    let fs_meta = tokio::fs::metadata(&resolved)
        .await
        .map_err(|_| share_not_found())?;

    // Best-effort counter: a failed UPDATE must not break the download.
    {
        let db = state.db.clone();
        let counted_token = share.token.clone();
        tokio::spawn(async move {
            if let Err(e) = share_repo::increment_download_count(&db, &counted_token).await {
                tracing::warn!("Failed to increment share download count: {e}");
            }
        });
    }

    if fs_meta.is_dir() {
        crate::api::archive::stream_zip_response(&state, vec![share.path.clone()]).await
    } else {
        crate::api::download::stream_file_response(resolved, &headers, false).await
    }
}

#[derive(Debug, Serialize)]
struct UploadedFile {
    name: String,
    size: u64,
}

#[derive(Debug, Serialize)]
struct UploadResponse {
    files: Vec<UploadedFile>,
}

/// Opens `dir/name`, appending " (1)", " (2)", … until a fresh file is
/// created — drop uploads never overwrite existing files (`create_new`
/// makes the check-and-create atomic).
async fn open_unique(
    dir: &std::path::Path,
    name: &str,
) -> Result<(tokio::fs::File, String), AppError> {
    let (stem, ext) = match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => (stem, format!(".{ext}")),
        _ => (name, String::new()),
    };

    for n in 0..MAX_NAME_DEDUPE_ATTEMPTS {
        let candidate = if n == 0 {
            name.to_string()
        } else {
            format!("{stem} ({n}){ext}")
        };

        match tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(dir.join(&candidate))
            .await
        {
            Ok(file) => return Ok((file, candidate)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(AppError::Io(e)),
        }
    }

    Err(AppError::Conflict(
        "Too many files with the same name in the drop directory".into(),
    ))
}

/// `POST /api/public/shares/{token}/upload` — anonymous multipart upload
/// into a drop share's directory.
async fn upload(
    State(state): State<AppState>,
    Path(token): Path<String>,
    Query(query): Query<DownloadQuery>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> Result<(StatusCode, Json<UploadResponse>), AppError> {
    let share = load_share(&state, &token).await?;

    if share.kind != "drop" {
        return Err(AppError::Forbidden(
            "This share does not accept uploads".into(),
        ));
    }

    authorize(&state, &share, &headers, query.t.as_deref())?;

    let dest_dir = file_ops::safe_resolve(&state.canonical_root, &share.path)?;
    let dir_meta = tokio::fs::metadata(&dest_dir)
        .await
        .map_err(|_| share_not_found())?;
    if !dir_meta.is_dir() {
        return Err(share_not_found());
    }

    let mut saved: Vec<UploadedFile> = Vec::new();

    while let Some(mut field) = multipart
        .next_field()
        .await
        .map_err(|e| AppError::BadRequest(format!("Invalid multipart body: {e}")))?
    {
        let Some(raw_name) = field.file_name().map(|s| s.to_string()) else {
            // Non-file fields are ignored.
            continue;
        };

        // Same component rules as every other upload path: traversal
        // attempts collapse to a bare filename, never a path.
        let filename = file_ops::sanitize_filename(&raw_name)?;
        file_ops::check_blocked_extension(&filename, &state.blocked_extensions)?;

        let (mut file, final_name) = open_unique(&dest_dir, &filename).await?;
        let mut size: u64 = 0;

        let write_result: Result<(), AppError> = async {
            while let Some(chunk) = field
                .chunk()
                .await
                .map_err(|e| AppError::BadRequest(format!("Failed to read upload body: {e}")))?
            {
                size += chunk.len() as u64;
                file.write_all(&chunk).await.map_err(AppError::Io)?;
            }
            file.flush().await.map_err(AppError::Io)?;
            file.sync_all().await.map_err(AppError::Io)?;
            Ok(())
        }
        .await;

        if let Err(err) = write_result {
            // Never leave a partial file behind.
            drop(file);
            let _ = tokio::fs::remove_file(dest_dir.join(&final_name)).await;
            return Err(err);
        }

        saved.push(UploadedFile {
            name: final_name,
            size,
        });
    }

    if saved.is_empty() {
        return Err(AppError::BadRequest("No files in upload".into()));
    }

    // Finalize like TUS/extract: invalidate the listing cache and index the
    // new files for search.
    state
        .dir_cache
        .invalidate(&dest_dir.to_string_lossy())
        .await;

    let indexer = state.search_indexer.clone();
    let rel_paths: Vec<String> = saved
        .iter()
        .map(|f| {
            if share.path.is_empty() {
                f.name.clone()
            } else {
                format!("{}/{}", share.path, f.name)
            }
        })
        .collect();
    tokio::spawn(async move {
        for rel_path in rel_paths {
            let _ = indexer.upsert(&rel_path).await;
        }
    });

    tracing::info!(
        share = %share.token,
        count = saved.len(),
        "Drop share upload completed"
    );

    Ok((StatusCode::CREATED, Json(UploadResponse { files: saved })))
}

pub fn routes(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/{token}", get(metadata))
        .route("/{token}/verify", post(verify))
        .route("/{token}/download", get(download))
        .route("/{token}/upload", post(upload))
        .route_layer(middleware::from_fn_with_state(
            state,
            crate::api::middleware::rate_limit::api_rate_limit,
        ))
}
