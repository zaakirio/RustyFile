use std::sync::Arc;
use std::time::Duration;

use moka::future::Cache;

use crate::error::AppError;
use crate::services::file_ops::DirListing;

#[derive(Clone)]
pub struct DirCache {
    inner: Cache<String, Arc<DirListing>>,
}

impl DirCache {
    pub fn new(max_entries: u64, ttl_secs: u64) -> Self {
        let inner = Cache::builder()
            .max_capacity(max_entries)
            .time_to_live(Duration::from_secs(ttl_secs))
            .time_to_idle(Duration::from_secs(ttl_secs / 2))
            .eviction_listener(|key, _value, cause| {
                tracing::debug!(key = %key, cause = ?cause, "dir cache eviction");
            })
            .build();
        Self { inner }
    }

    /// Returns the cached listing or runs `f` to produce one. Only successful
    /// listings are cached; errors propagate to the caller uncached.
    pub async fn get_or_insert<F, Fut>(
        &self,
        key: String,
        f: F,
    ) -> Result<Arc<DirListing>, AppError>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<Arc<DirListing>, AppError>>,
    {
        self.inner.try_get_with(key, f()).await.map_err(Into::into)
    }

    pub async fn invalidate(&self, key: &str) {
        self.inner.invalidate(key).await;
    }
}
