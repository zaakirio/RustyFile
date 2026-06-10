//! Server-side archive support: streaming ZIP creation and safe extraction.
//!
//! All functions here are synchronous and intended to run inside
//! `tokio::task::spawn_blocking`.

use std::collections::HashSet;
use std::fs::File;
use std::io::{BufReader, Read, Write};
use std::path::{Component, Path, PathBuf};

use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use crate::error::AppError;

/// Zip-bomb guard: total uncompressed bytes a single extraction may produce.
pub(crate) const MAX_EXTRACT_TOTAL_BYTES: u64 = 10 * 1024 * 1024 * 1024; // 10 GiB
/// Zip-bomb guard: uncompressed bytes a single entry may produce.
pub(crate) const MAX_EXTRACT_ENTRY_BYTES: u64 = 4 * 1024 * 1024 * 1024; // 4 GiB
/// Zip-bomb guard: maximum number of entries in an archive.
pub(crate) const MAX_EXTRACT_ENTRIES: usize = 100_000;

/// Mirrors the search indexer's defensive recursion cap; symlinks are
/// skipped, so this only guards pathologically deep trees.
const MAX_WALK_DEPTH: usize = 64;

/// Extensions that are already compressed: deflating them again wastes CPU
/// for ~0% gain, so they are stored verbatim.
const STORED_EXTENSIONS: &[&str] = &[
    "zip", "gz", "tgz", "bz2", "xz", "zst", "7z", "rar", "jpg", "jpeg", "png", "gif", "webp",
    "avif", "heic", "mp4", "mkv", "webm", "mov", "avi", "mp3", "m4a", "aac", "ogg", "opus", "flac",
    "woff", "woff2",
];

/// A root-level item selected for inclusion in a ZIP download.
#[derive(Debug)]
pub(crate) struct ZipSource {
    /// Absolute, jail-checked path on disk.
    pub abs_path: PathBuf,
    /// Entry name inside the archive (no separators).
    pub entry_name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ArchiveFormat {
    Zip,
    TarGz,
}

impl ArchiveFormat {
    /// Detects the archive format from a file name, or `None` if unsupported.
    pub(crate) fn from_name(name: &str) -> Option<Self> {
        let lower = name.to_lowercase();
        if lower.ends_with(".zip") {
            Some(Self::Zip)
        } else if lower.ends_with(".tar.gz") || lower.ends_with(".tgz") {
            Some(Self::TarGz)
        } else {
            None
        }
    }

    /// Strips the archive extension to derive the default destination name.
    pub(crate) fn strip_extension(self, name: &str) -> String {
        let lower = name.to_lowercase();
        let suffix_len = match self {
            Self::Zip => ".zip".len(),
            Self::TarGz if lower.ends_with(".tar.gz") => ".tar.gz".len(),
            Self::TarGz => ".tgz".len(),
        };
        name[..name.len() - suffix_len].to_string()
    }
}

#[derive(Debug)]
pub(crate) struct ExtractReport {
    /// Files and directories written.
    pub entries: usize,
}

fn options_for(path: &Path, size: u64) -> SimpleFileOptions {
    let already_compressed = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| STORED_EXTENSIONS.contains(&e.to_lowercase().as_str()))
        .unwrap_or(false);

    let method = if already_compressed {
        CompressionMethod::Stored
    } else {
        CompressionMethod::Deflated
    };

    SimpleFileOptions::default()
        .compression_method(method)
        .large_file(size >= u32::MAX as u64)
}

/// Streams a ZIP archive of the given sources into `out`.
///
/// Uses `ZipWriter::new_stream`, which writes data descriptors instead of
/// seeking back to patch local headers, so `out` only needs `Write` (it is
/// bridged to the HTTP response body by the caller). Directories are walked
/// recursively; symlinks are skipped entirely, matching the search indexer.
pub(crate) fn write_zip<W: Write>(out: W, sources: &[ZipSource]) -> Result<(), AppError> {
    let mut zip = ZipWriter::new_stream(out);

    for source in sources {
        let metadata = std::fs::symlink_metadata(&source.abs_path)
            .map_err(|_| AppError::NotFound(format!("Not found: {}", source.entry_name)))?;

        if metadata.is_symlink() {
            continue;
        }

        if metadata.is_dir() {
            zip_dir_recursive(&mut zip, &source.abs_path, &source.entry_name, 0)?;
        } else {
            zip_file(
                &mut zip,
                &source.abs_path,
                &source.entry_name,
                metadata.len(),
            )?;
        }
    }

    zip.finish()?.flush().map_err(AppError::Io)?;
    Ok(())
}

