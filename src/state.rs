use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use dashmap::DashMap;
use deadpool_sqlite::Pool;

use crate::config::AppConfig;
use crate::services::cache::DirCache;
use crate::services::thumbnail::ThumbWorker;
use crate::services::transcoder::HlsTranscoder;

#[derive(Clone)]
pub struct AppState {
    pub db: Pool,
    pub config: AppConfig,
    pub setup_guard: Arc<SetupGuard>,
    pub jwt_secret: Vec<u8>,
    pub canonical_root: PathBuf,
    pub login_limiter: Arc<LoginRateLimiter>,
    /// In-memory directory listing cache (moka).
    pub dir_cache: DirCache,
    /// Semaphore-limited image thumbnail worker with disk cache.
    pub thumb_worker: ThumbWorker,
    /// On-demand HLS video transcoder backed by FFmpeg.
    pub transcoder: HlsTranscoder,
    /// Maps HLS source keys to their resolved filesystem paths.
    pub hls_sources: Arc<DashMap<String, PathBuf>>,
}

pub struct LoginRateLimiter {
    attempts: DashMap<String, (AtomicU32, Instant)>,
    max_attempts: u32,
    window: Duration,
}

impl LoginRateLimiter {
    pub fn new(max_attempts: u32, window: Duration) -> Self {
        Self {
            attempts: DashMap::new(),
            max_attempts,
            window,
        }
    }

    pub fn check_rate_limit(&self, ip: &str) -> bool {
        let now = Instant::now();

        // Evict stale entries only when map grows large.
        if self.attempts.len() > 10_000 {
            self.attempts.retain(|_, v| now.duration_since(v.1) < self.window);
        }

        let entry = self.attempts.entry(ip.to_string()).or_insert_with(|| {
            (AtomicU32::new(0), now)
        });

        let (count, window_start) = entry.value();

        if now.duration_since(*window_start) >= self.window {
            count.store(1, Ordering::Relaxed);
            // Can't mutate window_start through DashMap shared ref; re-insert instead.
            drop(entry);
            self.attempts.insert(ip.to_string(), (AtomicU32::new(1), now));
            return true;
        }

        let current = count.fetch_add(1, Ordering::Relaxed) + 1;
        current <= self.max_attempts
    }

    pub fn reset(&self, ip: &str) {
        self.attempts.remove(ip);
    }
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
