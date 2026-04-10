use std::collections::HashSet;
use std::path::{Path, PathBuf};

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

fn default_limit() -> u32 {
    DEFAULT_SEARCH_LIMIT
}

/// Escape SQLite LIKE metacharacters. Use with `ESCAPE '\'`.
fn escape_like(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

#[derive(Debug, Deserialize)]
pub struct SearchQuery {
    pub q: String,
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

#[derive(Debug, Serialize)]
pub struct SearchResults {
    pub results: Vec<FileEntry>,
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
    canonical_root: PathBuf,
}

struct IndexEntry {
    rel_path: String,
    name: String,
    is_dir: bool,
    size: u64,
    modified: String,
    mime_type: Option<String>,
    extension: Option<String>,
}

impl SearchIndexer {
    pub fn new(db: Pool, canonical_root: PathBuf) -> Self {
        Self { db, canonical_root }
    }
}

#[async_trait::async_trait]
impl SearchIndex for SearchIndexer {
    async fn full_reindex(&self) -> anyhow::Result<()> {
        let root = self.canonical_root.clone();

        let entries: Vec<IndexEntry> =
            tokio::task::spawn_blocking(move || walk_tree(&root)).await??;

        let all_paths: HashSet<String> = entries.iter().map(|e| e.rel_path.clone()).collect();
        let entry_count = entries.len();

        let pool = self.db.clone();
        let conn = pool
            .get()
            .await
            .map_err(|e| anyhow::anyhow!("pool error: {e}"))?;

        conn.interact(move |conn| {
            let tx = conn.transaction()?;

            const BATCH_INSERT_SIZE: usize = 500;

            for chunk in entries.chunks(BATCH_INSERT_SIZE) {
                for entry in chunk {
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
                }
            }

            {
                let mut stmt = tx.prepare("SELECT path FROM file_index")?;
                let db_paths: Vec<String> = stmt
                    .query_map([], |row| row.get::<_, String>(0))?
                    .filter_map(|r| r.ok())
                    .collect();
                drop(stmt);

                for db_path in &db_paths {
                    if !all_paths.contains(db_path) {
                        tx.execute(
                            "DELETE FROM file_index WHERE path = ?1",
                            params![db_path],
                        )?;
                    }
                }
            }

            tx.commit()?;
            Ok::<_, rusqlite::Error>(())
        })
        .await
        .map_err(|e| anyhow::anyhow!("interact error: {e}"))??;

        tracing::info!("Search index: full reindex complete ({entry_count} entries)");
        Ok(())
    }

    async fn upsert(&self, rel_path: &str) -> anyhow::Result<()> {
        let abs_path = self.canonical_root.join(rel_path);
        let rel_path = rel_path.to_string();

        let entry = tokio::task::spawn_blocking(move || -> anyhow::Result<Option<IndexEntry>> {
            let Ok(metadata) = std::fs::metadata(&abs_path) else {
                return Ok(None); // file was deleted
            };

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

            Ok(Some(IndexEntry {
                rel_path,
                name,
                is_dir,
                size,
                modified: modified.to_rfc3339(),
                mime_type,
                extension,
            }))
        })
        .await??;

        let Some(entry) = entry else {
            return Ok(());
        };

        let conn = self
            .db
            .get()
            .await
            .map_err(|e| anyhow::anyhow!("pool error: {e}"))?;

        conn.interact(move |conn| {
            conn.execute(
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
            Ok::<_, rusqlite::Error>(())
        })
        .await
        .map_err(|e| anyhow::anyhow!("interact error: {e}"))??;

        Ok(())
    }

    async fn remove(&self, rel_path: &str) -> anyhow::Result<()> {
        let rel_path = rel_path.to_string();
        let conn = self
            .db
            .get()
            .await
            .map_err(|e| anyhow::anyhow!("pool error: {e}"))?;

        conn.interact(move |conn| {
            conn.execute("DELETE FROM file_index WHERE path = ?1", params![rel_path])?;
            Ok::<_, rusqlite::Error>(())
        })
        .await
        .map_err(|e| anyhow::anyhow!("interact error: {e}"))??;

        Ok(())
    }

    async fn remove_prefix(&self, rel_prefix: &str) -> anyhow::Result<()> {
        let exact = rel_prefix.to_string();
        let like_pattern = format!("{}/\\%", escape_like(rel_prefix));
        let conn = self
            .db
            .get()
            .await
            .map_err(|e| anyhow::anyhow!("pool error: {e}"))?;

        conn.interact(move |conn| {
            conn.execute(
                "DELETE FROM file_index WHERE path = ?1 OR path LIKE ?2 ESCAPE '\\'",
                params![exact, like_pattern],
            )?;
            Ok::<_, rusqlite::Error>(())
        })
        .await
        .map_err(|e| anyhow::anyhow!("interact error: {e}"))??;

        Ok(())
    }

    async fn rename_prefix(&self, old: &str, new: &str) -> anyhow::Result<()> {
        let old = old.to_string();
        let new = new.to_string();
        let escaped_old = escape_like(&old);

        let conn = self
            .db
            .get()
            .await
            .map_err(|e| anyhow::anyhow!("pool error: {e}"))?;

        conn.interact(move |conn| {
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

            Ok::<_, rusqlite::Error>(())
        })
        .await
        .map_err(|e| anyhow::anyhow!("interact error: {e}"))??;

        Ok(())
    }

    async fn search(&self, query: SearchQuery) -> anyhow::Result<SearchResults> {
        let conn = self
            .db
            .get()
            .await
            .map_err(|e| anyhow::anyhow!("pool error: {e}"))?;

        let limit = query.limit.min(200);
        let offset = query.offset;
        let q_text = query.q.clone();

        let escaped_q = escape_like(&query.q);

        conn.interact(move |conn| {
            let mut conditions: Vec<String> = Vec::new();
            let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

            param_values.push(Box::new(escaped_q.clone()));
            conditions.push("name LIKE ('%' || ?1 || '%') ESCAPE '\\' COLLATE NOCASE".to_string());

            let mut next_param = 2u32;

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

            if let Some(ref scope) = query.path {
                conditions.push(format!("path LIKE ?{next_param} ESCAPE '\\'"));
                param_values.push(Box::new(format!("{}/%", escape_like(scope))));
                next_param += 1;
            }

            let where_clause = conditions.join(" AND ");

            let count_sql = format!("SELECT COUNT(*) FROM file_index WHERE {where_clause}");

            let params_refs: Vec<&dyn rusqlite::types::ToSql> =
                param_values.iter().map(|p| p.as_ref()).collect();

            let total: usize = conn.query_row(
                &count_sql,
                rusqlite::params_from_iter(params_refs.iter().copied()),
                |row| row.get::<_, i64>(0),
            )? as usize;

            let limit_param_idx = next_param;
            let offset_param_idx = next_param + 1;

            let select_sql = format!(
                "SELECT path, name, is_dir, size, modified, mime_type, extension \
                 FROM file_index \
                 WHERE {where_clause} \
                 ORDER BY \
                   CASE \
                     WHEN name = ?1 COLLATE NOCASE THEN 0 \
                     WHEN name LIKE ?1 || '%' COLLATE NOCASE THEN 1 \
                     ELSE 2 \
                   END, \
                   name COLLATE NOCASE \
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

                    let modified: DateTime<Utc> =
                        modified_str.parse::<DateTime<Utc>>().unwrap_or_default();

                    Ok(FileEntry {
                        name,
                        path,
                        is_dir,
                        size,
                        modified,
                        mime_type,
                        extension,
                    })
                },
            )?;

            let results: Vec<FileEntry> = rows.collect::<Result<Vec<_>, _>>()?;

            Ok::<_, rusqlite::Error>(SearchResults {
                results,
                total,
                query: q_text,
            })
        })
        .await
        .map_err(|e| anyhow::anyhow!("interact error: {e}"))?
        .map_err(|e: rusqlite::Error| anyhow::anyhow!("search query error: {e}"))
    }
}

fn walk_tree(root: &Path) -> anyhow::Result<Vec<IndexEntry>> {
    let mut entries = Vec::new();
    walk_dir_recursive(root, root, &mut entries)?;
    Ok(entries)
}

fn walk_dir_recursive(
    root: &Path,
    dir: &Path,
    entries: &mut Vec<IndexEntry>,
) -> anyhow::Result<()> {
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

        entries.push(IndexEntry {
            rel_path,
            name,
            is_dir,
            size,
            modified: modified.to_rfc3339(),
            mime_type,
            extension,
        });

        if is_dir {
            walk_dir_recursive(root, &path, entries)?;
        }
    }

    Ok(())
}