fn zip_file<W: Write + std::io::Seek>(
    zip: &mut ZipWriter<W>,
    path: &Path,
    entry_name: &str,
    size: u64,
) -> Result<(), AppError> {
    zip.start_file(entry_name, options_for(path, size))?;
    let mut file = File::open(path).map_err(AppError::Io)?;
    std::io::copy(&mut file, zip).map_err(AppError::Io)?;
    Ok(())
}

fn zip_dir_recursive<W: Write + std::io::Seek>(
    zip: &mut ZipWriter<W>,
    dir: &Path,
    prefix: &str,
    depth: usize,
) -> Result<(), AppError> {
    if depth >= MAX_WALK_DEPTH {
        tracing::warn!(
            "Skipping {}: max walk depth ({MAX_WALK_DEPTH}) reached",
            dir.display()
        );
        return Ok(());
    }

    zip.add_directory(format!("{prefix}/"), SimpleFileOptions::default())?;

    let read_dir = std::fs::read_dir(dir).map_err(AppError::Io)?;
    let mut entries: Vec<_> = read_dir.filter_map(|e| e.ok()).collect();
    entries.sort_by_key(std::fs::DirEntry::file_name);

    for entry in entries {
        let path = entry.path();

        // file_type() never follows symlinks; skip them entirely so symlink
        // cycles cannot recurse forever and out-of-root targets never leak
        // into the archive (house rule, see search indexer).
        let file_type = match entry.file_type() {
            Ok(ft) => ft,
            Err(e) => {
                tracing::warn!("Skipping {}: {e}", path.display());
                continue;
            }
        };
        if file_type.is_symlink() {
            continue;
        }

        let name = entry.file_name().to_string_lossy().into_owned();
        let entry_name = format!("{prefix}/{name}");

        if file_type.is_dir() {
            zip_dir_recursive(zip, &path, &entry_name, depth + 1)?;
        } else {
            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
            zip_file(zip, &path, &entry_name, size)?;
        }
    }

    Ok(())
}

/// Filters an archive entry name through the same component logic as
/// `safe_resolve`, but *rejects* (rather than strips) traversal attempts so
/// malicious archives fail loudly.
///
/// Returns `Ok(None)` for entries that resolve to nothing (e.g. `.` or `/`),
/// which callers should skip.
fn sanitize_entry_path(name: &str) -> Result<Option<PathBuf>, AppError> {
    if name.as_bytes().contains(&0) {
        return Err(AppError::BadRequest(format!(
            "Archive entry contains null bytes: {name:?}"
        )));
    }

    if name.contains('\\') {
        return Err(AppError::BadRequest(format!(
            "Archive entry contains backslashes: {name:?}"
        )));
    }

    let mut relative = PathBuf::new();
    for component in Path::new(name).components() {
        match component {
            Component::Normal(seg) => relative.push(seg),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(AppError::BadRequest(format!(
                    "Archive entry escapes destination: {name:?}"
                )));
            }
        }
    }

    if relative.as_os_str().is_empty() {
        Ok(None)
    } else {
        Ok(Some(relative))
    }
}

/// Extracts an archive into `dest` without overwriting anything.
///
/// Strategy: entries are first extracted into a hidden staging directory
/// inside `dest` (so a half-finished extraction is cleaned up with a single
/// `remove_dir_all`), then the staged top-level items are renamed into
/// `dest`. If any top-level name already exists in `dest`, the whole
/// extraction is aborted with a 409 listing the first conflict — extraction
/// never merges into or overwrites existing files.
///
/// Security: entry names are component-filtered (zip-slip), symlink entries
/// are skipped, and named-constant caps bound entry count, per-entry size
/// and total uncompressed size (zip-bomb). Caps are enforced on *actual*
/// decompressed bytes, not the sizes declared in headers.
pub(crate) fn extract_archive(
    format: ArchiveFormat,
    archive_path: &Path,
    dest: &Path,
) -> Result<ExtractReport, AppError> {
    let dest_preexisted = dest.symlink_metadata().is_ok();
    std::fs::create_dir_all(dest).map_err(AppError::Io)?;

    let staging = dest.join(format!(
        ".rustyfile_extract_{}",
        uuid::Uuid::new_v4().as_hyphenated()
    ));
    std::fs::create_dir(&staging).map_err(AppError::Io)?;

    let result = match format {
        ArchiveFormat::Zip => extract_zip_into(archive_path, &staging),
        ArchiveFormat::TarGz => extract_tar_gz_into(archive_path, &staging),
    }
    .and_then(|entries| {
        promote_staged(&staging, dest)?;
        Ok(ExtractReport { entries })
    });

    if result.is_err() {
        // Abort: remove everything we created (partial extraction cleanup).
        let _ = std::fs::remove_dir_all(&staging);
        if !dest_preexisted {
            // remove_dir only succeeds when empty, so a pre-populated
            // destination can never be deleted by accident.
            let _ = std::fs::remove_dir(dest);
        }
    } else {
        let _ = std::fs::remove_dir(&staging);
    }

    result
}

