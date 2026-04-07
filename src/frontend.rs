use axum::http::{header, StatusCode, Uri};
use axum::response::{IntoResponse, Response};

#[cfg(feature = "embed-frontend")]
mod embedded {
    use super::*;
    use rust_embed::Embed;

    #[derive(Embed)]
    #[folder = "frontend/dist"]
    struct Assets;

    pub async fn static_handler(uri: Uri) -> Response {
        let path = uri.path().trim_start_matches('/');

        if path.is_empty() {
            return serve_index();
        }

        match Assets::get(path) {
            Some(file) => serve_file(path, file.data),
            None => {
                // Only treat the path as a missing static asset when the last
                // segment carries a file extension (e.g. `app.js`).  Paths
                // whose last segment has no extension—including those that
                // contain dots in earlier segments like `/browse/v1.2/detail`
                // —are SPA routes and should fall back to `index.html`.
                if std::path::Path::new(path).extension().is_some() {
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
                Vec::from(file.data),
            )
                .into_response(),
            None => StatusCode::NOT_FOUND.into_response(),
        }
    }

    fn serve_file(path: &str, data: std::borrow::Cow<'static, [u8]>) -> Response {
        let mime = mime_guess::from_path(path)
            .first_or_octet_stream()
            .to_string();

        // Vite hashed assets are immutable; everything else must revalidate.
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
            Vec::from(data),
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
