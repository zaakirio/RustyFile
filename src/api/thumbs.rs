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
use crate::services::thumbnail::ThumbnailGenerator;
use crate::state::AppState;

async fn thumbnail(
    State(state): State<AppState>,
    Path(user_path): Path<String>,
    Extension(_user): Extension<user_repo::User>,
) -> Result<Response<Body>, AppError> {
    let resolved = file_ops::safe_resolve(&state.canonical_root, &user_path)?;

    let thumb_path = state
        .thumb_worker
        .get_or_generate(&resolved)
        .await?;

    let file = tokio::fs::File::open(&thumb_path)
        .await
        .map_err(AppError::Io)?;
    let meta = file.metadata().await.map_err(AppError::Io)?;
    let stream = ReaderStream::new(file);
    let body = Body::from_stream(stream);

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "image/jpeg")
        .header(header::CONTENT_LENGTH, meta.len())
        .header(header::CACHE_CONTROL, "public, max-age=86400, immutable")
        .body(body)
        .map_err(|e| AppError::Internal(e.to_string()))
}

pub fn routes(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/{*path}", get(thumbnail))
        .route_layer(middleware::from_fn_with_state(state, require_auth))
}
