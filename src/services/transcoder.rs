use std::fmt::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::sync::Semaphore;

const HASH_PREFIX_LEN: usize = 24;

#[async_trait::async_trait]
pub trait VideoTranscoder: Send + Sync {
    fn source_key(&self, source: &Path) -> Result<String, TranscodeError>;
    async fn playlist(&self, source: &Path, source_key: &str) -> Result<String, TranscodeError>;
    async fn segment(
        &self,
        source: &Path,
        source_key: &str,
        segment_index: u32,
    ) -> Result<PathBuf, TranscodeError>;
}
#[derive(Clone)]
pub struct HlsTranscoder {
    segment_dir: Arc<PathBuf>,
    semaphore: Arc<Semaphore>,
    segment_duration: u32,
}

impl HlsTranscoder {
    pub fn new(segment_dir: PathBuf, max_concurrent: usize, segment_duration: u32) -> Self {
        Self {
            segment_dir: Arc::new(segment_dir),
            semaphore: Arc::new(Semaphore::new(max_concurrent)),
            segment_duration,
        }
    }

    /// Returns the segment directory path (for cleanup tasks).
    pub fn segment_dir(&self) -> &Path {
        &self.segment_dir
    }

    pub async fn probe_duration(&self, source: &Path) -> Result<f64, TranscodeError> {
        let output = tokio::process::Command::new("ffprobe")
            .args([
                "-v",
                "error",
                "-show_entries",
                "format=duration",
                "-of",
                "default=noprint_wrappers=1:nokey=1",
            ])
            .arg(source)
            .output()
            .await
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    TranscodeError::FfmpegNotFound
                } else {
                    TranscodeError::IoError
                }
            })?;

        if !output.status.success() {
            tracing::error!(
                "ffprobe failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            return Err(TranscodeError::ProbeFailed);
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let duration: f64 = stdout
            .trim()
            .parse()
            .map_err(|_| TranscodeError::ProbeFailed)?;

        Ok(duration)
    }
}

#[async_trait::async_trait]
impl VideoTranscoder for HlsTranscoder {
    fn source_key(&self, source: &Path) -> Result<String, TranscodeError> {
        let meta = std::fs::metadata(source).map_err(|_| TranscodeError::IoError)?;

        let mtime = meta
            .modified()
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let mut hasher = blake3::Hasher::new();
        hasher.update(source.to_string_lossy().as_bytes());
        hasher.update(&mtime.to_le_bytes());

        Ok(hasher.finalize().to_hex()[..HASH_PREFIX_LEN].to_string())
    }

    async fn playlist(&self, source: &Path, source_key: &str) -> Result<String, TranscodeError> {
        let duration = self.probe_duration(source).await?;
        let seg_dur = self.segment_duration as f64;
        let segment_count = (duration / seg_dur).ceil() as u32;

        let mut m3u8 = String::new();
        m3u8.push_str("#EXTM3U\n");
        m3u8.push_str("#EXT-X-VERSION:3\n");
        let _ = writeln!(m3u8, "#EXT-X-TARGETDURATION:{}", self.segment_duration);
        m3u8.push_str("#EXT-X-MEDIA-SEQUENCE:0\n");
        m3u8.push_str("#EXT-X-PLAYLIST-TYPE:VOD\n");

        for i in 0..segment_count {
            let remaining = duration - (i as f64 * seg_dur);
            let actual_dur = if remaining < seg_dur {
                remaining
            } else {
                seg_dur
            };
            let _ = writeln!(m3u8, "#EXTINF:{actual_dur:.3},");
            let _ = writeln!(m3u8, "/api/hls/segment/{source_key}/{i}.ts");
        }

        m3u8.push_str("#EXT-X-ENDLIST\n");

        Ok(m3u8)
    }

