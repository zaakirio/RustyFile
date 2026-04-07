use axum::http::{header, StatusCode, Uri};
use axum::response::{IntoResponse, Response};

#[cfg(feature = "embed-frontend")]
mod embedded {
    use super::*;
    use rust_embed::Embed;

    #[derive(Embed)]
    #[folder = "frontend/dist"]
    struct Assets;

    /// Serve embedded frontend assets with SPA catch-all.
    ///
    /// - Exact file match in `frontend/dist/` -> serve with Content-Type + Cache-Control
    /// - Path without file extension (SPA route) -> serve index.html
    /// - Path with extension but no match -> 404
    pub async fn static_handler(uri: Uri) -> Response {
        let path = uri.path().trim_start_matches('/');

        if path.is_empty() {
            return serve_index();
        }

        match Assets::get(path) {
            Some(file) => serve_file(path, &file.data),
            None => {
                // No dot = SPA route -> serve index.html for client-side routing
                // Has dot = actual missing asset -> 404
                if path.contains('.') {
                    StatusCode::NOT_FOUND.into_response()
                } else {
                    serve_index()
                }
            }
        }
    }

    fn serve_index() -> Response {
        match Assets::get("index.html") {
            Some(file) => (
                StatusCode::OK,
                [
                    (header::CONTENT_TYPE, "text/html; charset=utf-8"),
                    (header::CACHE_CONTROL, "no-cache"),
                ],
                file.data.to_vec(),
            )
                .into_response(),
            None => StatusCode::NOT_FOUND.into_response(),
        }
    }

    fn serve_file(path: &str, data: &[u8]) -> Response {
        let mime = mime_guess::from_path(path)
            .first_or_octet_stream()
            .to_string();

        // Vite hashed assets (assets/ dir) are immutable — cache aggressively.
        // Everything else (index.html, favicon, etc.) must revalidate.
        let cache_control = if path.starts_with("assets/") {
            "public, max-age=31536000, immutable"
        } else {
            "no-cache"
        };

        (
            StatusCode::OK,
            [
                (header::CONTENT_TYPE, mime.as_str()),
                (header::CACHE_CONTROL, cache_control),
            ],
            data.to_vec(),
        )
            .into_response()
    }
}

#[cfg(not(feature = "embed-frontend"))]
mod embedded {
    use super::*;

    pub async fn static_handler(_uri: Uri) -> Response {
        (
            StatusCode::NOT_FOUND,
            "Frontend not embedded. Build with: cd frontend && pnpm build && cd .. && cargo build",
        )
            .into_response()
    }
}

pub use embedded::static_handler;
