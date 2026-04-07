use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Unauthorized: {0}")]
    Unauthorized(String),

    #[error("Forbidden: {0}")]
    Forbidden(String),

    #[error("Bad request: {0}")]
    BadRequest(String),

    #[error("Conflict: {0}")]
    Conflict(String),

    #[error("Setup required")]
    SetupRequired,

    #[error("Setup expired")]
    SetupExpired,

    #[error("Too many requests: {0}")]
    TooManyRequests(String),

    #[error("Internal error: {0}")]
    Internal(String),

    #[error("Database error: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("Pool error: {0}")]
    Pool(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            AppError::NotFound(msg) => (StatusCode::NOT_FOUND, msg.clone()),
            AppError::Unauthorized(msg) => (StatusCode::UNAUTHORIZED, msg.clone()),
            AppError::Forbidden(msg) => (StatusCode::FORBIDDEN, msg.clone()),
            AppError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg.clone()),
            AppError::Conflict(msg) => (StatusCode::CONFLICT, msg.clone()),
            AppError::SetupRequired => {
                (StatusCode::PRECONDITION_REQUIRED, "Setup required".into())
            }
            AppError::SetupExpired => {
                (StatusCode::GONE, "Setup window expired".into())
            }
            AppError::TooManyRequests(msg) => {
                tracing::warn!("Rate limited: {msg}");
                (StatusCode::TOO_MANY_REQUESTS, msg.clone())
            }
            AppError::Internal(msg) => {
                tracing::error!("Internal error: {msg}");
                (StatusCode::INTERNAL_SERVER_ERROR, "Internal server error".into())
            }
            AppError::Database(e) => {
                tracing::error!("Database error: {e}");
                (StatusCode::INTERNAL_SERVER_ERROR, "Internal server error".into())
            }
            AppError::Pool(msg) => {
                tracing::error!("Pool error: {msg}");
                (StatusCode::INTERNAL_SERVER_ERROR, "Internal server error".into())
            }
            AppError::Io(e) => {
                tracing::error!("IO error: {e}");
                (StatusCode::INTERNAL_SERVER_ERROR, "Internal server error".into())
            }
            AppError::Json(e) => {
                tracing::error!("JSON error: {e}");
                (StatusCode::INTERNAL_SERVER_ERROR, "Internal server error".into())
            }
        };

        let body = json!({
            "error": message,
        });

        (status, axum::Json(body)).into_response()
    }
}
