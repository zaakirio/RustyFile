use axum::extract::State;
use axum::http::Request;
use axum::middleware::Next;
use axum::response::Response;
use std::net::SocketAddr;

use crate::error::AppError;
use crate::state::AppState;

/// Per-IP rate limiter for expensive API endpoints (search, thumbnails, HLS).
pub async fn api_rate_limit(
    State(state): State<AppState>,
    axum::extract::ConnectInfo(peer_addr): axum::extract::ConnectInfo<SocketAddr>,
    request: Request<axum::body::Body>,
    next: Next,
) -> Result<Response, AppError> {
    let client_ip = crate::api::extract_client_ip(
        request.headers(),
        Some(peer_addr),
        &state.config.trusted_proxies,
    );

    if state.api_limiter.check_key(&client_ip).is_err() {
        return Err(AppError::TooManyRequests(
            "Rate limit exceeded. Please slow down.".into(),
        ));
    }

    Ok(next.run(request).await)
}