/// Moves staged top-level items into `dest`, failing on the first conflict
/// before anything is moved.
fn promote_staged(staging: &Path, dest: &Path) -> Result<(), AppError> {
    let staged: Vec<_> = std::fs::read_dir(staging)
        .map_err(AppError::Io)?
        .filter_map(|e| e.ok())
        .collect();

    for entry in &staged {
        let target = dest.join(entry.file_name());
        if target.symlink_metadata().is_ok() {
            return Err(AppError::Conflict(format!(
                "Destination already contains '{}'",
                entry.file_name().to_string_lossy()
            )));
        }
    }

    for entry in &staged {
        let target = dest.join(entry.file_name());
        std::fs::rename(entry.path(), &target).map_err(AppError::Io)?;
    }

    Ok(())
}

/// Tracks the running uncompressed total across all entries.
struct ExtractBudget {
    total: u64,
}

impl ExtractBudget {
    fn new() -> Self {
        Self { total: 0 }
    }

    /// Copies one entry's bytes to `out`, enforcing per-entry and total caps
    /// on the actual decompressed stream.
    fn copy_entry<R: Read, W: Write>(&mut self, reader: R, out: &mut W) -> Result<(), AppError> {
        let entry_allowance = MAX_EXTRACT_ENTRY_BYTES.min(MAX_EXTRACT_TOTAL_BYTES - self.total);

        // Read one byte past the allowance to detect overflow.
        let written =
            std::io::copy(&mut reader.take(entry_allowance + 1), out).map_err(AppError::Io)?;

        if written > entry_allowance {
            return Err(AppError::BadRequest(format!(
                "Archive exceeds extraction limits ({MAX_EXTRACT_ENTRY_BYTES} bytes per entry, {MAX_EXTRACT_TOTAL_BYTES} bytes total)"
            )));
        }

        self.total += written;
        Ok(())
    }
}

fn entry_count_error() -> AppError {
    AppError::BadRequest(format!(
        "Archive has too many entries (limit: {MAX_EXTRACT_ENTRIES})"
    ))
}

fn write_staged_file<R: Read>(
    staging: &Path,
    rel: &Path,
    reader: R,
    budget: &mut ExtractBudget,
) -> Result<(), AppError> {
    let target = staging.join(rel);
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(AppError::Io)?;
    }

    // create_new: never overwrite, even within the staging area (duplicate
    // entry names in a crafted archive).
    let mut out = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&target)
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::AlreadyExists => AppError::BadRequest(format!(
                "Archive contains duplicate entry: {}",
                rel.display()
            )),
            _ => AppError::Io(e),
        })?;

    budget.copy_entry(reader, &mut out)
}

fn extract_zip_into(archive_path: &Path, staging: &Path) -> Result<usize, AppError> {
    let file = File::open(archive_path).map_err(AppError::Io)?;
    let mut archive = ZipArchive::new(BufReader::new(file))?;

    if archive.len() > MAX_EXTRACT_ENTRIES {
        return Err(entry_count_error());
    }

    let mut budget = ExtractBudget::new();
    let mut written = 0usize;

    for index in 0..archive.len() {
        let entry = archive.by_index(index)?;

        // Symlink entries are skipped entirely: extracting them would allow
        // later writes to traverse outside the jail.
        if entry.is_symlink() {
            continue;
        }

        let Some(rel) = sanitize_entry_path(entry.name())? else {
            continue;
        };

        if entry.is_dir() {
            std::fs::create_dir_all(staging.join(&rel)).map_err(AppError::Io)?;
        } else {
            write_staged_file(staging, &rel, entry, &mut budget)?;
        }
        written += 1;
    }

    Ok(written)
}

