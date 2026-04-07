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

/// Shared application state passed to all handlers via Axum's State extractor.
#[derive(Clone)]
pub struct AppState {
    pub db: Pool,
    pub config: AppConfig,
    pub setup_guard: Arc<SetupGuard>,
    pub jwt_secret: Vec<u8>,
    /// Pre-canonicalized root path, computed once at startup.
    pub canonical_root: PathBuf,
    /// In-memory login rate limiter keyed by IP address.
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

/// Simple sliding-window rate limiter for login attempts.
///
/// Tracks per-IP attempt counts with automatic expiry. Modelled after
/// the approach used in FileBrowser and Portainer for brute-force
/// protection.
pub struct LoginRateLimiter {
    /// Map of IP -> (attempt count, window start).
    attempts: DashMap<String, (AtomicU32, Instant)>,
    /// Maximum attempts allowed within the window.
    max_attempts: u32,
    /// Duration of the sliding window.
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

    /// Record an attempt and return `true` if the request should be allowed.
    pub fn check_rate_limit(&self, ip: &str) -> bool {
        let now = Instant::now();

        // Evict stale entries periodically (cheap check: only when map grows large).
        if self.attempts.len() > 10_000 {
            self.attempts.retain(|_, v| now.duration_since(v.1) < self.window);
        }

        let entry = self.attempts.entry(ip.to_string()).or_insert_with(|| {
            (AtomicU32::new(0), now)
        });

        let (count, window_start) = entry.value();

        // Reset if the window has expired.
        if now.duration_since(*window_start) >= self.window {
            count.store(1, Ordering::Relaxed);
            // We can't mutate window_start through the shared ref in DashMap easily,
            // so we drop and re-insert.
            drop(entry);
            self.attempts.insert(ip.to_string(), (AtomicU32::new(1), now));
            return true;
        }

        let current = count.fetch_add(1, Ordering::Relaxed) + 1;
        current <= self.max_attempts
    }

    /// Reset attempts for an IP after successful login.
    pub fn reset(&self, ip: &str) {
        self.attempts.remove(ip);
    }
}

/// Guards the initial setup window.
///
/// After the server starts without an admin account, there is a limited
/// time window during which the setup endpoint is available. Once an admin
/// is created (or the timeout elapses), the setup endpoint becomes unavailable.
pub struct SetupGuard {
    admin_created: AtomicBool,
    deadline: Instant,
}

impl SetupGuard {
    /// Create a new setup guard with the given timeout in minutes.
    pub fn new(timeout_minutes: u64) -> Self {
        Self {
            admin_created: AtomicBool::new(false),
            deadline: Instant::now() + Duration::from_secs(timeout_minutes * 60),
        }
    }

    /// Returns true if no admin has been created yet.
    pub fn is_setup_required(&self) -> bool {
        !self.admin_created.load(Ordering::Acquire)
    }

    /// Returns true if setup is still possible:
    /// no admin created AND the deadline has not passed.
    pub fn is_setup_allowed(&self) -> bool {
        !self.admin_created.load(Ordering::Acquire) && Instant::now() < self.deadline
    }

    /// Mark setup as complete (admin has been created).
    pub fn mark_complete(&self) {
        self.admin_created.store(true, Ordering::Release);
    }
}
