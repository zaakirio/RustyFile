pub mod auth;
pub mod download;
pub mod files;
pub mod health;
pub mod hls;
pub mod middleware;
pub mod search;
pub mod setup;
pub mod thumbs;
pub mod tus;

use std::net::{IpAddr, SocketAddr};
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

const REQUEST_TIMEOUT_SECS: u64 = 30;

/// Parse comma-separated trusted proxy IPs into a list of IpAddr.
/// Returns None if the list is empty (meaning trust all).
fn parse_trusted_proxies(config_value: &str) -> Option<Vec<IpAddr>> {
    let trimmed = config_value.trim();
    if trimmed.is_empty() {
        return None;
    }
    let addrs: Vec<IpAddr> = trimmed
        .split(',')
        .filter_map(|s| s.trim().parse::<IpAddr>().ok())
        .collect();
    if addrs.is_empty() {
        None
    } else {
        Some(addrs)
    }
}

/// Extract the real client IP address.
///
/// When `trusted_proxies` is empty: trusts proxy headers unconditionally
/// (backwards compatible — assumes a reverse proxy strips spoofed headers).
///
/// When `trusted_proxies` is set: only reads X-Forwarded-For / X-Real-IP
/// if the direct peer address is in the trusted list.
pub(crate) fn extract_client_ip(
    headers: &axum::http::HeaderMap,
    peer_addr: Option<SocketAddr>,
    trusted_proxies: &str,
) -> String {
    let peer_ip = peer_addr.map(|a| a.ip());
    let trusted = parse_trusted_proxies(trusted_proxies);

    let should_trust_headers = match (&trusted, peer_ip) {
        (None, _) => true,
        (Some(list), Some(ip)) => list.contains(&ip),
        (Some(_), None) => false,
    };

    if should_trust_headers {
        if let Some(forwarded) = headers
            .get("x-forwarded-for")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.split(',').next())
            .map(|s| s.trim().to_string())
        {
            return forwarded;
        }
        if let Some(real_ip) = headers
            .get("x-real-ip")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.trim().to_string())
        {
            return real_ip;
        }
    }

    peer_ip
        .map(|ip| ip.to_string())
        .unwrap_or_else(|| "unknown".into())
}

pub fn build_router(state: AppState) -> Router {
    let max_upload = state.config.max_upload_bytes;

    let cached_api_routes = Router::new()
        .nest("/health", health::routes())
        .nest("/setup", setup::routes())
        .nest("/auth", auth::routes())
        .nest("/fs/search", search::routes(state.clone()))
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

    let thumb_routes = Router::new().nest("/thumbs", thumbs::routes(state.clone()));

    let hls_routes = Router::new().nest("/hls", hls::routes(state.clone()));

    let cors = build_cors_layer(&state.config.cors_origins);

    let trace_layer =
        TraceLayer::new_for_http().make_span_with(|request: &axum::http::Request<_>| {
            let client_ip = extract_client_ip(request.headers(), None, "");
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
        .layer(TimeoutLayer::new(Duration::from_secs(REQUEST_TIMEOUT_SECS)));

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
