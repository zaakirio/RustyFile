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

    #[error("Upload not found: {0}")]
    UploadNotFound(String),

    #[error("Upload offset mismatch")]
    UploadConflict,

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

impl From<anyhow::Error> for AppError {
    fn from(err: anyhow::Error) -> Self {
        Self::Internal(format!("{err:#}"))
    }
}

impl From<crate::services::transcoder::TranscodeError> for AppError {
    fn from(err: crate::services::transcoder::TranscodeError) -> Self {
        use crate::services::transcoder::TranscodeError;
        match err {
            TranscodeError::FfmpegNotFound => Self::Internal("ffmpeg not found".into()),
            TranscodeError::Unavailable => Self::Internal("transcoder unavailable".into()),
            TranscodeError::ProbeFailed | TranscodeError::TranscodeFailed => {
                Self::Internal(err.to_string())
            }
            TranscodeError::IoError => Self::Internal("transcode IO error".into()),
        }
    }
}

impl From<crate::services::thumbnail::ThumbnailError> for AppError {
    fn from(err: crate::services::thumbnail::ThumbnailError) -> Self {
        use crate::services::thumbnail::ThumbnailError;
        match err {
            ThumbnailError::SourceNotFound => Self::NotFound("source file not found".into()),
            ThumbnailError::Unavailable => Self::Internal("thumbnail service unavailable".into()),
            ThumbnailError::GenerationFailed => Self::Internal(err.to_string()),
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            Self::NotFound(msg) => (StatusCode::NOT_FOUND, msg.clone()),
            Self::Unauthorized(msg) => (StatusCode::UNAUTHORIZED, msg.clone()),
            Self::Forbidden(msg) => (StatusCode::FORBIDDEN, msg.clone()),
            Self::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg.clone()),
            Self::Conflict(msg) => (StatusCode::CONFLICT, msg.clone()),
            Self::UploadNotFound(msg) => (StatusCode::NOT_FOUND, msg.clone()),
            Self::UploadConflict => (StatusCode::CONFLICT, "Upload offset mismatch".into()),
            Self::SetupRequired => (StatusCode::PRECONDITION_REQUIRED, "Setup required".into()),
            Self::SetupExpired => (StatusCode::GONE, "Setup window expired".into()),
            Self::TooManyRequests(msg) => {
                tracing::warn!("Rate limited: {msg}");
                (StatusCode::TOO_MANY_REQUESTS, msg.clone())
            }
            Self::Internal(msg) => {
                tracing::error!("Internal error: {msg}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".into(),
                )
            }
            Self::Database(e) => {
                tracing::error!("Database error: {e}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".into(),
                )
            }
            Self::Pool(msg) => {
                tracing::error!("Pool error: {msg}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".into(),
                )
            }
            Self::Io(e) => {
                tracing::error!("IO error: {e}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".into(),
                )
            }
            Self::Json(e) => {
                tracing::error!("JSON error: {e}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".into(),
                )
            }
        };

        let body = json!({
            "error": message,
        });

        (status, axum::Json(body)).into_response()
    }
}
