use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::{DateTime, Utc};
use deadpool_sqlite::Pool;
use rusqlite::params;
use serde::{Deserialize, Serialize};

use crate::services::file_ops::FileEntry;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FileType {
    File,
    Dir,
    Image,
    Video,
    Audio,
    Document,
}

const DEFAULT_SEARCH_LIMIT: u32 = 50;

/// Content larger than this is not indexed for full-text search (the file
/// itself is still name-indexed).
pub const MAX_CONTENT_INDEX_BYTES: u64 = 1024 * 1024; // 1 MiB

/// A NUL byte within the first chunk marks the file as binary (not indexed).
const BINARY_SNIFF_BYTES: usize = 8 * 1024;

/// Markers wrapped around matched terms in FTS5 snippets. Private-use
/// codepoints so the frontend can split on them and highlight safely
/// without HTML injection.
pub const SNIPPET_START: char = '\u{e000}';
pub const SNIPPET_END: char = '\u{e001}';

/// Number of tokens FTS5 includes in a snippet.
const SNIPPET_TOKENS: u32 = 12;

/// Extensions of text-like files whose content is full-text indexed.
const CONTENT_INDEX_EXTENSIONS: &[&str] = &[
    "txt",
    "md",
    "markdown",
    "rst",
    "json",
    "yaml",
    "yml",
    "toml",
    "xml",
    "html",
    "htm",
    "css",
    "js",
    "mjs",
    "cjs",
    "ts",
    "tsx",
    "jsx",
    "rs",
    "py",
    "go",
    "java",
    "c",
    "cc",
    "cpp",
    "h",
    "hpp",
    "sh",
    "bash",
    "zsh",
    "csv",
    "tsv",
    "log",
    "ini",
    "cfg",
    "conf",
    "sql",
    "env",
    "rb",
    "php",
    "swift",
    "kt",
    "lua",
    "vue",
    "svelte",
    "properties",
    "bat",
    "ps1",
];

fn default_limit() -> u32 {
    DEFAULT_SEARCH_LIMIT
}

/// Escape SQLite LIKE metacharacters. Use with `ESCAPE '\'`.
fn escape_like(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

/// Build an FTS5 MATCH expression treating user input as literal terms with
/// AND semantics. Each whitespace-separated term becomes a quoted phrase
/// (embedded double quotes doubled), so raw input — `OR`, `*`, unbalanced
/// quotes, parentheses — can never produce an FTS5 syntax error.
///
/// Returns `None` when the input contains no terms (whitespace-only).
fn fts5_match_expr(q: &str) -> Option<String> {
    let terms: Vec<String> = q
        .split_whitespace()
        .map(|t| format!("\"{}\"", t.replace('"', "\"\"")))
        .collect();
    if terms.is_empty() {
        None
    } else {
        Some(terms.join(" "))
    }
}

/// What the query is matched against: file names (default), file contents,
/// or both.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SearchScope {
    #[default]
    Names,
    Content,
    Both,
}

#[derive(Debug, Deserialize)]
pub struct SearchQuery {
    pub q: String,
    #[serde(default)]
    pub scope: SearchScope,
    #[serde(rename = "type")]
    pub file_type: Option<FileType>,
    pub min_size: Option<u64>,
    pub max_size: Option<u64>,
    pub after: Option<String>,
    pub before: Option<String>,
    pub path: Option<String>,
    #[serde(default = "default_limit")]
    pub limit: u32,
    #[serde(default)]
    pub offset: u32,
}

