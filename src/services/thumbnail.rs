use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::sync::Semaphore;

/// Length of the blake3 hex hash prefix used for cache keys.
const HASH_PREFIX_LEN: usize = 24;

// ---------------------------------------------------------------------------
// Trait
// ---------------------------------------------------------------------------

/// Abstraction over thumbnail generation for testability.
///
/// The canonical implementation is [`ThumbWorker`]. In tests you can supply a
/// mock or stub via `Box<dyn ThumbnailGenerator>`.
#[async_trait::async_trait]
pub trait ThumbnailGenerator: Send + Sync {
    /// Return path to a cached thumbnail, generating one if needed.
    async fn get_or_generate(&self, source: &Path) -> Result<PathBuf, ThumbnailError>;
}

// ---------------------------------------------------------------------------
// Concrete implementation
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct ThumbWorker {
    semaphore: Arc<Semaphore>,
    cache_dir: PathBuf,
    max_dimension: u32,
}

impl ThumbWorker {
    pub fn new(max_concurrent: usize, cache_dir: PathBuf, max_dimension: u32) -> Self {
        Self {
            semaphore: Arc::new(Semaphore::new(max_concurrent)),
            cache_dir,
            max_dimension,
        }
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

#[derive(Debug, thiserror::Error)]
pub enum ThumbnailError {
    #[error("Source file not found")]
    SourceNotFound,
    #[error("Thumbnail generation failed")]
    GenerationFailed,
    #[error("Thumbnail service unavailable")]
    Unavailable,
}
