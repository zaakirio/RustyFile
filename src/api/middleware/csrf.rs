use axum::extract::State;
use axum::http::{header, HeaderMap, Method, Request};
use axum::middleware::Next;
use axum::response::Response;

use crate::error::AppError;
use crate::state::AppState;

/// Browser CSRF defense-in-depth on top of `SameSite=Strict` cookies.
///
/// State-changing requests carrying an `Origin` header must be same-origin
/// (Origin authority matches the Host header) or match a configured CORS
/// origin. Requests without an `Origin` header (curl, native clients) pass
/// untouched — this guards browsers only.
pub async fn verify_origin(
    State(state): State<AppState>,
    request: Request<axum::body::Body>,
    next: Next,
) -> Result<Response, AppError> {
    let method = request.method();
    let is_state_changing = method == Method::POST
        || method == Method::PUT
        || method == Method::PATCH
        || method == Method::DELETE;

    if is_state_changing {
        if let Some(origin) = request
            .headers()
            .get(header::ORIGIN)
            .and_then(|v| v.to_str().ok())
        {
            if !origin_allowed(origin, request.headers(), &state.config.cors_origins) {
                return Err(AppError::Forbidden("Cross-origin request rejected".into()));
            }
        }
    }

    Ok(next.run(request).await)
}

fn origin_allowed(origin: &str, headers: &HeaderMap, cors_origins: &str) -> bool {
    let trimmed = cors_origins.trim();
    if trimmed == "*" {
        return true;
    }

    // Same-origin: the Origin's authority matches the Host header.
    let authority = origin
        .strip_prefix("https://")
        .or_else(|| origin.strip_prefix("http://"))
        .unwrap_or(origin);
    if let Some(host) = headers.get(header::HOST).and_then(|v| v.to_str().ok()) {
        if authority.eq_ignore_ascii_case(host.trim()) {
            return true;
        }
    }

    // Explicitly configured CORS origins.
    trimmed
        .split(',')
        .map(str::trim)
        .any(|allowed| !allowed.is_empty() && allowed.eq_ignore_ascii_case(origin))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn headers_with_host(host: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, HeaderValue::from_str(host).unwrap());
        headers
    }

    #[test]
    fn wildcard_allows_any_origin() {
        let headers = headers_with_host("example.com");
        assert!(origin_allowed("https://evil.example", &headers, "*"));
    }

    #[test]
    fn same_origin_allowed() {
        let headers = headers_with_host("example.com:8080");
        assert!(origin_allowed("http://example.com:8080", &headers, ""));
    }

    #[test]
    fn configured_origin_allowed() {
        let headers = headers_with_host("internal.host");
        assert!(origin_allowed(
            "https://app.example.com",
            &headers,
            "https://app.example.com, https://other.example.com"
        ));
    }

    #[test]
    fn cross_origin_rejected() {
        let headers = headers_with_host("example.com");
        assert!(!origin_allowed("https://evil.example", &headers, ""));
        assert!(!origin_allowed(
            "https://evil.example",
            &headers,
            "https://app.example.com"
        ));
    }
}
