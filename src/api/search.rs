use axum::extract::{Query, State};
use axum::middleware;
use axum::routing::get;
use axum::{Json, Router};

use crate::api::middleware::auth::require_auth;
use crate::error::AppError;
use crate::services::search_index::{SearchIndex, SearchQuery, SearchResults};
use crate::state::AppState;

async fn search(
    State(state): State<AppState>,
    Query(query): Query<SearchQuery>,
) -> Result<Json<SearchResults>, AppError> {
    if query.q.is_empty() {
        return Err(AppError::BadRequest("Search query 'q' is required".into()));
    }
    let results = state.search_indexer.search(query).await?;
    Ok(Json(results))
}

pub fn routes(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/", get(search))
        .route_layer(middleware::from_fn_with_state(state, require_auth))
}
