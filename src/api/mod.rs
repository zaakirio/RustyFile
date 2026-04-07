pub mod auth;
pub mod download;
pub mod files;
pub mod health;
pub mod middleware;
pub mod setup;

use axum::extract::DefaultBodyLimit;
use axum::http::{header, HeaderValue, Method};
use axum::response::Redirect;
use axum::routing::get;
use axum::Router;
use tower_http::compression::CompressionLayer;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::set_header::SetResponseHeaderLayer;
use tower_http::trace::TraceLayer;

use crate::state::AppState;

/// Extract client IP from proxy headers, with first hop from X-Forwarded-For.
pub fn extract_client_ip(headers: &axum::http::HeaderMap) -> String {
    headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .map(|s| s.trim().to_string())
        .or_else(|| {
            headers
                .get("x-real-ip")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string())
        })
        .unwrap_or_else(|| "unknown".into())
}

/// Build the complete application router with all routes and middleware layers.
pub fn build_router(state: AppState) -> Router {
    let max_upload = state.config.max_upload_bytes;

    // API routes that should carry Cache-Control: no-cache (mutable data).
    let cached_api_routes = Router::new()
        .nest("/health", health::routes())
        .nest("/setup", setup::routes())
        .nest("/auth", auth::routes())
        .nest("/fs", files::routes(state.clone()))
        // Handle /fs/ with trailing slash (Axum nest doesn't match trailing slash)
        .route("/fs/", get(|| async { Redirect::permanent("/api/fs") }))
        // API responses are mutable data -- never cache.
        .layer(SetResponseHeaderLayer::overriding(
            header::CACHE_CONTROL,
            HeaderValue::from_static("no-cache, no-store, must-revalidate"),
        ));

    // Download routes set their own Cache-Control (private), so they are separate.
    let download_routes = Router::new()
        .nest("/fs/download", download::routes(state.clone()));

    // Configurable CORS — defaults to Any but can be locked to specific origins.
    // Pattern from Portainer: restrict origins in production.
    let cors = build_cors_layer(&state.config.cors_origins);

    // Include client IP in access log spans.
    let trace_layer = TraceLayer::new_for_http().make_span_with(
        |request: &axum::http::Request<_>| {
            let client_ip = extract_client_ip(request.headers());
            tracing::info_span!(
                "request",
                method = %request.method(),
                uri = %request.uri(),
                client_ip = %client_ip,
            )
        },
    );

    // Security headers applied globally (pattern from Portainer's middleware).
    // Prevents clickjacking, MIME sniffing, and restricts referrer leakage.
    let security_headers = |r: Router| {
        r.layer(SetResponseHeaderLayer::overriding(
            header::HeaderName::from_static("x-frame-options"),
            HeaderValue::from_static("DENY"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::HeaderName::from_static("x-content-type-options"),
            HeaderValue::from_static("nosniff"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::HeaderName::from_static("x-xss-protection"),
            HeaderValue::from_static("1; mode=block"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::HeaderName::from_static("referrer-policy"),
            HeaderValue::from_static("strict-origin-when-cross-origin"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::HeaderName::from_static("permissions-policy"),
            HeaderValue::from_static("camera=(), microphone=(), geolocation=()"),
        ))
    };

    let app = Router::new()
        .nest("/api", download_routes)
        .nest("/api", cached_api_routes)
        .layer(trace_layer)
        .layer(CompressionLayer::new())
        .layer(cors)
        // Global body size limit — configurable, defaults to 50 MB.
        .layer(DefaultBodyLimit::max(max_upload))
        .with_state(state);

    security_headers(app)
}

/// Build CORS layer from a configuration string.
///
/// Supports "*" for any origin, or a comma-separated list of specific origins.
fn build_cors_layer(origins_config: &str) -> CorsLayer {
    let base = CorsLayer::new()
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::DELETE,
            Method::PATCH,
            Method::HEAD,
            Method::OPTIONS,
        ])
        .allow_headers([
            header::AUTHORIZATION,
            header::CONTENT_TYPE,
            header::RANGE,
            header::ACCEPT,
        ]);

    let trimmed = origins_config.trim();
    if trimmed == "*" || trimmed.is_empty() {
        base.allow_origin(tower_http::cors::Any)
    } else {
        let origins: Vec<HeaderValue> = trimmed
            .split(',')
            .filter_map(|s| {
                let s = s.trim();
                if s.is_empty() {
                    None
                } else {
                    HeaderValue::from_str(s).ok()
                }
            })
            .collect();
        base.allow_origin(AllowOrigin::list(origins))
    }
}