fn extract_tar_gz_into(archive_path: &Path, staging: &Path) -> Result<usize, AppError> {
    let file = File::open(archive_path).map_err(AppError::Io)?;
    let decoder = flate2::read::GzDecoder::new(BufReader::new(file));
    let mut archive = tar::Archive::new(decoder);

    let mut budget = ExtractBudget::new();
    let mut written = 0usize;
    let mut seen = 0usize;

    for entry in archive.entries().map_err(AppError::Io)? {
        let entry = entry
            .map_err(|e| AppError::BadRequest(format!("Invalid or unsupported archive: {e}")))?;

        seen += 1;
        if seen > MAX_EXTRACT_ENTRIES {
            return Err(entry_count_error());
        }

        let entry_type = entry.header().entry_type();

        let name = String::from_utf8_lossy(&entry.path_bytes()).into_owned();
        let Some(rel) = sanitize_entry_path(&name)? else {
            continue;
        };

        match entry_type {
            tar::EntryType::Directory => {
                std::fs::create_dir_all(staging.join(&rel)).map_err(AppError::Io)?;
            }
            tar::EntryType::Regular => {
                write_staged_file(staging, &rel, entry, &mut budget)?;
            }
            // Symlinks, hard links, devices, etc. are skipped entirely.
            _ => continue,
        }
        written += 1;
    }

    Ok(written)
}

/// Deduplicates top-level entry names: a second `report.txt` becomes
/// `report (1).txt` rather than failing the whole archive.
pub(crate) fn dedupe_entry_name(name: &str, used: &mut HashSet<String>) -> String {
    if used.insert(name.to_string()) {
        return name.to_string();
    }

    let (stem, ext) = match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => (stem, format!(".{ext}")),
        _ => (name, String::new()),
    };

    for n in 1.. {
        let candidate = format!("{stem} ({n}){ext}");
        if used.insert(candidate.clone()) {
            return candidate;
        }
    }
    unreachable!("dedupe counter exhausted")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_accepts_nested_paths() {
        let rel = sanitize_entry_path("a/b/c.txt").unwrap().unwrap();
        assert_eq!(rel, PathBuf::from("a/b/c.txt"));
    }

    #[test]
    fn sanitize_skips_curdir_components() {
        let rel = sanitize_entry_path("./a/./b.txt").unwrap().unwrap();
        assert_eq!(rel, PathBuf::from("a/b.txt"));
    }

    #[test]
    fn sanitize_rejects_traversal() {
        for name in ["../escape.txt", "a/../../b", "/etc/passwd"] {
            let err = sanitize_entry_path(name).unwrap_err();
            assert!(
                matches!(err, AppError::BadRequest(_)),
                "{name} not rejected"
            );
        }
    }

    #[test]
    fn sanitize_rejects_backslashes_and_nulls() {
        assert!(matches!(
            sanitize_entry_path("a\\b.txt").unwrap_err(),
            AppError::BadRequest(_)
        ));
        assert!(matches!(
            sanitize_entry_path("a\0b.txt").unwrap_err(),
            AppError::BadRequest(_)
        ));
    }

    #[test]
    fn sanitize_empty_entries_are_skippable() {
        assert!(sanitize_entry_path(".").unwrap().is_none());
        assert!(sanitize_entry_path("").unwrap().is_none());
    }

    #[test]
    fn format_detection() {
        assert_eq!(ArchiveFormat::from_name("a.zip"), Some(ArchiveFormat::Zip));
        assert_eq!(ArchiveFormat::from_name("a.ZIP"), Some(ArchiveFormat::Zip));
        assert_eq!(
            ArchiveFormat::from_name("a.tar.gz"),
            Some(ArchiveFormat::TarGz)
        );
        assert_eq!(
            ArchiveFormat::from_name("a.tgz"),
            Some(ArchiveFormat::TarGz)
        );
        assert_eq!(ArchiveFormat::from_name("a.rar"), None);
        assert_eq!(ArchiveFormat::from_name("a.gz"), None);
    }

    #[test]
    fn strip_extension_variants() {
        assert_eq!(ArchiveFormat::Zip.strip_extension("dir.zip"), "dir");
        assert_eq!(ArchiveFormat::TarGz.strip_extension("dir.tar.gz"), "dir");
        assert_eq!(ArchiveFormat::TarGz.strip_extension("dir.tgz"), "dir");
    }

    #[test]
    fn dedupe_names() {
        let mut used = HashSet::new();
        assert_eq!(dedupe_entry_name("a.txt", &mut used), "a.txt");
        assert_eq!(dedupe_entry_name("a.txt", &mut used), "a (1).txt");
        assert_eq!(dedupe_entry_name("a.txt", &mut used), "a (2).txt");
        assert_eq!(dedupe_entry_name("dir", &mut used), "dir");
        assert_eq!(dedupe_entry_name("dir", &mut used), "dir (1)");
    }
}
