use axum::extract::State;
use axum::routing::get;
use axum::Router;
use serde_json::json;

use crate::db;
use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new().route("/", get(health_check))
}

async fn health_check(
    State(state): State<AppState>,
) -> axum::Json<serde_json::Value> {
    let db_ok = db::interact(&state.db, |conn| {
        conn.execute_batch("SELECT 1")?;
        Ok(())
    })
    .await
    .is_ok();

    if db_ok {
        axum::Json(json!({ "status": "ok", "db": "connected" }))
    } else {
        tracing::error!("Health check: database unreachable");
        axum::Json(json!({ "status": "degraded", "db": "unreachable" }))
    }
}
