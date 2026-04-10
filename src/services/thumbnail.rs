use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::sync::Semaphore;

const HASH_PREFIX_LEN: usize = 24;

#[async_trait::async_trait]
pub trait ThumbnailGenerator: Send + Sync {
    async fn get_or_generate(&self, source: &Path) -> Result<PathBuf, ThumbnailError>;
}

#[derive(Clone)]
pub struct ThumbWorker {
    semaphore: Arc<Semaphore>,
    cache_dir: Arc<PathBuf>,
    max_dimension: u32,
}

impl ThumbWorker {
    pub fn new(max_concurrent: usize, cache_dir: PathBuf, max_dimension: u32) -> Self {
        Self {
            semaphore: Arc::new(Semaphore::new(max_concurrent)),
            cache_dir: Arc::new(cache_dir),
            max_dimension,
        }
    }

    /// Returns the cache directory path (for cleanup tasks).
    pub fn cache_dir(&self) -> &Path {
        &self.cache_dir
    }

    async fn cache_key(&self, source: &Path) -> Result<String, ThumbnailError> {
        let meta = tokio::fs::metadata(source)
            .await
            .map_err(|_| ThumbnailError::SourceNotFound)?;

        let mtime = meta
            .modified()
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let mut hasher = blake3::Hasher::new();
        hasher.update(source.to_string_lossy().as_bytes());
        hasher.update(&meta.len().to_le_bytes());
        hasher.update(&mtime.to_le_bytes());

        Ok(hasher.finalize().to_hex()[..HASH_PREFIX_LEN].to_string())
    }
}

#[async_trait::async_trait]
impl ThumbnailGenerator for ThumbWorker {
    async fn get_or_generate(&self, source: &Path) -> Result<PathBuf, ThumbnailError> {
        let cache_key = self.cache_key(source).await?;
        let cached_path = self.cache_dir.join(format!("{cache_key}.jpg"));

        if cached_path.exists() {
            return Ok(cached_path);
        }

        let _permit = self
            .semaphore
            .acquire()
            .await
            .map_err(|_| ThumbnailError::Unavailable)?;

        // Double-check after acquiring permit
        if cached_path.exists() {
            return Ok(cached_path);
        }

        let source = source.to_path_buf();
        let max_dim = self.max_dimension;
        let out_path = cached_path.clone();

        tokio::task::spawn_blocking(move || generate_image_thumbnail(&source, &out_path, max_dim))
            .await
            .map_err(|_| ThumbnailError::GenerationFailed)??;

        Ok(cached_path)
    }
}

fn generate_image_thumbnail(
    source: &Path,
    output: &Path,
    max_dim: u32,
) -> Result<(), ThumbnailError> {
    let img = image::open(source).map_err(|_| ThumbnailError::GenerationFailed)?;
    let thumb = img.thumbnail(max_dim, max_dim);

    let mut buf = Cursor::new(Vec::new());
    thumb
        .write_to(&mut buf, image::ImageFormat::Jpeg)
        .map_err(|_| ThumbnailError::GenerationFailed)?;

    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    std::fs::write(output, buf.into_inner()).map_err(|_| ThumbnailError::GenerationFailed)?;

    Ok(())
}

/// Periodically cleans up stale thumbnail cache files.
///
/// Runs every 2 hours. Removes `.jpg` files in `thumb_dir` that are older than
/// 7 days. Respects the given cancellation token for graceful shutdown.
pub async fn cleanup_thumbnails(thumb_dir: PathBuf, token: tokio_util::sync::CancellationToken) {
    use std::time::{Duration, SystemTime};
    use tokio::fs;

    let interval_dur = Duration::from_secs(2 * 60 * 60);
    let max_age = Duration::from_secs(7 * 24 * 60 * 60);

    loop {
        tokio::select! {
            _ = tokio::time::sleep(interval_dur) => {}
            _ = token.cancelled() => {
                tracing::info!("Thumbnail cleanup task shutting down");
                return;
            }
        }

        tracing::debug!("Running thumbnail cleanup");
        let now = SystemTime::now();
        let mut removed = 0u32;

        let Ok(mut entries) = fs::read_dir(&thumb_dir).await else {
            continue;
        };

        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            let Ok(meta) = entry.metadata().await else {
                continue;
            };
            if !meta.is_file() {
                continue;
            }

            let modified = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            if now.duration_since(modified).unwrap_or_default() > max_age {
                if let Err(e) = fs::remove_file(&path).await {
                    tracing::warn!("Failed to remove stale thumbnail {}: {e}", path.display());
                } else {
                    removed += 1;
                }
            }
        }

        if removed > 0 {
            tracing::info!(removed, "Thumbnail cleanup complete");
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ThumbnailError {
    #[error("Source file not found")]
    SourceNotFound,
    #[error("Thumbnail generation failed")]
    GenerationFailed,
    #[error("Thumbnail service unavailable")]
    Unavailable,
}
