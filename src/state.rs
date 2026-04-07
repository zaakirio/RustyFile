use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use deadpool_sqlite::Pool;

use crate::config::AppConfig;

/// Shared application state passed to all handlers via Axum's State extractor.
#[derive(Clone)]
pub struct AppState {
    pub db: Pool,
    pub config: AppConfig,
    pub setup_guard: Arc<SetupGuard>,
    pub jwt_secret: Vec<u8>,
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
        !self.admin_created.load(Ordering::SeqCst)
    }

    /// Returns true if setup is still possible:
    /// no admin created AND the deadline has not passed.
    pub fn is_setup_allowed(&self) -> bool {
        !self.admin_created.load(Ordering::SeqCst) && Instant::now() < self.deadline
    }

    /// Mark setup as complete (admin has been created).
    pub fn mark_complete(&self) {
        self.admin_created.store(true, Ordering::SeqCst);
    }
}