/// A single search result: the indexed file entry plus, for content
/// matches, an FTS5 snippet with matched terms wrapped in
/// [`SNIPPET_START`]/[`SNIPPET_END`] markers.
#[derive(Debug, Serialize)]
pub struct SearchHit {
    #[serde(flatten)]
    pub entry: FileEntry,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snippet: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SearchResults {
    pub results: Vec<SearchHit>,
    pub total: usize,
    pub query: String,
}

#[async_trait::async_trait]
pub trait SearchIndex: Send + Sync {
    async fn search(&self, query: SearchQuery) -> anyhow::Result<SearchResults>;
    async fn full_reindex(&self) -> anyhow::Result<()>;
    async fn upsert(&self, rel_path: &str) -> anyhow::Result<()>;
    async fn remove(&self, rel_path: &str) -> anyhow::Result<()>;
    async fn remove_prefix(&self, prefix: &str) -> anyhow::Result<()>;
    async fn rename_prefix(&self, old_prefix: &str, new_prefix: &str) -> anyhow::Result<()>;
}

#[derive(Clone)]
pub struct SearchIndexer {
    db: Pool,
    canonical_root: Arc<PathBuf>,
}

struct IndexEntry {
    rel_path: String,
    name: String,
    is_dir: bool,
    size: u64,
    modified: String,
    mime_type: Option<String>,
    extension: Option<String>,
    /// Text content for full-text indexing, when the file is an eligible
    /// text-like type within the size cap. `None` otherwise.
    content: Option<String>,
}

/// Read a file's text content for full-text indexing, or `None` when the
/// file is not eligible: extension not text-like, larger than
/// [`MAX_CONTENT_INDEX_BYTES`], binary (NUL byte in the first chunk), or
/// unreadable. Never fails — ineligible files just skip content indexing.
fn read_indexable_content(abs_path: &Path, size: u64, extension: Option<&str>) -> Option<String> {
    let ext = extension?;
    if !CONTENT_INDEX_EXTENSIONS.contains(&ext) {
        return None;
    }
    if size > MAX_CONTENT_INDEX_BYTES {
        tracing::debug!(
            "Content index: skipping oversized file {} ({size} bytes)",
            abs_path.display()
        );
        return None;
    }

    let bytes = match std::fs::read(abs_path) {
        Ok(b) => b,
        Err(e) => {
            tracing::debug!(
                "Content index: skipping unreadable file {}: {e}",
                abs_path.display()
            );
            return None;
        }
    };

    // Binary sniff: a NUL byte in the first chunk means this is not text.
    let sniff_len = bytes.len().min(BINARY_SNIFF_BYTES);
    if bytes[..sniff_len].contains(&0) {
        tracing::debug!("Content index: skipping binary file {}", abs_path.display());
        return None;
    }

    Some(String::from_utf8_lossy(&bytes).into_owned())
}

impl SearchIndexer {
    pub fn new(db: Pool, canonical_root: Arc<PathBuf>) -> Self {
        Self { db, canonical_root }
    }
}

#[async_trait::async_trait]
impl SearchIndex for SearchIndexer {
    async fn full_reindex(&self) -> anyhow::Result<()> {
        let root = self.canonical_root.clone();

        let entries: Vec<IndexEntry> =
            tokio::task::spawn_blocking(move || walk_tree(&root)).await??;

        let entry_count = entries.len();

        crate::db::interact(&self.db, move |conn| {
            let tx = conn.transaction()?;

            // Collect all live paths for the stale-entry cleanup pass.
            let all_paths: HashSet<&str> = entries.iter().map(|e| e.rel_path.as_str()).collect();

            const BATCH_SIZE: usize = 500;

            // Prepare once and reuse across all rows; re-parsing the INSERT
            // per entry dominates the insert cost otherwise.
            {
                let mut stmt = tx.prepare(
                    "INSERT OR REPLACE INTO file_index \
                     (path, name, is_dir, size, modified, mime_type, extension, indexed_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
                )?;
                for entry in &entries {
                    stmt.execute(params![
                        entry.rel_path,
                        entry.name,
                        entry.is_dir as i32,
                        entry.size as i64,
                        entry.modified,
                        entry.mime_type,
                        entry.extension,
                    ])?;
                }
            }

            // Batch-delete stale entries: SELECT all DB paths, collect stale
            // ones, then delete in batched IN-clauses instead of N individual
            // DELETE statements.
            {
                let mut stmt = tx.prepare("SELECT path FROM file_index")?;
                let stale_paths: Vec<String> = stmt
                    .query_map([], |row| row.get::<_, String>(0))?
                    .filter_map(|r| r.ok())
                    .filter(|p| !all_paths.contains(p.as_str()))
                    .collect();
                drop(stmt);

                for chunk in stale_paths.chunks(BATCH_SIZE) {
                    let placeholders: String = chunk
                        .iter()
                        .enumerate()
                        .map(|(i, _)| format!("?{}", i + 1))
                        .collect::<Vec<_>>()
                        .join(",");
                    let sql = format!("DELETE FROM file_index WHERE path IN ({placeholders})");
                    let params: Vec<&dyn rusqlite::types::ToSql> = chunk
                        .iter()
                        .map(|s| s as &dyn rusqlite::types::ToSql)
                        .collect();
                    tx.execute(&sql, params.as_slice())?;
                }
            }

            // Rebuild the content index from scratch within the same
            // transaction so it can never drift from file_index (renames,
            // stale rows, missed watcher events).
            {
                tx.execute("DELETE FROM files_fts", [])?;
                let mut stmt =
                    tx.prepare("INSERT INTO files_fts (path, content) VALUES (?1, ?2)")?;
                for entry in &entries {
                    if let Some(ref content) = entry.content {
                        stmt.execute(params![entry.rel_path, content])?;
                    }
                }
            }

            tx.commit()?;
            Ok(())
        })
        .await?;

        tracing::info!("Search index: full reindex complete ({entry_count} entries)");
        Ok(())
    }

