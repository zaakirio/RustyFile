//! Server-Sent Events endpoint streaming live filesystem change events.

use std::convert::Infallible;
use std::time::Duration;

use axum::extract::{Extension, State};
use axum::middleware;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::routing::get;
use axum::Router;
use futures_util::stream::Stream;
use tokio::sync::broadcast::error::RecvError;

use crate::api::middleware::auth::require_auth;
use crate::db::user_repo;
use crate::state::AppState;

/// Keep-alive comment interval; keeps proxies and browsers from timing out
/// the stream during quiet periods.
const KEEP_ALIVE_SECS: u64 = 15;

/// `GET /api/events` — long-lived SSE stream of filesystem change events.
///
/// Each event's data field is a JSON-serialized [`crate::services::watcher::FsEvent`],
/// e.g. `{"type":"dir_changed","path":"some/dir"}`. Lagged receivers skip
/// missed events rather than erroring; the stream ends only when the
/// broadcast channel closes (shutdown).
async fn stream_events(
    State(state): State<AppState>,
    Extension(_user): Extension<user_repo::User>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let rx = state.fs_events.subscribe();

    let stream = futures_util::stream::unfold(rx, |mut rx| async move {
        loop {
            match rx.recv().await {
                Ok(fs_event) => match Event::default().json_data(&fs_event) {
                    Ok(event) => return Some((Ok(event), rx)),
                    Err(e) => {
                        tracing::error!("Failed to serialize fs event: {e}");
                        continue;
                    }
                },
                // Missed events are fine: the client refetches the listing
                // on the next event it does see.
                Err(RecvError::Lagged(skipped)) => {
                    tracing::debug!(skipped, "SSE subscriber lagged; skipping events");
                    continue;
                }
                Err(RecvError::Closed) => return None,
            }
        }
    });

    Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(KEEP_ALIVE_SECS)))
}

pub fn routes(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/", get(stream_events))
        .route_layer(middleware::from_fn_with_state(state, require_auth))
}
