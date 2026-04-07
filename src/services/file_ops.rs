use std::path::{Component, Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::error::AppError;

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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<usize>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub truncated: bool,
}

/// Resolve user path safely within root. Rejects traversal attempts and
/// null bytes. `canonical_root` must already be canonicalized.
pub fn safe_resolve(canonical_root: &Path, user_path: &str) -> Result<PathBuf, AppError> {
    if user_path.as_bytes().contains(&0) {
        return Err(AppError::BadRequest(
            "Invalid path: null bytes not allowed".into(),
        ));
    }

    if user_path.contains('\\') {
        return Err(AppError::BadRequest(
            "Invalid path: backslashes not allowed".into(),
        ));
    }

    // Only Normal components are kept; RootDir/CurDir/ParentDir/Prefix
    // are dropped to prevent traversal.
    let mut relative = PathBuf::new();
    for component in Path::new(user_path).components() {
        if let Component::Normal(seg) = component {
            relative.push(seg);
        }
    }

    let target = canonical_root.join(&relative);

    // For non-existent paths, canonicalize nearest ancestor then re-append remainder.
    let canonical_target = if target.exists() {
        target.canonicalize().map_err(AppError::Io)?
    } else {
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

/// `max_items` caps entries to prevent unbounded memory usage on huge directories.
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

        if items.len() >= max_items {
            continue;
        }

        let metadata = entry.metadata().await.map_err(AppError::Io)?;
        let entry_path = entry.path();

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

        let name = entry.file_name().to_string_lossy().into_owned();

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

    // num_dirs/num_files reflect only the returned (visible) entries.
    // When truncated, the total entry count is in `total` but the
    // dir/file breakdown of unreturned entries is unknown.
    let num_dirs = items.iter().filter(|e| e.is_dir).count();
    let num_files = items.iter().filter(|e| !e.is_dir).count();
    let truncated = total_count > max_items;

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

pub async fn file_info(canonical_root: &Path, file_path: &Path) -> Result<FileEntry, AppError> {
    let metadata = tokio::fs::metadata(file_path).await.map_err(|_| {
        // Use relative path to avoid leaking server filesystem layout.
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

/// Atomic write: temp file + rename to prevent partial writes.
pub async fn write_file(file_path: &Path, content: &[u8]) -> Result<(), AppError> {
    let parent = file_path
        .parent()
        .ok_or_else(|| AppError::BadRequest("Invalid file path".into()))?;

    tokio::fs::create_dir_all(parent)
        .await
        .map_err(AppError::Io)?;

    let tmp_name = format!(".rustyfile_tmp_{}", uuid::Uuid::new_v4().as_hyphenated());
    let tmp_path = parent.join(tmp_name);

    {
        use tokio::io::AsyncWriteExt;

        let mut opts = tokio::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        opts.mode(0o600);

        let mut file = opts.open(&tmp_path).await.map_err(AppError::Io)?;
        file.write_all(content).await.map_err(AppError::Io)?;
        file.sync_data().await.map_err(AppError::Io)?;
    }

    tokio::fs::rename(&tmp_path, file_path).await.map_err(|e| {
        let tmp = tmp_path.clone();
        tokio::spawn(async move {
            let _ = tokio::fs::remove_file(tmp).await;
        });
        AppError::Io(e)
    })?;

    Ok(())
}

pub async fn create_directory(dir_path: &Path) -> Result<(), AppError> {
    // Only create a single directory level; parent must exist.
    tokio::fs::create_dir(dir_path)
        .await
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => {
                AppError::NotFound("Parent directory does not exist".into())
            }
            std::io::ErrorKind::AlreadyExists => {
                AppError::Conflict("Directory already exists".into())
            }
            _ => AppError::Io(e),
        })
}

pub async fn delete(canonical_root: &Path, file_path: &Path) -> Result<(), AppError> {
    if file_path == canonical_root {
        return Err(AppError::Forbidden(
            "Cannot delete the root directory".into(),
        ));
    }

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

/// Rename (move) a file or directory.
///
/// **Note:** The `overwrite=false` check has an inherent TOCTOU race on POSIX:
/// between `to.exists()` returning false and `fs::rename()` executing, another
/// process could create a file at `to`. This is a known limitation of path-based
/// file operations. Use `overwrite=true` when atomic replacement is needed.
pub async fn rename(from: &Path, to: &Path, overwrite: bool) -> Result<(), AppError> {
    if let Some(parent) = to.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(AppError::Io)?;
    }

    if !overwrite && to.exists() {
        return Err(AppError::Conflict("Destination already exists".into()));
    }

    tokio::fs::rename(from, to)
        .await
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => AppError::NotFound("Source not found".into()),
            _ => AppError::Io(e),
        })
}

pub async fn detect_subtitles(video_path: PathBuf) -> Vec<String> {
    tokio::task::spawn_blocking(move || detect_subtitles_sync(&video_path))
        .await
        .unwrap_or_default()
}

fn detect_subtitles_sync(video_path: &Path) -> Vec<String> {
    let Some(parent) = video_path.parent() else {
        return Vec::new();
    };

    let stem = match video_path.file_stem() {
        Some(s) => s.to_string_lossy().to_string(),
        None => return Vec::new(),
    };

    let subtitle_extensions = ["vtt", "srt", "ass", "ssa"];
    let mut subtitles = Vec::new();

    let Ok(entries) = std::fs::read_dir(parent) else {
        return Vec::new();
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

        if file_stem == stem || file_stem.starts_with(&format!("{stem}.")) {
            subtitles.push(path.file_name().unwrap().to_string_lossy().to_string());
        }
    }

    subtitles.sort();
    subtitles
}