    async fn upsert(&self, rel_path: &str) -> anyhow::Result<()> {
        let abs_path = self.canonical_root.join(rel_path);
        let rel_path = rel_path.to_string();

        let entry = tokio::task::spawn_blocking(move || -> anyhow::Result<Option<IndexEntry>> {
            // symlink_metadata never follows links — symlinks are not indexed
            // (matching walk_tree), so out-of-root targets stay out of the index.
            let Ok(metadata) = std::fs::symlink_metadata(&abs_path) else {
                return Ok(None); // file was deleted
            };
            if metadata.is_symlink() {
                return Ok(None);
            }

            let is_dir = metadata.is_dir();
            let size = if is_dir { 0 } else { metadata.len() };

            let modified: DateTime<Utc> = metadata
                .modified()
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
                .into();

            let name = abs_path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();

            let extension = if is_dir {
                None
            } else {
                abs_path
                    .extension()
                    .map(|e| e.to_string_lossy().into_owned().to_ascii_lowercase())
            };

            let mime_type = if is_dir {
                None
            } else {
                mime_guess::from_path(&abs_path)
                    .first()
                    .map(|m| m.to_string())
            };

            let content = if is_dir {
                None
            } else {
                read_indexable_content(&abs_path, size, extension.as_deref())
            };

            Ok(Some(IndexEntry {
                rel_path,
                name,
                is_dir,
                size,
                modified: modified.to_rfc3339(),
                mime_type,
                extension,
                content,
            }))
        })
        .await??;

        let Some(entry) = entry else {
            return Ok(());
        };

        crate::db::interact(&self.db, move |conn| {
            let tx = conn.transaction()?;
            tx.execute(
                "INSERT OR REPLACE INTO file_index \
                 (path, name, is_dir, size, modified, mime_type, extension, indexed_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
                params![
                    entry.rel_path,
                    entry.name,
                    entry.is_dir as i32,
                    entry.size as i64,
                    entry.modified,
                    entry.mime_type,
                    entry.extension,
                ],
            )?;
            // FTS5 has no primary key, so replace is delete + insert.
            tx.execute(
                "DELETE FROM files_fts WHERE path = ?1",
                params![entry.rel_path],
            )?;
            if let Some(ref content) = entry.content {
                tx.execute(
                    "INSERT INTO files_fts (path, content) VALUES (?1, ?2)",
                    params![entry.rel_path, content],
                )?;
            }
            tx.commit()?;
            Ok(())
        })
        .await?;

        Ok(())
    }