    async fn segment(
        &self,
        source: &Path,
        source_key: &str,
        segment_index: u32,
    ) -> Result<PathBuf, TranscodeError> {
        let key_dir = self.segment_dir.join(source_key);
        let segment_path = key_dir.join(format!("{segment_index}.ts"));

        // Fast path: already cached.
        if segment_path.exists() {
            return Ok(segment_path);
        }

        let _permit = self
            .semaphore
            .acquire()
            .await
            .map_err(|_| TranscodeError::Unavailable)?;

        // Double-check after acquiring permit (another task may have generated it).
        if segment_path.exists() {
            return Ok(segment_path);
        }

        tokio::fs::create_dir_all(&key_dir)
            .await
            .map_err(|_| TranscodeError::IoError)?;

        let start_time = segment_index as f64 * self.segment_duration as f64;

        let output = tokio::process::Command::new("ffmpeg")
            .args(["-y", "-ss", &format!("{start_time}"), "-i"])
            .arg(source)
            .args([
                "-t",
                &format!("{}", self.segment_duration),
                "-c:v",
                "libx264",
                "-preset",
                "veryfast",
                "-crf",
                "23",
                "-c:a",
                "aac",
                "-b:a",
                "128k",
                "-f",
                "mpegts",
                "-vsync",
                "cfr",
            ])
            .arg(&segment_path)
            .output()
            .await
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    TranscodeError::FfmpegNotFound
                } else {
                    TranscodeError::IoError
                }
            })?;

        if !output.status.success() {
            tracing::error!(
                "ffmpeg transcode failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let _ = tokio::fs::remove_file(&segment_path).await;
            return Err(TranscodeError::TranscodeFailed);
        }

        Ok(segment_path)
    }
}

/// Periodically cleans up stale HLS segment directories.
///
/// Runs every 30 minutes. Removes subdirectories of `hls_dir` where all files
/// are older than 2 hours. Respects the given cancellation token for graceful
/// shutdown.
pub async fn cleanup_hls_segments(hls_dir: PathBuf, token: tokio_util::sync::CancellationToken) {
    use std::time::Duration;
    use tokio::fs;

    let interval_dur = Duration::from_secs(30 * 60);
    let max_age = Duration::from_secs(2 * 60 * 60);

    loop {
        tokio::select! {
            _ = tokio::time::sleep(interval_dur) => {}
            _ = token.cancelled() => {
                tracing::info!("HLS cleanup task shutting down");
                return;
            }
        }

        tracing::debug!("Running HLS segment cleanup");
        let mut removed = 0u32;

        let Ok(mut entries) = fs::read_dir(&hls_dir).await else {
            continue;
        };

        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            let Ok(ft) = entry.file_type().await else {
                continue;
            };
            if !ft.is_dir() {
                continue;
            }

            // Check if all files in this subdirectory are older than max_age.
            let Ok(all_stale) = check_dir_all_stale(&path, max_age).await else {
                continue;
            };

            if all_stale {
                if let Err(e) = fs::remove_dir_all(&path).await {
                    tracing::warn!("Failed to remove stale HLS dir {}: {e}", path.display());
                } else {
                    removed += 1;
                }
            }
        }

        if removed > 0 {
            tracing::info!(removed, "HLS segment cleanup complete");
        }
    }
}

async fn check_dir_all_stale(dir: &Path, max_age: std::time::Duration) -> std::io::Result<bool> {
    use std::time::SystemTime;
    use tokio::fs;

    let now = SystemTime::now();
    let mut entries = fs::read_dir(dir).await?;

    while let Some(entry) = entries.next_entry().await? {
        let meta = entry.metadata().await?;
        let modified = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        if now.duration_since(modified).unwrap_or_default() < max_age {
            return Ok(false);
        }
    }

    // All files are older than max_age, or directory is empty — consider stale.
    Ok(true)
}

#[derive(Debug, thiserror::Error)]
pub enum TranscodeError {
    #[error("ffmpeg not found")]
    FfmpegNotFound,
    #[error("ffprobe failed")]
    ProbeFailed,
    #[error("transcode failed")]
    TranscodeFailed,
    #[error("IO error")]
    IoError,
    #[error("transcoder unavailable")]
    Unavailable,
}
