use axum::extract::{Extension, Path, State};
use axum::http::{header, Response, StatusCode};
use axum::middleware;
use axum::routing::get;
use axum::{body::Body, Router};
use tokio_util::io::ReaderStream;

use crate::api::middleware::auth::require_auth;
use crate::db::user_repo;
use crate::error::AppError;
use crate::services::file_ops;
use crate::state::AppState;

/// GET /api/hls/playlist/{*path}
///
/// Generate and return an HLS m3u8 playlist for the given video file.
/// Registers the source_key -> path mapping in state for segment lookups.
async fn playlist(
    State(state): State<AppState>,
    Path(user_path): Path<String>,
    Extension(_user): Extension<user_repo::User>,
) -> Result<Response<Body>, AppError> {
    let resolved = file_ops::safe_resolve(&state.canonical_root, &user_path)?;

    // Ensure target is a file.
    let meta = tokio::fs::metadata(&resolved)
        .await
        .map_err(|_| AppError::NotFound("File not found".into()))?;

    if meta.is_dir() {
        return Err(AppError::BadRequest("Cannot transcode a directory".into()));
    }

    let source_key = state
        .transcoder
        .source_key(&resolved)
        .map_err(|e| AppError::Internal(e.to_string()))?;

    // Register the mapping so segment requests can resolve the source path.
    state.hls_sources.insert(source_key.clone(), resolved.clone());

    let m3u8 = state
        .transcoder
        .playlist(&resolved, &source_key)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/vnd.apple.mpegurl")
        .header(header::CACHE_CONTROL, "no-cache")
        .body(Body::from(m3u8))
        .map_err(|e| AppError::Internal(e.to_string()))
}

/// GET /api/hls/segment/{source_key}/{index}
///
/// Serve a single .ts segment for a previously registered HLS source.
/// The `index` parameter may include a `.ts` suffix which is stripped.
async fn segment(
    State(state): State<AppState>,
    Path((source_key, index_raw)): Path<(String, String)>,
    Extension(_user): Extension<user_repo::User>,
) -> Result<Response<Body>, AppError> {
    // Strip optional .ts suffix from the index parameter.
    let index_str = index_raw.strip_suffix(".ts").unwrap_or(&index_raw);

    let segment_index: u32 = index_str
        .parse()
        .map_err(|_| AppError::BadRequest("Invalid segment index".into()))?;

    // Look up the source path from the registered mapping.
    let source_path = state
        .hls_sources
        .get(&source_key)
        .map(|entry| entry.value().clone())
        .ok_or_else(|| AppError::NotFound("Unknown HLS source".into()))?;

    let segment_path = state
        .transcoder
        .segment(&source_path, &source_key, segment_index)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;

    let file = tokio::fs::File::open(&segment_path)
        .await
        .map_err(AppError::Io)?;
    let meta = file.metadata().await.map_err(AppError::Io)?;
    let stream = ReaderStream::new(file);
    let body = Body::from_stream(stream);

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "video/mp2t")
        .header(header::CONTENT_LENGTH, meta.len())
        .header(header::CACHE_CONTROL, "public, max-age=86400, immutable")
        .body(body)
        .map_err(|e| AppError::Internal(e.to_string()))
}

pub fn routes(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/playlist/{*path}", get(playlist))
        .route("/segment/{source_key}/{index}", get(segment))
        .route_layer(middleware::from_fn_with_state(state, require_auth))
}