    async fn remove(&self, rel_path: &str) -> anyhow::Result<()> {
        let rel_path = rel_path.to_string();

        crate::db::interact(&self.db, move |conn| {
            conn.execute("DELETE FROM file_index WHERE path = ?1", params![rel_path])?;
            conn.execute("DELETE FROM files_fts WHERE path = ?1", params![rel_path])?;
            Ok(())
        })
        .await?;

        Ok(())
    }

    async fn remove_prefix(&self, rel_prefix: &str) -> anyhow::Result<()> {
        let exact = rel_prefix.to_string();
        let like_pattern = format!("{}/\\%", escape_like(rel_prefix));

        crate::db::interact(&self.db, move |conn| {
            conn.execute(
                "DELETE FROM file_index WHERE path = ?1 OR path LIKE ?2 ESCAPE '\\'",
                params![exact, like_pattern],
            )?;
            conn.execute(
                "DELETE FROM files_fts WHERE path = ?1 OR path LIKE ?2 ESCAPE '\\'",
                params![exact, like_pattern],
            )?;
            Ok(())
        })
        .await?;

        Ok(())
    }

    async fn rename_prefix(&self, old: &str, new: &str) -> anyhow::Result<()> {
        let old = old.to_string();
        let new = new.to_string();
        let escaped_old = escape_like(&old);

        crate::db::interact(&self.db, move |conn| {
            let new_name = Path::new(&new)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();

            conn.execute(
                "UPDATE file_index SET path = ?1, name = ?2 WHERE path = ?3",
                params![new, new_name, old],
            )?;

            let children_pattern = format!("{}/\\%", escaped_old);
            let old_prefix_len = old.len() as i64;

            conn.execute(
                "UPDATE file_index \
                 SET path = ?1 || substr(path, ?2 + 1) \
                 WHERE path LIKE ?3 ESCAPE '\\'",
                params![&new, old_prefix_len, children_pattern],
            )?;

            // Mirror the rename in the content index (path is UNINDEXED,
            // so these updates do not touch the FTS terms).
            conn.execute(
                "UPDATE files_fts SET path = ?1 WHERE path = ?2",
                params![new, old],
            )?;
            conn.execute(
                "UPDATE files_fts \
                 SET path = ?1 || substr(path, ?2 + 1) \
                 WHERE path LIKE ?3 ESCAPE '\\'",
                params![&new, old_prefix_len, children_pattern],
            )?;

            Ok(())
        })
        .await?;

        Ok(())
    }

