use std::path::{Component, Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::error::AppError;

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct FileEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
    pub modified: DateTime<Utc>,
    pub mime_type: Option<String>,
    pub extension: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct DirListing {
    pub path: String,
    pub items: Vec<FileEntry>,
    pub num_dirs: usize,
    pub num_files: usize,
    /// Total items before pagination (if truncated).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<usize>,
    /// Whether the listing was truncated by the max_items limit.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub truncated: bool,
}

// ---------------------------------------------------------------------------
// Path safety
// ---------------------------------------------------------------------------

/// Resolve a user-provided path safely within the given root directory.
///
/// Rejects null bytes and path traversal attempts. Only `Component::Normal`
/// segments are kept. The resolved path is canonicalized and verified to remain
/// under the canonical root to prevent directory-traversal attacks.
///
/// `canonical_root` must already be canonicalized (computed once at startup).
pub fn safe_resolve(canonical_root: &Path, user_path: &str) -> Result<PathBuf, AppError> {
    // Reject null bytes — a classic path injection vector.
    if user_path.as_bytes().contains(&0) {
        return Err(AppError::BadRequest("Invalid path: null bytes not allowed".into()));
    }

    // Reject paths containing backslash (normalise on forward-slash only).
    if user_path.contains('\\') {
        return Err(AppError::BadRequest("Invalid path: backslashes not allowed".into()));
    }

    // Build a clean relative path from only Normal components.
    let mut relative = PathBuf::new();
    for component in Path::new(user_path).components() {
        if let Component::Normal(seg) = component {
            // Reject segment-level dangerous names.
            let s = seg.to_string_lossy();
            if s.starts_with('.') && s != "." {
                // Allow dotfiles but log them; reject pure ".." (already filtered).
            }
            relative.push(seg);
        }
        // All other components (RootDir, CurDir, ParentDir, Prefix) are
        // silently dropped to prevent directory-traversal attacks.
    }

    let target = canonical_root.join(&relative);

    // If the full target exists, canonicalize it directly.
    // Otherwise, walk up to the nearest existing ancestor, canonicalize that,
    // and re-append the remaining segments.
    let canonical_target = if target.exists() {
        target.canonicalize().map_err(AppError::Io)?
    } else {
        // Find the nearest existing ancestor.
        let mut existing = target.clone();
        let mut tail_parts: Vec<std::ffi::OsString> = Vec::new();
        while !existing.exists() {
            if let Some(name) = existing.file_name() {
                tail_parts.push(name.to_os_string());
            }
            if !existing.pop() {
                break;
            }
        }
        let mut resolved = existing.canonicalize().map_err(AppError::Io)?;
        for part in tail_parts.into_iter().rev() {
            resolved.push(part);
        }
        resolved
    };

    if !canonical_target.starts_with(canonical_root) {
        return Err(AppError::Forbidden("Path escapes root directory".into()));
    }

    Ok(canonical_target)
}

// ---------------------------------------------------------------------------
// Directory listing
// ---------------------------------------------------------------------------

/// List the contents of a directory, returning metadata for every entry.
///
/// `max_items` caps the number of entries returned to prevent unbounded memory
/// usage on huge directories (pattern from FileBrowser's paginated listing).
pub async fn list_directory(
    canonical_root: &Path,
    dir_path: &Path,
    max_items: usize,
) -> Result<DirListing, AppError> {
    let mut entries = tokio::fs::read_dir(dir_path)
        .await
        .map_err(|_| AppError::NotFound("Cannot read directory".into()))?;

    let mut items = Vec::new();
    let mut total_count: usize = 0;

    while let Some(entry) = entries.next_entry().await.map_err(AppError::Io)? {
        total_count += 1;

        // Stop collecting after max_items but keep counting total.
        if items.len() >= max_items {
            continue;
        }

        let metadata = entry.metadata().await.map_err(AppError::Io)?;
        let entry_path = entry.path();

        // Compute path relative to root.
        let rel_path = entry_path
            .strip_prefix(canonical_root)
            .unwrap_or(&entry_path)
            .to_string_lossy()
            .into_owned();

        let is_dir = metadata.is_dir();
        let size = if is_dir { 0 } else { metadata.len() };

        let modified: DateTime<Utc> = metadata
            .modified()
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
            .into();

        let name = entry
            .file_name()
            .to_string_lossy()
            .into_owned();

        let extension = if is_dir {
            None
        } else {
            entry_path
                .extension()
                .map(|e| e.to_string_lossy().into_owned())
        };

        let mime_type = if is_dir {
            None
        } else {
            mime_guess::from_path(&entry_path)
                .first()
                .map(|m| m.to_string())
        };

        items.push(FileEntry {
            name,
            path: rel_path,
            is_dir,
            size,
            modified,
            mime_type,
            extension,
        });
    }

    let num_dirs = items.iter().filter(|e| e.is_dir).count();
    let num_files = items.iter().filter(|e| !e.is_dir).count();
    let truncated = total_count > max_items;

    // Compute the listing path relative to root.
    let listing_path = dir_path
        .strip_prefix(canonical_root)
        .unwrap_or(Path::new(""))
        .to_string_lossy()
        .into_owned();

    Ok(DirListing {
        path: listing_path,
        items,
        num_dirs,
        num_files,
        total: if truncated { Some(total_count) } else { None },
        truncated,
    })
}

// ---------------------------------------------------------------------------
// Single file info
// ---------------------------------------------------------------------------

/// Return metadata for a single file or directory.
pub async fn file_info(canonical_root: &Path, file_path: &Path) -> Result<FileEntry, AppError> {
    let metadata = tokio::fs::metadata(file_path)
        .await
        .map_err(|_| {
            // Return only the relative path to avoid leaking server filesystem layout.
            let rel = file_path
                .strip_prefix(canonical_root)
                .unwrap_or(Path::new("unknown"));
            AppError::NotFound(format!("Not found: {}", rel.display()))
        })?;

    let is_dir = metadata.is_dir();
    let size = if is_dir { 0 } else { metadata.len() };

    let modified: DateTime<Utc> = metadata
        .modified()
        .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
        .into();

    let name = file_path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();

    let rel_path = file_path
        .strip_prefix(canonical_root)
        .unwrap_or(file_path)
        .to_string_lossy()
        .to_string();

    let extension = if is_dir {
        None
    } else {
        file_path
            .extension()
            .map(|e| e.to_string_lossy().to_string())
    };

    let mime_type = if is_dir {
        None
    } else {
        mime_guess::from_path(file_path)
            .first()
            .map(|m| m.to_string())
    };

    Ok(FileEntry {
        name,
        path: rel_path,
        is_dir,
        size,
        modified,
        mime_type,
        extension,
    })
}

// ---------------------------------------------------------------------------
// File content operations
// ---------------------------------------------------------------------------

/// Read a text file's contents. Rejects files larger than 5 MB and binary files.
pub async fn read_text_content(file_path: &Path) -> Result<String, AppError> {
    const MAX_SIZE: u64 = 5 * 1024 * 1024; // 5 MB

    let metadata = tokio::fs::metadata(file_path)
        .await
        .map_err(|_| AppError::NotFound("File not found".into()))?;

    if metadata.len() > MAX_SIZE {
        return Err(AppError::BadRequest(
            "File exceeds 5 MB text preview limit".into(),
        ));
    }

    let bytes = tokio::fs::read(file_path).await.map_err(AppError::Io)?;

    String::from_utf8(bytes)
        .map_err(|_| AppError::BadRequest("File appears to be binary, not text".into()))
}

/// Write content to a file atomically (write to temp file, then rename).
pub async fn write_file(file_path: &Path, content: &[u8]) -> Result<(), AppError> {
    let parent = file_path
        .parent()
        .ok_or_else(|| AppError::BadRequest("Invalid file path".into()))?;

    // Ensure parent directory exists.
    tokio::fs::create_dir_all(parent).await.map_err(AppError::Io)?;

    let tmp_name = format!(
        ".rustyfile_tmp_{}",
        uuid::Uuid::new_v4().as_hyphenated()
    );
    let tmp_path = parent.join(tmp_name);

    tokio::fs::write(&tmp_path, content)
        .await
        .map_err(AppError::Io)?;

    tokio::fs::rename(&tmp_path, file_path)
        .await
        .map_err(|e| {
            // Best-effort cleanup of the temp file.
            let tmp = tmp_path.clone();
            tokio::spawn(async move {
                let _ = tokio::fs::remove_file(tmp).await;
            });
            AppError::Io(e)
        })?;

    Ok(())
}

/// Create a directory (and all missing parents).
pub async fn create_directory(dir_path: &Path) -> Result<(), AppError> {
    tokio::fs::create_dir_all(dir_path)
        .await
        .map_err(AppError::Io)
}

/// Delete a file or directory (recursive for directories).
pub async fn delete(file_path: &Path) -> Result<(), AppError> {
    let metadata = tokio::fs::metadata(file_path)
        .await
        .map_err(|_| AppError::NotFound("Path not found".into()))?;

    if metadata.is_dir() {
        tokio::fs::remove_dir_all(file_path)
            .await
            .map_err(AppError::Io)
    } else {
        tokio::fs::remove_file(file_path)
            .await
            .map_err(AppError::Io)
    }
}

/// Rename (move) a file or directory. Ensures the destination's parent exists.
///
/// Refuses to overwrite an existing destination to prevent silent data loss
/// (pattern from FileBrowser which returns 409 Conflict on overwrite attempts).
pub async fn rename(from: &Path, to: &Path, overwrite: bool) -> Result<(), AppError> {
    if let Some(parent) = to.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(AppError::Io)?;
    }

    // Check for overwrite unless explicitly allowed.
    if !overwrite && to.exists() {
        return Err(AppError::Conflict(
            "Destination already exists".into(),
        ));
    }

    tokio::fs::rename(from, to).await.map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => AppError::NotFound("Source not found".into()),
        _ => AppError::Io(e),
    })
}

