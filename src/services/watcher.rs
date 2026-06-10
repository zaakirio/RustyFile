//! Filesystem watcher: invalidates the directory cache, keeps the search
//! index fresh, and publishes typed change events for SSE subscribers.

use std::collections::HashSet;

use notify::RecursiveMode;
use notify_debouncer_full::new_debouncer;
use serde::Serialize;
use tokio_util::sync::CancellationToken;

use crate::services::search_index::SearchIndex;
use crate::state::AppState;

/// Capacity of the broadcast channel feeding SSE subscribers. Slow or
/// disconnected receivers lag and are skipped; the watcher never blocks.
pub const FS_EVENTS_CAPACITY: usize = 256;

/// Debounce window for filesystem events.
const DEBOUNCE_MILLIS: u64 = 500;

/// Root-relative path as the frontend sees it (forward slashes, no leading
/// separator; empty string for the root itself).
fn event_path(rel: &std::path::Path) -> String {
    let s = rel.to_string_lossy();
    if cfg!(windows) {
        s.replace('\\', "/")
    } else {
        s.into_owned()
    }
}

/// A filesystem change event published to live (SSE) subscribers.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FsEvent {
    /// The contents of a directory changed. `path` is root-relative
    /// (empty string for the root itself).
    DirChanged { path: String },
}

/// Spawns the debounced filesystem watcher for the configured root.
///
/// For every batch of debounced events it invalidates affected directory
/// cache entries, upserts/removes search index rows, and broadcasts a
/// [`FsEvent::DirChanged`] per affected parent directory. Broadcast send
/// errors (no subscribers) are ignored. The task exits when `shutdown` is
/// cancelled.
pub fn spawn(state: &AppState, shutdown: CancellationToken) {
    let dir_cache = state.dir_cache.clone();
    let watch_root = state.canonical_root.clone();
    let search_indexer = state.search_indexer.clone();
    let fs_events = state.fs_events.clone();

    let (tx, mut rx) = tokio::sync::mpsc::channel(256);

    let mut debouncer = new_debouncer(
        std::time::Duration::from_millis(DEBOUNCE_MILLIS),
        None,
        move |result: notify_debouncer_full::DebounceEventResult| {
            let _ = tx.blocking_send(result);
        },
    )
    .expect("Failed to create filesystem watcher");

    debouncer
        .watch(&*watch_root, RecursiveMode::Recursive)
        .expect("Failed to watch root directory");

    tokio::spawn(async move {
        let _debouncer = debouncer; // Keep alive
        loop {
            tokio::select! {
                msg = rx.recv() => {
                    let Some(Ok(events)) = msg else { break };
                    // Root-relative parent dirs changed in this batch,
                    // deduplicated so bursts produce one event per dir.
                    let mut changed_dirs: HashSet<String> = HashSet::new();
                    for event in events {
                        for path in &event.paths {
                            if let Some(parent) = path.parent() {
                                let key = parent.to_string_lossy().to_string();
                                dir_cache.invalidate(&key).await;
                                if let Ok(rel_parent) = parent.strip_prefix(&*watch_root) {
                                    changed_dirs.insert(event_path(rel_parent));
                                }
                            }
                            if let Ok(rel) = path.strip_prefix(&*watch_root) {
                                let rel_str = rel.to_string_lossy().to_string();
                                if path.exists() {
                                    let _ = search_indexer.upsert(&rel_str).await;
                                } else {
                                    let _ = search_indexer.remove(&rel_str).await;
                                }
                            }
                        }
                    }
                    for dir in changed_dirs {
                        // Errors just mean no SSE subscribers are connected.
                        let _ = fs_events.send(FsEvent::DirChanged { path: dir });
                    }
                }
                _ = shutdown.cancelled() => {
                    tracing::info!("Filesystem watcher shutting down");
                    break;
                }
            }
        }
    });

    tracing::info!(
        "Filesystem watcher active for cache invalidation, search indexing, and live events"
    );
}