    async fn search(&self, query: SearchQuery) -> anyhow::Result<SearchResults> {
        let limit = query.limit.min(200);
        let offset = query.offset;
        let q_text = query.q.clone();

        let escaped_q = escape_like(&query.q);
        let fts_expr = fts5_match_expr(&query.q);

        // A whitespace-only query has no terms to match: content scope
        // matches nothing, and `both` degrades to a name search.
        let scope = match (query.scope, &fts_expr) {
            (SearchScope::Content, None) => {
                return Ok(SearchResults {
                    results: Vec::new(),
                    total: 0,
                    query: q_text,
                });
            }
            (SearchScope::Both, None) => SearchScope::Names,
            (s, _) => s,
        };

        let results = crate::db::interact(&self.db, move |conn| {
            let mut conditions: Vec<String> = Vec::new();
            let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

            // ?1: query text for the name LIKE filter and prefix-match
            // ordering. Always bound; unused (but harmless) in content scope.
            param_values.push(Box::new(escaped_q.clone()));

            let mut next_param = 2u32;

            // Content matches come from a subquery against FTS5, joined on
            // path: INNER JOIN for content scope (FTS is the match), LEFT
            // JOIN for both (name OR content). The count variant skips the
            // snippet computation.
            let (select_join, count_join, snippet_col) = if scope == SearchScope::Names {
                (String::new(), String::new(), "NULL".to_string())
            } else {
                let fts_idx = next_param;
                param_values.push(Box::new(fts_expr.clone().unwrap_or_default()));
                next_param += 1;
                let join_kind = if scope == SearchScope::Content {
                    "JOIN"
                } else {
                    "LEFT JOIN"
                };
                (
                    format!(
                        "{join_kind} (SELECT path, snippet(files_fts, 1, \
                         '{SNIPPET_START}', '{SNIPPET_END}', '…', {SNIPPET_TOKENS}) AS snip, \
                         rank FROM files_fts WHERE files_fts MATCH ?{fts_idx}) s \
                         ON s.path = fi.path"
                    ),
                    format!(
                        "{join_kind} (SELECT path FROM files_fts \
                         WHERE files_fts MATCH ?{fts_idx}) s ON s.path = fi.path"
                    ),
                    "s.snip".to_string(),
                )
            };

            match scope {
                SearchScope::Names => conditions
                    .push("fi.name LIKE ('%' || ?1 || '%') ESCAPE '\\' COLLATE NOCASE".to_string()),
                // The inner join against the MATCH subquery is the filter.
                SearchScope::Content => {}
                SearchScope::Both => conditions.push(
                    "(fi.name LIKE ('%' || ?1 || '%') ESCAPE '\\' COLLATE NOCASE \
                     OR s.path IS NOT NULL)"
                        .to_string(),
                ),
            }

            if let Some(ref ft) = query.file_type {
                match ft {
                    FileType::Image => {
                        conditions.push("mime_type LIKE 'image/%'".to_string());
                    }
                    FileType::Video => {
                        conditions.push("mime_type LIKE 'video/%'".to_string());
                    }
                    FileType::Audio => {
                        conditions.push("mime_type LIKE 'audio/%'".to_string());
                    }
                    FileType::Document => {
                        conditions.push(
                            "extension IN (\
                             'pdf','doc','docx','xls','xlsx','ppt','pptx',\
                             'txt','md','csv','json','xml','yaml','yml','toml')"
                                .to_string(),
                        );
                    }
                    FileType::File => {
                        conditions.push("is_dir = 0".to_string());
                    }
                    FileType::Dir => {
                        conditions.push("is_dir = 1".to_string());
                    }
                }
            }

            if let Some(min) = query.min_size {
                conditions.push(format!("size >= ?{next_param}"));
                param_values.push(Box::new(min as i64));
                next_param += 1;
            }

            if let Some(max) = query.max_size {
                conditions.push(format!("size <= ?{next_param}"));
                param_values.push(Box::new(max as i64));
                next_param += 1;
            }

            if let Some(ref after) = query.after {
                conditions.push(format!("modified >= ?{next_param}"));
                param_values.push(Box::new(after.clone()));
                next_param += 1;
            }

            if let Some(ref before) = query.before {
                conditions.push(format!("modified <= ?{next_param}"));
                param_values.push(Box::new(before.clone()));
                next_param += 1;
            }

            if let Some(ref path_scope) = query.path {
                conditions.push(format!("fi.path LIKE ?{next_param} ESCAPE '\\'"));
                param_values.push(Box::new(format!("{}/%", escape_like(path_scope))));
                next_param += 1;
            }

            let where_clause = if conditions.is_empty() {
                "1=1".to_string()
            } else {
                conditions.join(" AND ")
            };

            let count_sql =
                format!("SELECT COUNT(*) FROM file_index fi {count_join} WHERE {where_clause}");

            let params_refs: Vec<&dyn rusqlite::types::ToSql> =
                param_values.iter().map(|p| p.as_ref()).collect();

            let total: usize = conn.query_row(
                &count_sql,
                rusqlite::params_from_iter(params_refs.iter().copied()),
                |row| row.get::<_, i64>(0),
            )? as usize;

            let limit_param_idx = next_param;
            let offset_param_idx = next_param + 1;

            // Content scope orders by FTS5 relevance; name/both keep the
            // exact-then-prefix-then-substring name ordering (content-only
            // hits in `both` fall into the last bucket).
            let order_clause = if scope == SearchScope::Content {
                "s.rank, fi.name COLLATE NOCASE".to_string()
            } else {
                "CASE \
                   WHEN fi.name = ?1 COLLATE NOCASE THEN 0 \
                   WHEN fi.name LIKE ?1 || '%' COLLATE NOCASE THEN 1 \
                   ELSE 2 \
                 END, \
                 fi.name COLLATE NOCASE"
                    .to_string()
            };

            let select_sql = format!(
                "SELECT fi.path, fi.name, fi.is_dir, fi.size, fi.modified, \
                        fi.mime_type, fi.extension, {snippet_col} \
                 FROM file_index fi {select_join} \
                 WHERE {where_clause} \
                 ORDER BY {order_clause} \
                 LIMIT ?{limit_param_idx} OFFSET ?{offset_param_idx}"
            );

            param_values.push(Box::new(limit as i64));
            param_values.push(Box::new(offset as i64));

            let params_refs2: Vec<&dyn rusqlite::types::ToSql> =
                param_values.iter().map(|p| p.as_ref()).collect();

            let mut stmt = conn.prepare(&select_sql)?;
            let rows = stmt.query_map(
                rusqlite::params_from_iter(params_refs2.iter().copied()),
                |row| {
                    let path: String = row.get(0)?;
                    let name: String = row.get(1)?;
                    let is_dir: bool = row.get::<_, i32>(2)? != 0;
                    let size: u64 = row.get::<_, i64>(3)? as u64;
                    let modified_str: String = row.get(4)?;
                    let mime_type: Option<String> = row.get(5)?;
                    let extension: Option<String> = row.get(6)?;
                    let snippet: Option<String> = row.get(7)?;

                    let modified: DateTime<Utc> =
                        modified_str.parse::<DateTime<Utc>>().unwrap_or_default();

                    Ok(SearchHit {
                        entry: FileEntry {
                            name,
                            path,
                            is_dir,
                            size,
                            modified,
                            mime_type,
                            extension,
                        },
                        snippet,
                    })
                },
            )?;

            let results: Vec<SearchHit> = rows.collect::<Result<Vec<_>, _>>()?;

            Ok(SearchResults {
                results,
                total,
                query: q_text,
            })
        })
        .await?;

        Ok(results)
    }
}

/// Defensive cap on recursion depth; symlinks are skipped, so this only
/// guards against pathologically deep directory trees.
const MAX_WALK_DEPTH: usize = 64;

fn walk_tree(root: &Path) -> anyhow::Result<Vec<IndexEntry>> {
    let mut entries = Vec::new();
    walk_dir_recursive(root, root, &mut entries, 0)?;
    Ok(entries)
}

fn walk_dir_recursive(
    root: &Path,
    dir: &Path,
    entries: &mut Vec<IndexEntry>,
    depth: usize,
) -> anyhow::Result<()> {
    if depth >= MAX_WALK_DEPTH {
        tracing::warn!(
            "Skipping {}: max walk depth ({MAX_WALK_DEPTH}) reached",
            dir.display()
        );
        return Ok(());
    }

    let read_dir = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(e) => {
            tracing::warn!("Skipping unreadable directory {}: {e}", dir.display());
            return Ok(());
        }
    };

