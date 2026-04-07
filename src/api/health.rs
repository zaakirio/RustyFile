use axum::routing::get;
use axum::Router;
use serde_json::json;

use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new().route("/", get(health_check))
}

async fn health_check() -> axum::Json<serde_json::Value> {
    axum::Json(json!({ "status": "ok" }))
}
