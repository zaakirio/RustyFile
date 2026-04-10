use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::error::AppError;

/// Returns `Err` if the filename's extension is in the pre-parsed blocked set.
pub fn check_blocked_extension(filename: &str, blocked: &HashSet<String>) -> Result<(), AppError> {
    let ext = Path::new(filename)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| format!(".{}", e.to_lowercase()));

    if let Some(ref ext) = ext {
        if blocked.contains(ext) {
            return Err(AppError::BadRequest(format!(
                "File type '{ext}' is not allowed"
            )));
        }
    }
    Ok(())
}

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

#[derive(Debug, Clone, Serialize)]
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

impl FileEntry {
    pub(crate) fn from_path_and_metadata(
        canonical_root: &Path,
        entry_path: &Path,
        metadata: &std::fs::Metadata,
    ) -> Self {
        let is_dir = metadata.is_dir();
        let size = if is_dir { 0 } else { metadata.len() };

        let modified: DateTime<Utc> = metadata
            .modified()
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
            .into();

        let name = entry_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();

        let rel_path = entry_path
            .strip_prefix(canonical_root)
            .unwrap_or(entry_path)
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
            mime_guess::from_path(entry_path)
                .first()
                .map(|m| m.to_string())
        };

        Self {
            name,
            path: rel_path,
            is_dir,
            size,
            modified,
            mime_type,
            extension,
        }
    }
}

pub(crate) fn safe_resolve(canonical_root: &Path, user_path: &str) -> Result<PathBuf, AppError> {
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

    // Drop RootDir/CurDir/ParentDir/Prefix to prevent traversal.
    let mut relative = PathBuf::new();
    for component in Path::new(user_path).components() {
        if let Component::Normal(seg) = component {
            relative.push(seg);
        }
    }

    let target = canonical_root.join(&relative);

    // Non-existent paths: canonicalize nearest ancestor, re-append remainder.
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

pub(crate) async fn list_directory(
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

        items.push(FileEntry::from_path_and_metadata(
            canonical_root,
            &entry_path,
            &metadata,
        ));
    }

    // Counts reflect only returned entries, not truncated ones.
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

pub(crate) async fn file_info(
    canonical_root: &Path,
    file_path: &Path,
) -> Result<FileEntry, AppError> {
    let metadata = tokio::fs::metadata(file_path).await.map_err(|_| {
        let rel = file_path
            .strip_prefix(canonical_root)
            .unwrap_or(Path::new("unknown"));
        AppError::NotFound(format!("Not found: {}", rel.display()))
    })?;
    Ok(FileEntry::from_path_and_metadata(
        canonical_root,
        file_path,
        &metadata,
    ))
}

pub(crate) async fn read_text_content(file_path: &Path) -> Result<String, AppError> {
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

pub(crate) async fn write_file(file_path: &Path, content: &[u8]) -> Result<(), AppError> {
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

pub(crate) async fn create_directory(dir_path: &Path) -> Result<(), AppError> {
    // Single level only; parent must exist.
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

pub(crate) async fn delete(canonical_root: &Path, file_path: &Path) -> Result<(), AppError> {
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

/// TOCTOU: `overwrite=false` has an inherent race between `exists()` and
/// `rename()`. Use `overwrite=true` when atomic replacement is needed.
pub(crate) async fn rename(from: &Path, to: &Path, overwrite: bool) -> Result<(), AppError> {
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

pub(crate) async fn detect_subtitles(video_path: PathBuf) -> Vec<String> {
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
