pub mod auth;
pub mod download;
pub mod files;
pub mod health;
pub mod middleware;
pub mod setup;

use axum::response::Redirect;
use axum::routing::get;
use axum::Router;
use tower_http::compression::CompressionLayer;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

use crate::state::AppState;

/// Build the complete application router with all routes and middleware layers.
pub fn build_router(state: AppState) -> Router {
    let api_routes = Router::new()
        .nest("/health", health::routes())
        .nest("/setup", setup::routes())
        .nest("/auth", auth::routes())
        .nest("/fs/download", download::routes(state.clone()))
        .nest("/fs", files::routes(state.clone()))
        // Handle /fs/ with trailing slash (Axum nest doesn't match trailing slash)
        .route("/fs/", get(|| async { Redirect::permanent("/api/fs") }));

    Router::new()
        .nest("/api", api_routes)
        .layer(TraceLayer::new_for_http())
        .layer(CompressionLayer::new())
        .layer(CorsLayer::permissive())
        .with_state(state)
}
