use axum::extract::State;
use axum::http::Request;
use axum::middleware::Next;
use axum::response::Response;

use crate::api::auth::{extract_token, validate_token};
use crate::db::user_repo;
use crate::error::AppError;
use crate::state::AppState;

pub async fn require_auth(
    State(state): State<AppState>,
    mut request: Request<axum::body::Body>,
    next: Next,
) -> Result<Response, AppError> {
    let token = extract_token(request.headers())?;
    let claims = validate_token(&token, &state.jwt_secret)?;

    let user = user_repo::find_by_id(&state.db, claims.sub)
        .await?
        .ok_or_else(|| AppError::Unauthorized("User not found".into()))?;

    request.extensions_mut().insert(claims);
    request.extensions_mut().insert(user);

    Ok(next.run(request).await)
}
