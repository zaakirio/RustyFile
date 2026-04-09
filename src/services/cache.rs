use std::sync::Arc;
use std::time::Duration;

use moka::future::Cache;

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

    pub async fn get_or_insert<F, Fut>(&self, key: String, f: F) -> Arc<DirListing>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Arc<DirListing>>,
    {
        self.inner.get_with(key, f()).await
    }

    pub async fn invalidate(&self, key: &str) {
        self.inner.invalidate(key).await;
    }

    pub(crate) fn invalidate_prefix(&self, prefix: &str) {
        let prefix = prefix.to_string();
        self.inner
            .invalidate_entries_if(move |key, _| key.starts_with(&prefix))
            .ok();
    }

    pub(crate) fn entry_count(&self) -> u64 {
        self.inner.entry_count()
    }
}