// ---------------------------------------------------------------------------
// Subtitle detection
// ---------------------------------------------------------------------------

/// Non-blocking subtitle detection: spawns blocking I/O on a worker thread.
pub async fn detect_subtitles(video_path: PathBuf) -> Vec<String> {
    tokio::task::spawn_blocking(move || detect_subtitles_sync(&video_path))
        .await
        .unwrap_or_default()
}

/// Look for subtitle files (.vtt, .srt, .ass, .ssa) with the same base name as
/// the given video file in the same directory.
fn detect_subtitles_sync(video_path: &Path) -> Vec<String> {
    let parent = match video_path.parent() {
        Some(p) => p,
        None => return Vec::new(),
    };

    let stem = match video_path.file_stem() {
        Some(s) => s.to_string_lossy().to_string(),
        None => return Vec::new(),
    };

    let subtitle_extensions = ["vtt", "srt", "ass", "ssa"];
    let mut subtitles = Vec::new();

    let entries = match std::fs::read_dir(parent) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }

        let ext = match path.extension() {
            Some(e) => e.to_string_lossy().to_lowercase(),
            None => continue,
        };

        if !subtitle_extensions.contains(&ext.as_str()) {
            continue;
        }

        let file_stem = match path.file_stem() {
            Some(s) => s.to_string_lossy().to_string(),
            None => continue,
        };

        // Match exact stem or stem with a language suffix (e.g. "movie.en").
        if file_stem == stem || file_stem.starts_with(&format!("{stem}.")) {
            subtitles.push(path.file_name().unwrap().to_string_lossy().to_string());
        }
    }

    subtitles.sort();
    subtitles
}
