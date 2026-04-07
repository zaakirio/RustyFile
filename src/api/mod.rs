pub mod auth;
pub mod download;
pub mod files;
pub mod health;
pub mod middleware;
pub mod setup;

use axum::http::{header, HeaderValue, Method};
use axum::response::Redirect;
use axum::routing::get;
use axum::Router;
use tower_http::compression::CompressionLayer;
use tower_http::cors::CorsLayer;
use tower_http::set_header::SetResponseHeaderLayer;
use tower_http::trace::TraceLayer;

use crate::state::AppState;

/// Build the complete application router with all routes and middleware layers.
pub fn build_router(state: AppState) -> Router {
    // API routes that should carry Cache-Control: no-cache (mutable data).
    let cached_api_routes = Router::new()
        .nest("/health", health::routes())
        .nest("/setup", setup::routes())
        .nest("/auth", auth::routes())
        .nest("/fs", files::routes(state.clone()))
        // Handle /fs/ with trailing slash (Axum nest doesn't match trailing slash)
        .route("/fs/", get(|| async { Redirect::permanent("/api/fs") }))
        // Fix 6: API responses are mutable data -- never cache.
        .layer(SetResponseHeaderLayer::overriding(
            header::CACHE_CONTROL,
            HeaderValue::from_static("no-cache, no-store, must-revalidate"),
        ));

    // Download routes set their own Cache-Control (private), so they are separate.
    let download_routes = Router::new()
        .nest("/fs/download", download::routes(state.clone()));

    // Fix 1: Restrictive CORS -- explicit methods and headers instead of permissive().
    let cors = CorsLayer::new()
        .allow_origin(tower_http::cors::Any)
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

    // Fix 11: Include client IP in access log spans.
    let trace_layer = TraceLayer::new_for_http().make_span_with(
        |request: &axum::http::Request<_>| {
            let client_ip = request
                .headers()
                .get("x-forwarded-for")
                .and_then(|v| v.to_str().ok())
                .or_else(|| {
                    request
                        .headers()
                        .get("x-real-ip")
                        .and_then(|v| v.to_str().ok())
                })
                .unwrap_or("unknown");
            tracing::info_span!(
                "request",
                method = %request.method(),
                uri = %request.uri(),
                client_ip = %client_ip,
            )
        },
    );

    Router::new()
        .nest("/api", download_routes)
        .nest("/api", cached_api_routes)
        .layer(trace_layer)
        .layer(CompressionLayer::new())
        .layer(cors)
        .with_state(state)
}
