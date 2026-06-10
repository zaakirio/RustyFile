use std::collections::HashSet;
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
use crate::services::watcher::FsEvent;

pub type HlsSources = moka::future::Cache<String, PathBuf>;

pub type IpRateLimiter = RateLimiter<String, DashMapStateStore<String>, DefaultClock>;

pub type TokenBlocklist = moka::future::Cache<String, ()>;

pub fn new_rate_limiter(max_requests: NonZeroU32, window_secs: u64) -> Arc<IpRateLimiter> {
    let period_ms = (window_secs * 1000) / max_requests.get() as u64;
    let period = Duration::from_millis(period_ms.max(1));

    let quota = Quota::with_period(period)
        .expect("Non-zero rate-limit period")
        .allow_burst(max_requests);

    Arc::new(RateLimiter::dashmap(quota))
}

/// Periodically evicts idle per-IP buckets from the keyed rate limiters.
///
/// Runs every 5 minutes. Without this, the key maps grow without bound as
/// new client IPs are seen. Respects the given cancellation token for
/// graceful shutdown.
pub async fn rate_limiter_maintenance(
    limiters: Vec<Arc<IpRateLimiter>>,
    token: tokio_util::sync::CancellationToken,
) {
    let interval_dur = Duration::from_secs(5 * 60);

    loop {
        tokio::select! {
            _ = tokio::time::sleep(interval_dur) => {}
            _ = token.cancelled() => {
                tracing::info!("Rate-limiter maintenance task shutting down");
                return;
            }
        }

        for limiter in &limiters {
            limiter.retain_recent();
        }
    }
}

#[derive(Clone)]
pub struct AppState {
    pub db: Pool,
    pub config: Arc<AppConfig>,
    pub setup_guard: Arc<SetupGuard>,
    pub jwt_secret: Arc<[u8]>,
    pub canonical_root: Arc<PathBuf>,
    pub login_limiter: Arc<IpRateLimiter>,
    /// Timing-attack-safe login failures.
    pub dummy_hash: Arc<str>,
    pub dir_cache: DirCache,
    pub thumb_worker: ThumbWorker,
    pub transcoder: HlsTranscoder,
    pub hls_sources: HlsSources,
    pub search_indexer: SearchIndexer,
    pub token_blocklist: TokenBlocklist,
    pub api_limiter: Arc<IpRateLimiter>,
    /// Pre-parsed set of blocked upload extensions (parsed once at startup).
    pub blocked_extensions: Arc<HashSet<String>>,
    /// Broadcasts filesystem change events to live (SSE) subscribers.
    pub fs_events: tokio::sync::broadcast::Sender<FsEvent>,
}

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
