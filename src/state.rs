use std::num::NonZeroU32;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use deadpool_sqlite::Pool;
use governor::clock::DefaultClock;
use governor::state::keyed::DashMapStateStore;
use governor::{Quota, RateLimiter};

use crate::config::AppConfig;
use crate::services::cache::DirCache;
use crate::services::search_index::SearchIndexer;
use crate::services::thumbnail::ThumbWorker;
use crate::services::transcoder::HlsTranscoder;

pub type LoginRateLimiter = RateLimiter<String, DashMapStateStore<String>, DefaultClock>;

/// Create a keyed rate limiter that allows `max_attempts` within `window_secs`.
///
/// Taking `NonZeroU32` for `max_attempts` makes zero an unrepresentable value,
/// preventing a panic or divide-by-zero at runtime.
pub fn new_login_limiter(max_attempts: NonZeroU32, window_secs: u64) -> Arc<LoginRateLimiter> {
    // Use milliseconds to avoid integer division truncating to zero when
    // window_secs < max_attempts (e.g. 60s / 100 attempts = 0s).
    let period_ms = (window_secs * 1000) / max_attempts.get() as u64;
    let period = Duration::from_millis(period_ms.max(1));

    let quota = Quota::with_period(period)
        .expect("Non-zero rate-limit period")
        .allow_burst(max_attempts);

    Arc::new(RateLimiter::dashmap(quota))
}

#[derive(Clone)]
pub struct AppState {
    pub db: Pool,
    pub config: AppConfig,
    pub setup_guard: Arc<SetupGuard>,
    pub jwt_secret: Vec<u8>,
    pub canonical_root: PathBuf,
    pub login_limiter: Arc<LoginRateLimiter>,
    /// Pre-hashed dummy password for timing-attack-safe login failures.
    pub dummy_hash: String,
    /// In-memory directory listing cache (moka).
    pub dir_cache: DirCache,
    /// Semaphore-limited image thumbnail worker with disk cache.
    pub thumb_worker: ThumbWorker,
    /// On-demand HLS video transcoder backed by FFmpeg.
    pub transcoder: HlsTranscoder,
    /// Maps HLS source keys to their resolved filesystem paths.
    pub hls_sources: Arc<dashmap::DashMap<String, PathBuf>>,
    /// Full-text search index backed by SQLite.
    pub search_indexer: SearchIndexer,
}

/// Time-limited window for initial admin creation. Closes on admin creation
/// or timeout, whichever comes first.
pub struct SetupGuard {
    admin_created: AtomicBool,
    deadline: Instant,
}

impl SetupGuard {
    pub fn new(timeout_minutes: u64) -> Self {
        Self {
            admin_created: AtomicBool::new(false),
            deadline: Instant::now() + Duration::from_secs(timeout_minutes * 60),
        }
    }

    pub fn is_setup_required(&self) -> bool {
        !self.admin_created.load(Ordering::Acquire)
    }

    pub fn is_setup_allowed(&self) -> bool {
        !self.admin_created.load(Ordering::Acquire) && Instant::now() < self.deadline
    }

    pub fn mark_complete(&self) {
        self.admin_created.store(true, Ordering::Release);
    }
}