    for entry in read_dir {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!("Skipping directory entry: {e}");
                continue;
            }
        };

        let path = entry.path();

        // file_type() never follows symlinks; skip them entirely so symlink
        // cycles cannot recurse forever and out-of-root targets are never
        // indexed.
        match entry.file_type() {
            Ok(ft) if ft.is_symlink() => continue,
            Ok(_) => {}
            Err(e) => {
                tracing::warn!("Skipping {}: {e}", path.display());
                continue;
            }
        }

        let metadata = match entry.metadata() {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!("Skipping {}: {e}", path.display());
                continue;
            }
        };

        let is_dir = metadata.is_dir();
        let size = if is_dir { 0 } else { metadata.len() };

        let modified: DateTime<Utc> = metadata
            .modified()
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
            .into();

        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();

        let rel_path = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .into_owned();

        let extension = if is_dir {
            None
        } else {
            path.extension()
                .map(|e| e.to_string_lossy().into_owned().to_ascii_lowercase())
        };

        let mime_type = if is_dir {
            None
        } else {
            mime_guess::from_path(&path).first().map(|m| m.to_string())
        };

        let content = if is_dir {
            None
        } else {
            read_indexable_content(&path, size, extension.as_deref())
        };

        entries.push(IndexEntry {
            rel_path,
            name,
            is_dir,
            size,
            modified: modified.to_rfc3339(),
            mime_type,
            extension,
            content,
        });

        if is_dir {
            walk_dir_recursive(root, &path, entries, depth + 1)?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Guard: the bundled SQLite build must include the FTS5 extension that
    /// migration V5 and content search depend on.
    #[test]
    fn bundled_sqlite_has_fts5() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE VIRTUAL TABLE files_fts \
             USING fts5(path UNINDEXED, content, tokenize='unicode61');",
        )
        .expect("bundled SQLite lacks FTS5");

        conn.execute(
            "INSERT INTO files_fts (path, content) VALUES ('a.md', 'hello world')",
            [],
        )
        .unwrap();

        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM files_fts WHERE files_fts MATCH '\"hello\"'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(n, 1);
    }

    #[test]
    fn fts5_match_expr_quotes_terms() {
        assert_eq!(fts5_match_expr("hello"), Some("\"hello\"".to_string()));
        assert_eq!(
            fts5_match_expr("hello world"),
            Some("\"hello\" \"world\"".to_string())
        );
    }

    #[test]
    fn fts5_match_expr_neutralizes_syntax() {
        // Operators and globs become literal phrases.
        assert_eq!(
            fts5_match_expr("\"foo\" OR bar*"),
            Some("\"\"\"foo\"\"\" \"OR\" \"bar*\"".to_string())
        );
        // Unbalanced quotes are doubled, never left dangling.
        assert_eq!(
            fts5_match_expr("\"unbalanced"),
            Some("\"\"\"unbalanced\"".to_string())
        );
    }

    #[test]
    fn fts5_match_expr_empty_for_whitespace() {
        assert_eq!(fts5_match_expr(""), None);
        assert_eq!(fts5_match_expr("   \t"), None);
    }

    /// Every sanitized expression must parse as valid FTS5, no matter how
    /// hostile the raw input.
    #[test]
    fn fts5_match_expr_never_errors() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE VIRTUAL TABLE f USING fts5(content);")
            .unwrap();

        for raw in [
            "\"foo\" OR bar*",
            "\"unbalanced",
            "(paren AND",
            "NEAR(a b)",
            "col:value",
            "a-b c+d",
            "***",
            "\"\"\"",
        ] {
            let expr = fts5_match_expr(raw).unwrap();
            let result: Result<i64, _> =
                conn.query_row("SELECT COUNT(*) FROM f WHERE f MATCH ?1", [&expr], |row| {
                    row.get(0)
                });
            assert!(
                result.is_ok(),
                "raw query {raw:?} (expr {expr:?}) errored: {result:?}"
            );
        }
    }
}
