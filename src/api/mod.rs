pub mod auth;
pub mod download;
pub mod files;
pub mod health;
pub mod hls;
pub mod middleware;
pub mod setup;
pub mod thumbs;
pub mod tus;

use std::time::Duration;

use axum::extract::DefaultBodyLimit;
use axum::http::{header, HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Redirect};
use axum::routing::get;
use axum::Router;
use tower::timeout::TimeoutLayer;
use tower::ServiceBuilder;
use tower_http::compression::CompressionLayer;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::set_header::SetResponseHeaderLayer;
use tower_http::trace::TraceLayer;

use crate::state::AppState;

/// Extract client IP from proxy headers.
///
/// **Security note:** This trusts X-Forwarded-For unconditionally.
/// In production, ensure a reverse proxy (nginx, Traefik) strips/sets
/// this header from untrusted sources. Configure `RUSTYFILE_TRUSTED_PROXIES`
/// if direct client access is possible.
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

pub fn build_router(state: AppState) -> Router {
    let max_upload = state.config.max_upload_bytes;

    let cached_api_routes = Router::new()
        .nest("/health", health::routes())
        .nest("/setup", setup::routes())
        .nest("/auth", auth::routes())
        .nest("/fs", files::routes(state.clone()))
        // Axum nest doesn't match trailing slash.
        .route("/fs/", get(|| async { Redirect::permanent("/api/fs") }))
        .layer(SetResponseHeaderLayer::overriding(
            header::CACHE_CONTROL,
            HeaderValue::from_static("no-cache, no-store, must-revalidate"),
        ));

    // Download routes set their own Cache-Control, so they're layered separately.
    let download_routes = Router::new().nest("/fs/download", download::routes(state.clone()));

    let tus_routes = Router::new()
        .nest("/tus", tus::routes(state.clone()))
        .layer(DefaultBodyLimit::disable());

    let thumb_routes = Router::new()
        .nest("/thumbs", thumbs::routes(state.clone()));

    let hls_routes = Router::new()
        .nest("/hls", hls::routes(state.clone()));

    let cors = build_cors_layer(&state.config.cors_origins);

    let trace_layer =
        TraceLayer::new_for_http().make_span_with(|request: &axum::http::Request<_>| {
            let client_ip = extract_client_ip(request.headers());
            tracing::info_span!(
                "request",
                method = %request.method(),
                uri = %request.uri(),
                client_ip = %client_ip,
            )
        });

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

    let timeout_layer = ServiceBuilder::new()
        .layer(axum::error_handling::HandleErrorLayer::new(
            |_: tower::BoxError| async { StatusCode::REQUEST_TIMEOUT.into_response() },
        ))
        .layer(TimeoutLayer::new(Duration::from_secs(30)));

    let app = Router::new()
        .nest("/api", tus_routes)
        .nest("/api", download_routes)
        .nest("/api", thumb_routes)
        .nest("/api", hls_routes)
        .nest("/api", cached_api_routes)
        .fallback(crate::frontend::static_handler)
        .layer(trace_layer)
        .layer(CompressionLayer::new())
        .layer(cors)
        .layer(timeout_layer)
        .layer(DefaultBodyLimit::max(max_upload))
        .with_state(state);

    security_headers(app)
}

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
            header::HeaderName::from_static("upload-offset"),
            header::HeaderName::from_static("upload-length"),
            header::HeaderName::from_static("upload-metadata"),
            header::HeaderName::from_static("tus-resumable"),
        ])
        .expose_headers([
            header::HeaderName::from_static("upload-offset"),
            header::HeaderName::from_static("upload-length"),
            header::HeaderName::from_static("tus-resumable"),
            header::HeaderName::from_static("upload-expires"),
            header::LOCATION,
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
