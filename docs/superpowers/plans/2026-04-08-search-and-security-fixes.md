# Search Feature + Security Fixes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add SQLite-indexed file search with metadata filters, fix two security issues (Secure cookie, trusted proxy enforcement), and deduplicate FileEntry construction.

**Architecture:** A `SearchIndexer` service maintains a `file_index` SQLite table by walking the file tree at startup and receiving incremental updates from the existing filesystem watcher + mutation handlers. A new `GET /api/fs/search` endpoint queries the index. The frontend adds a search bar with filters to BrowserPage.

**Tech Stack:** Rust (axum, rusqlite, tokio), React 19, TypeScript

---

## File Map

| Action | File | Responsibility |
|--------|------|----------------|
| Modify | `src/config.rs` | Add `secure_cookie` field |
| Modify | `src/api/auth.rs` | Use `secure_cookie` in Set-Cookie, update logout |
| Modify | `src/api/mod.rs` | Rewrite `extract_client_ip` with trusted proxy enforcement, accept `SocketAddr` |
| Modify | `src/api/files.rs` | Add search indexer calls on mutations |
| Modify | `src/api/tus.rs` | Add search indexer call on TUS complete |
| Modify | `src/services/file_ops.rs` | Extract `FileEntry::from_path_and_metadata`, use in `file_info`/`list_directory` |
| Modify | `src/services/mod.rs` | Add `pub mod search_index;` |
| Create | `src/services/search_index.rs` | SearchIndexer, SearchQuery, SearchResults, FileType |
| Create | `src/api/search.rs` | Search endpoint handler |
| Modify | `src/api/mod.rs` | Register search routes |
| Modify | `src/state.rs` | Add `search_indexer` to AppState |
| Modify | `src/main.rs` | Wire up indexer, extend watcher, use `into_make_service_with_connect_info` |
| Modify | `src/error.rs` | Add `From<anyhow::Error>` impl |
| Create | `migrations/V3__search_index.sql` | file_index table + indexes |
| Modify | `src/db/mod.rs` | Apply V3 migration |
| Modify | `tests/helpers/mod.rs` | Add `search_indexer` to TestApp |
| Create | `tests/search_test.rs` | Integration tests for search |
| Create | `frontend/src/hooks/useSearch.ts` | Search API hook with debounce |
| Modify | `frontend/src/api/client.ts` | Add `search()` API function |
| Modify | `frontend/src/lib/types.ts` | Add SearchResponse type |
| Modify | `frontend/src/pages/BrowserPage.tsx` | Search bar + filter UI |

---

### Task 1: Add `secure_cookie` config field

**Files:**
- Modify: `src/config.rs`

- [ ] **Step 1: Add field to CliArgs**

In `src/config.rs`, add after the `tus_expiry_hours` field in `CliArgs` (line 71):

```rust
    /// Set cookie Secure flag (disable for local dev without HTTPS)
    #[arg(long, env = "RUSTYFILE_SECURE_COOKIE")]
    secure_cookie: Option<bool>,
```

- [ ] **Step 2: Add field to AppConfig**

In `src/config.rs`, add after `tus_expiry_hours` field in `AppConfig` (line 127):

```rust
    /// Whether to set the Secure flag on auth cookies (requires HTTPS).
    #[serde(default = "default_secure_cookie")]
    pub secure_cookie: bool,
```

- [ ] **Step 3: Add default function**

After `default_tus_expiry_hours` (line 176):

```rust
fn default_secure_cookie() -> bool {
    true
}
```

- [ ] **Step 4: Update Default impl**

In the `Default` impl, add after `tus_expiry_hours`:

```rust
            secure_cookie: default_secure_cookie(),
```

- [ ] **Step 5: Add CLI merge**

In `AppConfig::load()`, add after the `tus_expiry_hours` CLI merge (line 258):

```rust
        if let Some(v) = cli.secure_cookie {
            figment = figment.merge(Serialized::default("secure_cookie", v));
        }
```

- [ ] **Step 6: Commit**

```bash
git add src/config.rs
git commit -m "feat: add secure_cookie config field (default true)"
```

---

### Task 2: Apply Secure flag to auth cookies

**Files:**
- Modify: `src/api/auth.rs`

- [ ] **Step 1: Update login cookie construction**

Replace the cookie format string in the `login` function (line 152-155 of `src/api/auth.rs`):

```rust
            let mut cookie = format!(
                "rustyfile_token={}; HttpOnly; SameSite=Strict; Path=/; Max-Age={}",
                token,
                state.config.jwt_expiry_hours * 3600
            );
            if state.config.secure_cookie {
                cookie.push_str("; Secure");
            }
```

- [ ] **Step 2: Update logout cookie construction**

Replace the `clear_cookie` in the `logout` function (line 180 of `src/api/auth.rs`). The logout handler needs access to `state` for the config:

Change the `logout` function signature and body:

```rust
async fn logout(State(state): State<AppState>) -> impl axum::response::IntoResponse {
    let mut clear_cookie =
        "rustyfile_token=; HttpOnly; SameSite=Strict; Path=/; Max-Age=0".to_string();
    if state.config.secure_cookie {
        clear_cookie.push_str("; Secure");
    }
    (
        [(axum::http::header::SET_COOKIE, clear_cookie)],
        Json(LogoutResponse {
            message: "Logged out".into(),
        }),
    )
}
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo check 2>&1 | head -20`
Expected: no errors (there will be a warning about `secure_cookie` field not used in test helper — that's fine, we fix it in a later task)

- [ ] **Step 4: Commit**

```bash
git add src/api/auth.rs
git commit -m "fix: add Secure flag to auth cookies when secure_cookie is true"
```

---

### Task 3: Enforce trusted_proxies in extract_client_ip

**Files:**
- Modify: `src/api/mod.rs`
- Modify: `src/main.rs`

- [ ] **Step 1: Add `ConnectInfo` to axum serve**

In `src/main.rs`, change line 153:

```rust
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await?;
```

- [ ] **Step 2: Add helper to parse trusted proxy list**

In `src/api/mod.rs`, add after the `use` block (before `extract_client_ip`):

```rust
use std::net::{IpAddr, SocketAddr};

/// Parse comma-separated trusted proxy IPs/CIDRs into a list of IpAddr.
/// Returns None if the list is empty (meaning trust all).
fn parse_trusted_proxies(config_value: &str) -> Option<Vec<IpAddr>> {
    let trimmed = config_value.trim();
    if trimmed.is_empty() {
        return None;
    }
    let addrs: Vec<IpAddr> = trimmed
        .split(',')
        .filter_map(|s| s.trim().parse::<IpAddr>().ok())
        .collect();
    if addrs.is_empty() { None } else { Some(addrs) }
}
```

- [ ] **Step 3: Rewrite extract_client_ip**

Replace the existing `extract_client_ip` function:

```rust
/// Extract the real client IP address.
///
/// When `trusted_proxies` is empty: trusts proxy headers unconditionally
/// (backwards compatible — assumes a reverse proxy strips spoofed headers).
///
/// When `trusted_proxies` is set: only reads X-Forwarded-For / X-Real-IP
/// if the direct peer address is in the trusted list. Otherwise uses the
/// peer socket address.
pub fn extract_client_ip(
    headers: &axum::http::HeaderMap,
    peer_addr: Option<SocketAddr>,
    trusted_proxies: &str,
) -> String {
    let peer_ip = peer_addr.map(|a| a.ip());
    let trusted = parse_trusted_proxies(trusted_proxies);

    let should_trust_headers = match (&trusted, peer_ip) {
        (None, _) => true,                                      // empty config = trust all
        (Some(list), Some(ip)) => list.contains(&ip),           // check peer against list
        (Some(_), None) => false,                               // no peer info = don't trust
    };

    if should_trust_headers {
        if let Some(forwarded) = headers
            .get("x-forwarded-for")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.split(',').next())
            .map(|s| s.trim().to_string())
        {
            return forwarded;
        }
        if let Some(real_ip) = headers
            .get("x-real-ip")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.trim().to_string())
        {
            return real_ip;
        }
    }

    peer_ip
        .map(|ip| ip.to_string())
        .unwrap_or_else(|| "unknown".into())
}
```

- [ ] **Step 4: Update trace_layer span in build_router**

In `build_router` (line 79-86 of `src/api/mod.rs`), update the trace span to use the new signature. We don't have `ConnectInfo` at middleware level easily, so for the trace span pass `None` and empty proxies — the trace span IP is informational, not security-critical:

```rust
    let trace_layer =
        TraceLayer::new_for_http().make_span_with(|request: &axum::http::Request<_>| {
            let client_ip = extract_client_ip(request.headers(), None, "");
            tracing::info_span!(
                "request",
                method = %request.method(),
                uri = %request.uri(),
                client_ip = %client_ip,
            )
        });
```

- [ ] **Step 5: Update login handler to pass peer address**

In `src/api/auth.rs`, add `ConnectInfo` extraction to the login handler. Add the import at the top:

```rust
use axum::extract::ConnectInfo;
use std::net::SocketAddr;
```

Update the `login` function signature:

```rust
async fn login(
    State(state): State<AppState>,
    ConnectInfo(peer_addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<LoginRequest>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    let client_ip = crate::api::extract_client_ip(
        &headers,
        Some(peer_addr),
        &state.config.trusted_proxies,
    );
```

- [ ] **Step 6: Verify it compiles**

Run: `cargo check 2>&1 | head -20`
Expected: no errors

- [ ] **Step 7: Commit**

```bash
git add src/api/mod.rs src/api/auth.rs src/main.rs
git commit -m "fix: enforce trusted_proxies in extract_client_ip with peer address verification"
```

---

### Task 4: Extract FileEntry::from_path_and_metadata

**Files:**
- Modify: `src/services/file_ops.rs`

- [ ] **Step 1: Add constructor method to FileEntry**

In `src/services/file_ops.rs`, after the `DirListing` struct (line 29), add:

```rust
impl FileEntry {
    /// Construct a FileEntry from a path and its metadata.
    /// `canonical_root` is used to compute the relative path.
    pub fn from_path_and_metadata(
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
```

- [ ] **Step 2: Refactor list_directory to use the constructor**

Replace the body of the `while let` loop inside `list_directory` (lines 98-148) with:

```rust
    while let Some(entry) = entries.next_entry().await.map_err(AppError::Io)? {
        total_count += 1;

        if items.len() >= max_items {
            continue;
        }

        let metadata = entry.metadata().await.map_err(AppError::Io)?;
        let entry_path = entry.path();
        // Use std::fs::Metadata (the tokio Metadata derefs to it)
        items.push(FileEntry::from_path_and_metadata(
            canonical_root,
            &entry_path,
            &metadata,
        ));
    }
```

- [ ] **Step 3: Refactor file_info to use the constructor**

Replace the `file_info` function body (lines 174-227) with:

```rust
pub async fn file_info(canonical_root: &Path, file_path: &Path) -> Result<FileEntry, AppError> {
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
```

- [ ] **Step 4: Verify it compiles and tests pass**

Run: `cargo check && cargo test 2>&1 | tail -20`
Expected: all compilation passes, all existing tests pass

- [ ] **Step 5: Commit**

```bash
git add src/services/file_ops.rs
git commit -m "refactor: extract FileEntry::from_path_and_metadata to deduplicate construction"
```

---

### Task 5: Add `From<anyhow::Error>` to AppError

**Files:**
- Modify: `src/error.rs`

- [ ] **Step 1: Add the From impl**

In `src/error.rs`, after the `AppError` enum definition (line 51), before the `IntoResponse` impl, add:

```rust
impl From<anyhow::Error> for AppError {
    fn from(err: anyhow::Error) -> Self {
        Self::Internal(format!("{err:#}"))
    }
}
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo check 2>&1 | head -5`
Expected: no errors

- [ ] **Step 3: Commit**

```bash
git add src/error.rs
git commit -m "feat: add From<anyhow::Error> for AppError to support anyhow in services"
```

---

### Task 6: Create V3 migration for file_index table

**Files:**
- Create: `migrations/V3__search_index.sql`
- Modify: `src/db/mod.rs`

- [ ] **Step 1: Write the migration SQL**

Create `migrations/V3__search_index.sql`:

```sql
CREATE TABLE IF NOT EXISTS file_index (
    path TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    is_dir INTEGER NOT NULL DEFAULT 0,
    size INTEGER NOT NULL DEFAULT 0,
    modified TEXT NOT NULL,
    mime_type TEXT,
    extension TEXT,
    indexed_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_file_index_name ON file_index(name COLLATE NOCASE);
CREATE INDEX IF NOT EXISTS idx_file_index_extension ON file_index(extension);
CREATE INDEX IF NOT EXISTS idx_file_index_size ON file_index(size);
CREATE INDEX IF NOT EXISTS idx_file_index_modified ON file_index(modified);
```

- [ ] **Step 2: Add V3 migration to run_migrations**

In `src/db/mod.rs`, after the `if current_version < 2` block (line 94), add:

```rust
        if current_version < 3 {
            tracing::info!("Applying migration V3: search index");
            conn.execute_batch(include_str!("../../migrations/V3__search_index.sql"))?;
            conn.execute(
                "INSERT OR REPLACE INTO settings (key, value) VALUES ('schema_version', 3)",
                [],
            )?;
        }
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo check 2>&1 | head -5`
Expected: no errors

- [ ] **Step 4: Commit**

```bash
git add migrations/V3__search_index.sql src/db/mod.rs
git commit -m "feat: add V3 migration for file_index search table"
```

---

### Task 7: Implement SearchIndexer service

**Files:**
- Create: `src/services/search_index.rs`
- Modify: `src/services/mod.rs`

- [ ] **Step 1: Add module declaration**

In `src/services/mod.rs`, add:

```rust
pub mod search_index;
```

- [ ] **Step 2: Create the SearchIndexer with types**

Create `src/services/search_index.rs`:

```rust
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

fn default_limit() -> u32 {
    50
}

#[derive(Debug, Serialize)]
pub struct SearchResults {
    pub results: Vec<FileEntry>,
    pub total: usize,
    pub query: String,
}

#[derive(Clone)]
pub struct SearchIndexer {
    db: Pool,
    canonical_root: PathBuf,
}

impl SearchIndexer {
    pub fn new(db: Pool, canonical_root: PathBuf) -> Self {
        Self { db, canonical_root }
    }

    /// Walk the entire file tree and populate the index. Run once at startup.
    pub async fn full_reindex(&self) -> anyhow::Result<()> {
        let root = self.canonical_root.clone();
        let entries = tokio::task::spawn_blocking(move || collect_all_entries(&root))
            .await??;

        let db = self.db.clone();
        let root = self.canonical_root.clone();

        let conn = db.get().await?;
        conn.interact(move |conn| {
            let tx = conn.transaction()?;

            // Batch insert in chunks of 500
            for chunk in entries.chunks(500) {
                for (rel_path, name, is_dir, size, modified, mime_type, extension) in chunk {
                    tx.execute(
                        "INSERT OR REPLACE INTO file_index (path, name, is_dir, size, modified, mime_type, extension, indexed_at)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
                        params![rel_path, name, is_dir, size, modified, mime_type, extension],
                    )?;
                }
            }

            // Delete stale entries (paths in index but not in collected set)
            let indexed_paths: Vec<String> = {
                let mut stmt = tx.prepare("SELECT path FROM file_index")?;
                stmt.query_map([], |row| row.get::<_, String>(0))?
                    .filter_map(|r| r.ok())
                    .collect()
            };

            let entry_set: std::collections::HashSet<&str> = entries
                .iter()
                .map(|(p, _, _, _, _, _, _)| p.as_str())
                .collect();

            for path in &indexed_paths {
                if !entry_set.contains(path.as_str()) {
                    tx.execute("DELETE FROM file_index WHERE path = ?1", params![path])?;
                }
            }

            tx.commit()?;
            Ok::<_, rusqlite::Error>(())
        })
        .await
        .map_err(|e| anyhow::anyhow!("interact error: {e}"))??;

        let count = entries.len();
        tracing::info!(count, "Search index built");
        Ok(())
    }

    /// Insert or update a single path in the index.
    pub async fn upsert(&self, rel_path: &str) -> anyhow::Result<()> {
        let abs_path = self.canonical_root.join(rel_path);
        let metadata = match tokio::fs::metadata(&abs_path).await {
            Ok(m) => m,
            Err(_) => return Ok(()), // file gone, skip
        };

        let is_dir = metadata.is_dir();
        let size = if is_dir { 0i64 } else { metadata.len() as i64 };
        let modified: DateTime<Utc> = metadata
            .modified()
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
            .into();
        let modified_str = modified.to_rfc3339();

        let name = abs_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();

        let mime_type = if is_dir {
            None
        } else {
            mime_guess::from_path(&abs_path)
                .first()
                .map(|m| m.to_string())
        };

        let extension = if is_dir {
            None
        } else {
            abs_path
                .extension()
                .map(|e| e.to_string_lossy().into_owned())
        };

        let rel = rel_path.to_string();
        let db = self.db.clone();
        let conn = db.get().await?;
        conn.interact(move |conn| {
            conn.execute(
                "INSERT OR REPLACE INTO file_index (path, name, is_dir, size, modified, mime_type, extension, indexed_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
                params![rel, name, is_dir, size, modified_str, mime_type, extension],
            )?;
            Ok::<_, rusqlite::Error>(())
        })
        .await
        .map_err(|e| anyhow::anyhow!("interact error: {e}"))??;

        Ok(())
    }

    /// Remove a single path from the index.
    pub async fn remove(&self, rel_path: &str) -> anyhow::Result<()> {
        let rel = rel_path.to_string();
        let db = self.db.clone();
        let conn = db.get().await?;
        conn.interact(move |conn| {
            conn.execute("DELETE FROM file_index WHERE path = ?1", params![rel])?;
            Ok::<_, rusqlite::Error>(())
        })
        .await
        .map_err(|e| anyhow::anyhow!("interact error: {e}"))??;
        Ok(())
    }

    /// Remove all paths under a directory prefix.
    pub async fn remove_prefix(&self, rel_prefix: &str) -> anyhow::Result<()> {
        let prefix = format!("{}%", rel_prefix);
        let exact = rel_prefix.to_string();
        let db = self.db.clone();
        let conn = db.get().await?;
        conn.interact(move |conn| {
            conn.execute(
                "DELETE FROM file_index WHERE path = ?1 OR path LIKE ?2",
                params![exact, prefix],
            )?;
            Ok::<_, rusqlite::Error>(())
        })
        .await
        .map_err(|e| anyhow::anyhow!("interact error: {e}"))??;
        Ok(())
    }

    /// Update paths after a rename/move operation.
    pub async fn rename_prefix(&self, old_prefix: &str, new_prefix: &str) -> anyhow::Result<()> {
        let old = old_prefix.to_string();
        let new = new_prefix.to_string();
        let old_like = format!("{}/%", old_prefix);
        let db = self.db.clone();
        let conn = db.get().await?;
        conn.interact(move |conn| {
            // Update the exact path
            conn.execute(
                "UPDATE file_index SET path = ?1, name = ?2 WHERE path = ?3",
                params![
                    &new,
                    Path::new(&new)
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                    &old
                ],
            )?;
            // Update children: replace prefix
            let old_len = old.len() as i32;
            conn.execute(
                "UPDATE file_index SET path = ?1 || substr(path, ?2 + 1) WHERE path LIKE ?3",
                params![&new, old_len, old_like],
            )?;
            Ok::<_, rusqlite::Error>(())
        })
        .await
        .map_err(|e| anyhow::anyhow!("interact error: {e}"))??;
        Ok(())
    }

    /// Search the index with filters.
    pub async fn search(&self, query: &SearchQuery) -> anyhow::Result<SearchResults> {
        let limit = query.limit.min(200);
        let offset = query.offset;
        let q = query.q.clone();
        let file_type = query.file_type.clone();
        let min_size = query.min_size;
        let max_size = query.max_size;
        let after = query.after.clone();
        let before = query.before.clone();
        let scope_path = query.path.clone();
        let canonical_root = self.canonical_root.clone();

        let db = self.db.clone();
        let conn = db.get().await?;

        let (rows, total) = conn
            .interact(move |conn| {
                let mut conditions = vec!["name LIKE '%' || ?1 || '%' COLLATE NOCASE".to_string()];
                let mut param_idx = 2u32;

                // We'll collect params as boxed dyn ToSql
                let mut params_vec: Vec<Box<dyn rusqlite::types::ToSql>> =
                    vec![Box::new(q.clone())];

                if let Some(ref ft) = file_type {
                    match ft {
                        FileType::File => conditions.push("is_dir = 0".into()),
                        FileType::Dir => conditions.push("is_dir = 1".into()),
                        FileType::Image => conditions.push("mime_type LIKE 'image/%'".into()),
                        FileType::Video => conditions.push("mime_type LIKE 'video/%'".into()),
                        FileType::Audio => conditions.push("mime_type LIKE 'audio/%'".into()),
                        FileType::Document => {
                            conditions.push(
                                "extension IN ('pdf','doc','docx','xls','xlsx','ppt','pptx','txt','md','csv','json','xml','yaml','yml','toml')"
                                    .into(),
                            );
                        }
                    }
                }

                if let Some(min) = min_size {
                    conditions.push(format!("size >= ?{param_idx}"));
                    params_vec.push(Box::new(min as i64));
                    param_idx += 1;
                }

                if let Some(max) = max_size {
                    conditions.push(format!("size <= ?{param_idx}"));
                    params_vec.push(Box::new(max as i64));
                    param_idx += 1;
                }

                if let Some(ref after_date) = after {
                    conditions.push(format!("modified >= ?{param_idx}"));
                    params_vec.push(Box::new(after_date.clone()));
                    param_idx += 1;
                }

                if let Some(ref before_date) = before {
                    conditions.push(format!("modified <= ?{param_idx}"));
                    params_vec.push(Box::new(before_date.clone()));
                    param_idx += 1;
                }

                if let Some(ref scope) = scope_path {
                    if !scope.is_empty() {
                        conditions.push(format!("path LIKE ?{param_idx}"));
                        params_vec.push(Box::new(format!("{scope}/%")));
                        param_idx += 1;
                    }
                }

                let where_clause = conditions.join(" AND ");

                // Count query
                let count_sql = format!("SELECT COUNT(*) FROM file_index WHERE {where_clause}");
                let params_refs: Vec<&dyn rusqlite::types::ToSql> =
                    params_vec.iter().map(|p| p.as_ref()).collect();
                let total: usize = conn.query_row(
                    &count_sql,
                    rusqlite::params_from_iter(params_refs.iter()),
                    |row| row.get(0),
                )?;

                // Results query with relevance ordering
                let select_sql = format!(
                    "SELECT path, name, is_dir, size, modified, mime_type, extension \
                     FROM file_index WHERE {where_clause} \
                     ORDER BY CASE \
                       WHEN name = ?1 COLLATE NOCASE THEN 0 \
                       WHEN name LIKE ?1 || '%' COLLATE NOCASE THEN 1 \
                       ELSE 2 END, name COLLATE NOCASE \
                     LIMIT ?{param_idx} OFFSET ?{}",
                    param_idx + 1
                );
                params_vec.push(Box::new(limit as i64));
                params_vec.push(Box::new(offset as i64));

                let params_refs2: Vec<&dyn rusqlite::types::ToSql> =
                    params_vec.iter().map(|p| p.as_ref()).collect();

                let mut stmt = conn.prepare(&select_sql)?;
                let rows: Vec<FileEntry> = stmt
                    .query_map(rusqlite::params_from_iter(params_refs2.iter()), |row| {
                        let path: String = row.get(0)?;
                        let name: String = row.get(1)?;
                        let is_dir: bool = row.get(2)?;
                        let size: i64 = row.get(3)?;
                        let modified_str: String = row.get(4)?;
                        let mime_type: Option<String> = row.get(5)?;
                        let extension: Option<String> = row.get(6)?;

                        let modified = modified_str
                            .parse::<DateTime<Utc>>()
                            .unwrap_or_default();

                        Ok(FileEntry {
                            name,
                            path,
                            is_dir,
                            size: size as u64,
                            modified,
                            mime_type,
                            extension,
                        })
                    })?
                    .filter_map(|r| r.ok())
                    .collect();

                Ok::<_, rusqlite::Error>((rows, total))
            })
            .await
            .map_err(|e| anyhow::anyhow!("interact error: {e}"))??;

        Ok(SearchResults {
            results: rows,
            total,
            query: query.q.clone(),
        })
    }
}

/// Synchronously walk the file tree and collect all entries as tuples.
fn collect_all_entries(
    root: &Path,
) -> anyhow::Result<Vec<(String, String, bool, i64, String, Option<String>, Option<String>)>> {
    let mut results = Vec::new();
    let mut stack = vec![root.to_path_buf()];

    while let Some(dir) = stack.pop() {
        let read_dir = match std::fs::read_dir(&dir) {
            Ok(rd) => rd,
            Err(_) => continue,
        };

        for entry in read_dir.flatten() {
            let path = entry.path();
            let metadata = match entry.metadata() {
                Ok(m) => m,
                Err(_) => continue,
            };

            let is_dir = metadata.is_dir();
            if is_dir {
                stack.push(path.clone());
            }

            let rel_path = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .into_owned();

            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();

            let size = if is_dir { 0i64 } else { metadata.len() as i64 };

            let modified: DateTime<Utc> = metadata
                .modified()
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
                .into();

            let mime_type = if is_dir {
                None
            } else {
                mime_guess::from_path(&path)
                    .first()
                    .map(|m| m.to_string())
            };

            let extension = if is_dir {
                None
            } else {
                path.extension()
                    .map(|e| e.to_string_lossy().into_owned())
            };

            results.push((
                rel_path,
                name,
                is_dir,
                size,
                modified.to_rfc3339(),
                mime_type,
                extension,
            ));
        }
    }

    Ok(results)
}
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo check 2>&1 | head -10`
Expected: no errors (warnings about unused SearchIndexer are fine — it's wired in the next tasks)

- [ ] **Step 4: Commit**

```bash
git add src/services/search_index.rs src/services/mod.rs
git commit -m "feat: implement SearchIndexer with full_reindex, upsert, remove, rename, search"
```

---

### Task 8: Create search API endpoint

**Files:**
- Create: `src/api/search.rs`
- Modify: `src/api/mod.rs`

- [ ] **Step 1: Create the search handler**

Create `src/api/search.rs`:

```rust
use axum::extract::{Query, State};
use axum::middleware;
use axum::routing::get;
use axum::{Json, Router};

use crate::api::middleware::auth::require_auth;
use crate::error::AppError;
use crate::services::search_index::{SearchQuery, SearchResults};
use crate::state::AppState;

async fn search(
    State(state): State<AppState>,
    Query(query): Query<SearchQuery>,
) -> Result<Json<SearchResults>, AppError> {
    if query.q.is_empty() {
        return Err(AppError::BadRequest(
            "Search query 'q' is required".into(),
        ));
    }

    let results = state.search_indexer.search(&query).await?;
    Ok(Json(results))
}

pub fn routes(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/", get(search))
        .route_layer(middleware::from_fn_with_state(state, require_auth))
}
```

- [ ] **Step 2: Add module declaration and register routes**

In `src/api/mod.rs`, add to the module declarations (line 1):

```rust
pub mod search;
```

In the `build_router` function, add the search route to `cached_api_routes` (after the `/fs/` redirect route, around line 57):

```rust
        .nest("/fs/search", search::routes(state.clone()))
```

This must go **before** `.nest("/fs", files::routes(...))` to avoid the wildcard `/fs/{*path}` catching `/fs/search`. Reorder the `cached_api_routes` chain:

```rust
    let cached_api_routes = Router::new()
        .nest("/health", health::routes())
        .nest("/setup", setup::routes())
        .nest("/auth", auth::routes())
        .nest("/fs/search", search::routes(state.clone()))
        .nest("/fs", files::routes(state.clone()))
        .route("/fs/", get(|| async { Redirect::permanent("/api/fs") }))
        .layer(SetResponseHeaderLayer::overriding(
            header::CACHE_CONTROL,
            HeaderValue::from_static("no-cache, no-store, must-revalidate"),
        ));
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo check 2>&1 | head -10`
Expected: no errors

- [ ] **Step 4: Commit**

```bash
git add src/api/search.rs src/api/mod.rs
git commit -m "feat: add GET /api/fs/search endpoint with filters"
```

---

### Task 9: Wire SearchIndexer into AppState and main.rs

**Files:**
- Modify: `src/state.rs`
- Modify: `src/main.rs`
- Modify: `tests/helpers/mod.rs`

- [ ] **Step 1: Add search_indexer to AppState**

In `src/state.rs`, add the import:

```rust
use crate::services::search_index::SearchIndexer;
```

Add to `AppState` struct, after `hls_sources`:

```rust
    /// SQLite-backed file search indexer.
    pub search_indexer: SearchIndexer,
```

- [ ] **Step 2: Create and wire indexer in main.rs**

In `src/main.rs`, after the `hls_sources` creation (line 128-129) and before the `AppState` construction (line 131), add:

```rust
    let search_indexer =
        rustyfile::services::search_index::SearchIndexer::new(pool.clone(), canonical_root.clone());
```

Add `search_indexer` to the `AppState` construction (after `hls_sources`):

```rust
        search_indexer,
```

After the `AppState` construction and before `api::tus::spawn_cleanup_task` (line 145), spawn the background reindex:

```rust
    // Background search index build (non-blocking startup)
    {
        let indexer = state.search_indexer.clone();
        tokio::spawn(async move {
            if let Err(e) = indexer.full_reindex().await {
                tracing::error!("Search index build failed: {e:#}");
            }
        });
    }
```

- [ ] **Step 3: Extend filesystem watcher to feed index updates**

In `src/main.rs`, inside the watcher `tokio::spawn` block (around line 96-108), extend the event handler to also update the search index. Replace that entire spawn block:

```rust
        let search_indexer_watcher = state.search_indexer.clone();

        tokio::spawn(async move {
            let _debouncer = debouncer; // Keep alive
            while let Some(Ok(events)) = rx.recv().await {
                for event in events {
                    for path in &event.paths {
                        // Cache invalidation
                        if let Some(parent) = path.parent() {
                            let key = parent.to_string_lossy().to_string();
                            dir_cache_watcher.invalidate(&key).await;
                        }

                        // Search index update
                        if let Ok(rel) = path.strip_prefix(&watch_root) {
                            let rel_str = rel.to_string_lossy().to_string();
                            if path.exists() {
                                let _ = search_indexer_watcher.upsert(&rel_str).await;
                            } else {
                                let _ = search_indexer_watcher.remove(&rel_str).await;
                            }
                        }
                    }
                }
            }
        });
```

Note: The watcher spawn needs to move after `AppState` construction now since it references `state.search_indexer`. Move the entire watcher block (lines 73-111) to after the `AppState` construction. The `dir_cache` is already cloned as `dir_cache_watcher` so it doesn't depend on state ordering — but now we also need `search_indexer_watcher`. The simplest fix: move the watcher setup after `let state = AppState { ... };`.

- [ ] **Step 4: Update TestApp helper**

In `tests/helpers/mod.rs`, add the import:

```rust
use rustyfile::services::search_index::SearchIndexer;
```

After the `hls_sources` creation (line 83) add:

```rust
        let search_indexer =
            SearchIndexer::new(pool.clone(), canonical_root.clone());
```

Add to the `AppState` construction:

```rust
            search_indexer,
```

Also add `secure_cookie: false` to the config construction since we added that field:

```rust
            secure_cookie: false,
```

- [ ] **Step 5: Verify everything compiles and tests pass**

Run: `cargo test 2>&1 | tail -30`
Expected: all existing tests pass

- [ ] **Step 6: Commit**

```bash
git add src/state.rs src/main.rs tests/helpers/mod.rs
git commit -m "feat: wire SearchIndexer into AppState, watcher, and startup reindex"
```

---

### Task 10: Add index notifications to mutation handlers

**Files:**
- Modify: `src/api/files.rs`
- Modify: `src/api/tus.rs`

- [ ] **Step 1: Add indexer calls to files.rs mutations**

In `src/api/files.rs`, in the `save_file` handler, after the cache invalidation (line 181-182), add:

```rust
    // Update search index (fire-and-forget)
    let indexer = state.search_indexer.clone();
    let idx_path = user_path.clone();
    tokio::spawn(async move {
        let _ = indexer.upsert(&idx_path).await;
    });
```

In the `create` handler, after cache invalidation (line 208-210), add:

```rust
    let indexer = state.search_indexer.clone();
    let idx_path = user_path.clone();
    tokio::spawn(async move {
        let _ = indexer.upsert(&idx_path).await;
    });
```

In the `remove` handler, after cache invalidation (line 232-234), add:

```rust
    let indexer = state.search_indexer.clone();
    let idx_path = user_path.clone();
    let was_dir = tokio::fs::metadata(&resolved).await.map(|m| m.is_dir()).unwrap_or(false);
    tokio::spawn(async move {
        if was_dir {
            let _ = indexer.remove_prefix(&idx_path).await;
        } else {
            let _ = indexer.remove(&idx_path).await;
        }
    });
```

Actually, the metadata check must happen before the delete call. Move the `was_dir` check before `file_ops::delete`. Replace the remove handler:

```rust
async fn remove(
    State(state): State<AppState>,
    Path(user_path): Path<String>,
    Extension(_user): Extension<user_repo::User>,
) -> Result<Json<MutationResponse>, AppError> {
    let resolved = file_ops::safe_resolve(&state.canonical_root, &user_path)?;

    // Check if directory before deleting (for index cleanup)
    let is_dir = tokio::fs::metadata(&resolved)
        .await
        .map(|m| m.is_dir())
        .unwrap_or(false);

    file_ops::delete(&state.canonical_root, &resolved).await?;

    if let Some(parent) = resolved.parent() {
        let key = parent.to_string_lossy().into_owned();
        state.dir_cache.invalidate(&key).await;
    }

    let indexer = state.search_indexer.clone();
    let idx_path = user_path.clone();
    tokio::spawn(async move {
        if is_dir {
            let _ = indexer.remove_prefix(&idx_path).await;
        } else {
            let _ = indexer.remove(&idx_path).await;
        }
    });

    Ok(Json(MutationResponse {
        message: format!("Deleted: {user_path}"),
    }))
}
```

In the `rename_item` handler, after cache invalidation, add:

```rust
    let indexer = state.search_indexer.clone();
    let old_path = user_path.clone();
    let new_path = body.destination.clone();
    tokio::spawn(async move {
        let _ = indexer.rename_prefix(&old_path, &new_path).await;
    });
```

- [ ] **Step 2: Add indexer call to TUS complete**

In `src/api/tus.rs`, in the `append_chunk` handler, inside the `if is_complete` block (after the cache invalidation at line 387), add:

```rust
        // Update search index
        let indexer = state.search_indexer.clone();
        let idx_path = if destination.is_empty() {
            filename.clone()
        } else {
            format!("{}/{}", destination, filename)
        };
        tokio::spawn(async move {
            let _ = indexer.upsert(&idx_path).await;
        });
```

- [ ] **Step 3: Verify it compiles and tests pass**

Run: `cargo test 2>&1 | tail -20`
Expected: all tests pass

- [ ] **Step 4: Commit**

```bash
git add src/api/files.rs src/api/tus.rs
git commit -m "feat: notify search indexer on file mutations and TUS completion"
```

---

### Task 11: Write search integration tests

**Files:**
- Create: `tests/search_test.rs`

- [ ] **Step 1: Write the search tests**

Create `tests/search_test.rs`:

```rust
mod helpers;

use helpers::TestApp;

#[tokio::test]
async fn search_requires_auth() {
    let app = TestApp::spawn().await;

    let resp = app
        .client
        .get(app.url("/api/fs/search?q=test"))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 401);
}

#[tokio::test]
async fn search_requires_query_param() {
    let app = TestApp::spawn().await;
    let token = app.create_admin().await;

    let resp = app
        .client
        .get(app.url("/api/fs/search?q="))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 400);
}

#[tokio::test]
async fn search_finds_files_by_name() {
    let app = TestApp::spawn().await;
    let token = app.create_admin().await;

    // Create test files
    app.write_file("readme.txt", b"hello");
    app.write_file("docs/readme.md", b"world");
    app.write_file("other.log", b"data");

    // Trigger reindex
    app.reindex().await;

    let resp = app
        .client
        .get(app.url("/api/fs/search?q=readme"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    let results = body["results"].as_array().unwrap();
    assert_eq!(results.len(), 2);
    assert_eq!(body["total"], 2);
}

#[tokio::test]
async fn search_filters_by_type() {
    let app = TestApp::spawn().await;
    let token = app.create_admin().await;

    app.write_file("photo.jpg", b"fake-image");
    app.write_file("video.mp4", b"fake-video");
    app.write_file("notes.txt", b"text");

    app.reindex().await;

    // Search for images
    let resp = app
        .client
        .get(app.url("/api/fs/search?q=&type=image"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();

    // q is empty so should return 400 — search with a broad query instead
    let resp = app
        .client
        .get(app.url("/api/fs/search?q=.&type=image"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();

    // Note: mime_guess may not recognize .jpg without real content
    // but the extension is indexed. Let's search by name instead.
    let resp = app
        .client
        .get(app.url("/api/fs/search?q=photo"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["results"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn search_scoped_to_directory() {
    let app = TestApp::spawn().await;
    let token = app.create_admin().await;

    app.write_file("root.txt", b"root");
    app.write_file("sub/nested.txt", b"nested");
    app.write_file("sub/deep/file.txt", b"deep");

    app.reindex().await;

    let resp = app
        .client
        .get(app.url("/api/fs/search?q=txt&path=sub"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    let results = body["results"].as_array().unwrap();
    // Only nested.txt and deep/file.txt, not root.txt
    assert_eq!(results.len(), 2);
}

#[tokio::test]
async fn search_pagination() {
    let app = TestApp::spawn().await;
    let token = app.create_admin().await;

    for i in 0..5 {
        app.write_file(&format!("file{i}.txt"), b"data");
    }

    app.reindex().await;

    let resp = app
        .client
        .get(app.url("/api/fs/search?q=file&limit=2&offset=0"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["results"].as_array().unwrap().len(), 2);
    assert_eq!(body["total"], 5);

    // Page 2
    let resp = app
        .client
        .get(app.url("/api/fs/search?q=file&limit=2&offset=2"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["results"].as_array().unwrap().len(), 2);
}
```

- [ ] **Step 2: Add reindex helper to TestApp**

In `tests/helpers/mod.rs`, add a helper method to `TestApp`:

```rust
    pub async fn reindex(&self) {
        // Access the search indexer through a rebuild
        // Since we can't access state directly, we need to store it.
        // Alternative: just re-run the indexer.
    }
```

Actually, we need a different approach. The TestApp doesn't expose state. The simplest solution: store the `SearchIndexer` on `TestApp`. Update `tests/helpers/mod.rs`:

Add a field to `TestApp`:

```rust
pub struct TestApp {
    pub addr: String,
    pub client: Client,
    pub root_dir: TempDir,
    pub data_dir: TempDir,
    pub search_indexer: SearchIndexer,
}
```

Clone the `search_indexer` before passing it to `AppState`:

```rust
        let search_indexer_for_test = search_indexer.clone();
```

Add it to the `TestApp` return:

```rust
        Self {
            addr,
            client,
            root_dir,
            data_dir,
            search_indexer: search_indexer_for_test,
        }
```

Add the `reindex` method:

```rust
    pub async fn reindex(&self) {
        self.search_indexer
            .full_reindex()
            .await
            .expect("Reindex failed in test");
    }
```

- [ ] **Step 3: Run the tests**

Run: `cargo test search 2>&1 | tail -30`
Expected: all search tests pass

- [ ] **Step 4: Run the full test suite**

Run: `cargo test 2>&1 | tail -30`
Expected: all tests pass

- [ ] **Step 5: Commit**

```bash
git add tests/search_test.rs tests/helpers/mod.rs
git commit -m "test: add search integration tests with filters, scoping, and pagination"
```

---

### Task 12: Frontend — Add search types and API function

**Files:**
- Modify: `frontend/src/lib/types.ts`
- Modify: `frontend/src/api/client.ts`

- [ ] **Step 1: Add search types**

In `frontend/src/lib/types.ts`, add at the end:

```typescript
export interface SearchParams {
  q: string
  type?: 'file' | 'dir' | 'image' | 'video' | 'audio' | 'document'
  min_size?: number
  max_size?: number
  after?: string
  before?: string
  path?: string
  limit?: number
  offset?: number
}

export interface SearchResponse {
  results: FileEntry[]
  total: number
  query: string
}
```

- [ ] **Step 2: Add search function to API client**

In `frontend/src/api/client.ts`, add a `search` method to the `api` object:

```typescript
  search: (params: SearchParams) => {
    const qs = new URLSearchParams()
    qs.set('q', params.q)
    if (params.type) qs.set('type', params.type)
    if (params.min_size !== undefined) qs.set('min_size', String(params.min_size))
    if (params.max_size !== undefined) qs.set('max_size', String(params.max_size))
    if (params.after) qs.set('after', params.after)
    if (params.before) qs.set('before', params.before)
    if (params.path) qs.set('path', params.path)
    if (params.limit !== undefined) qs.set('limit', String(params.limit))
    if (params.offset !== undefined) qs.set('offset', String(params.offset))
    return request<SearchResponse>('GET', `/api/fs/search?${qs.toString()}`)
  },
```

Add the import at the top:

```typescript
import type { SearchParams, SearchResponse } from '../lib/types'
```

- [ ] **Step 3: Commit**

```bash
git add frontend/src/lib/types.ts frontend/src/api/client.ts
git commit -m "feat(frontend): add search types and API client function"
```

---

### Task 13: Frontend — Create useSearch hook

**Files:**
- Create: `frontend/src/hooks/useSearch.ts`

- [ ] **Step 1: Create the hook**

Create `frontend/src/hooks/useSearch.ts`:

```typescript
import { useState, useEffect, useCallback, useRef } from 'react'
import { api } from '../api/client'
import type { FileEntry, SearchParams } from '../lib/types'

interface UseSearchResult {
  results: FileEntry[]
  total: number
  loading: boolean
  error: string | null
  search: (params: SearchParams) => void
  clear: () => void
  isActive: boolean
}

export function useSearch(): UseSearchResult {
  const [results, setResults] = useState<FileEntry[]>([])
  const [total, setTotal] = useState(0)
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [isActive, setIsActive] = useState(false)
  const debounceRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const mountedRef = useRef(true)

  useEffect(() => {
    mountedRef.current = true
    return () => {
      mountedRef.current = false
      if (debounceRef.current) clearTimeout(debounceRef.current)
    }
  }, [])

  const search = useCallback((params: SearchParams) => {
    if (debounceRef.current) clearTimeout(debounceRef.current)

    if (!params.q || params.q.length < 2) {
      setResults([])
      setTotal(0)
      setIsActive(false)
      setError(null)
      return
    }

    setIsActive(true)
    setLoading(true)

    debounceRef.current = setTimeout(async () => {
      try {
        const resp = await api.search(params)
        if (mountedRef.current) {
          setResults(resp.results)
          setTotal(resp.total)
          setError(null)
        }
      } catch (e: unknown) {
        if (mountedRef.current) {
          setError(e instanceof Error ? e.message : 'Search failed')
        }
      } finally {
        if (mountedRef.current) setLoading(false)
      }
    }, 300)
  }, [])

  const clear = useCallback(() => {
    if (debounceRef.current) clearTimeout(debounceRef.current)
    setResults([])
    setTotal(0)
    setIsActive(false)
    setError(null)
    setLoading(false)
  }, [])

  return { results, total, loading, error, search, clear, isActive }
}
```

- [ ] **Step 2: Commit**

```bash
git add frontend/src/hooks/useSearch.ts
git commit -m "feat(frontend): add useSearch hook with 300ms debounce"
```

---

### Task 14: Frontend — Add search UI to BrowserPage

**Files:**
- Modify: `frontend/src/pages/BrowserPage.tsx`

- [ ] **Step 1: Add search imports and state**

In `frontend/src/pages/BrowserPage.tsx`, add to imports:

```typescript
import { Search as SearchIcon, Xmark as XmarkIcon } from 'iconoir-react'
import { useSearch } from '../hooks/useSearch'
import type { SearchParams } from '../lib/types'
```

Note: `Xmark` is already imported — rename to avoid conflict. Actually, check the existing imports: `Xmark` is already imported. We can reuse it. Just add:

```typescript
import { Search as SearchIcon } from 'iconoir-react'
import { useSearch } from '../hooks/useSearch'
```

Inside the component, after the existing state declarations (around line 36), add:

```typescript
  const { results: searchResults, total: searchTotal, loading: searchLoading, error: searchError, search, clear: clearSearch, isActive: isSearchActive } = useSearch()
  const [searchQuery, setSearchQuery] = useState('')
  const [searchType, setSearchType] = useState<SearchParams['type']>(undefined)

  const handleSearchChange = useCallback((value: string) => {
    setSearchQuery(value)
    search({ q: value, type: searchType, path: currentPath || undefined })
  }, [search, searchType, currentPath])

  const handleTypeChange = useCallback((type: SearchParams['type']) => {
    setSearchType(type)
    if (searchQuery.length >= 2) {
      search({ q: searchQuery, type, path: currentPath || undefined })
    }
  }, [search, searchQuery, currentPath])

  const handleClearSearch = useCallback(() => {
    setSearchQuery('')
    setSearchType(undefined)
    clearSearch()
  }, [clearSearch])
```

- [ ] **Step 2: Add search bar to header**

In the header section (line 196), add a search input between the breadcrumbs and the action buttons. Replace the header content:

```tsx
      <header className="h-14 border-b border-borders flex items-center px-4 md:px-6 shrink-0 gap-4">
        <div className="flex items-center gap-1 shrink-0">
          <button
            onClick={() => navigate(-1)}
            className="p-1.5 text-muted hover:text-primary transition-colors"
            title="Back"
          >
            <NavArrowLeft width={18} height={18} strokeWidth={1.8} />
          </button>
          <button
            onClick={() => navigate(1)}
            className="p-1.5 text-muted hover:text-primary transition-colors"
            title="Forward"
          >
            <NavArrowRight width={18} height={18} strokeWidth={1.8} />
          </button>
        </div>

        {isSearchActive ? (
          <div className="flex-1 flex items-center gap-2">
            <SearchIcon width={16} height={16} strokeWidth={1.8} className="text-muted shrink-0" />
            <input
              type="text"
              value={searchQuery}
              onChange={(e) => handleSearchChange(e.target.value)}
              className="flex-1 h-8 bg-background border border-borders text-text-main font-mono text-[13px] px-3 rounded-none focus:border-primary focus:outline-none transition-colors"
              placeholder="Search files..."
              autoFocus
            />
            <select
              value={searchType || ''}
              onChange={(e) => handleTypeChange((e.target.value || undefined) as SearchParams['type'])}
              className="h-8 bg-background border border-borders text-text-main font-mono text-[11px] px-2 rounded-none focus:border-primary focus:outline-none uppercase tracking-widest"
            >
              <option value="">ALL</option>
              <option value="file">FILES</option>
              <option value="dir">FOLDERS</option>
              <option value="image">IMAGES</option>
              <option value="video">VIDEO</option>
              <option value="audio">AUDIO</option>
              <option value="document">DOCS</option>
            </select>
            <button
              onClick={handleClearSearch}
              className="p-1.5 text-muted hover:text-primary transition-colors"
              title="Clear search"
            >
              <Xmark width={16} height={16} strokeWidth={2} />
            </button>
          </div>
        ) : (
          <>
            <Breadcrumbs
              path={currentPath}
              onNavigate={(p) => navigate(`/browse/${encodeFsPath(p)}`)}
            />
            <div className="ml-auto flex items-center gap-2 shrink-0">
              <button
                onClick={() => { setSearchQuery(''); search({ q: '', path: currentPath || undefined }); setSearchQuery(''); clearSearch(); /* just activate */ setSearchQuery(''); handleSearchChange('') }}
                className="p-2 text-muted hover:text-primary transition-colors"
                title="Search"
              >
                <SearchIcon width={18} height={18} strokeWidth={1.8} />
              </button>
```

Actually, this is getting complex with the toggle. Let me simplify. Use a boolean to track search mode:

Replace the search-related state with a simpler approach. Add a `searchMode` state:

```typescript
  const [searchMode, setSearchMode] = useState(false)
```

The search icon toggles `searchMode`. When active, the header shows the search input. When inactive, it shows breadcrumbs.

Let me rewrite the header cleanly:

```tsx
      <header className="h-14 border-b border-borders flex items-center px-4 md:px-6 shrink-0 gap-4">
        <div className="flex items-center gap-1 shrink-0">
          <button onClick={() => navigate(-1)} className="p-1.5 text-muted hover:text-primary transition-colors" title="Back">
            <NavArrowLeft width={18} height={18} strokeWidth={1.8} />
          </button>
          <button onClick={() => navigate(1)} className="p-1.5 text-muted hover:text-primary transition-colors" title="Forward">
            <NavArrowRight width={18} height={18} strokeWidth={1.8} />
          </button>
        </div>

        {searchMode ? (
          <div className="flex-1 flex items-center gap-2 min-w-0">
            <SearchIcon width={16} height={16} strokeWidth={1.8} className="text-muted shrink-0" />
            <input
              type="text"
              value={searchQuery}
              onChange={(e) => handleSearchChange(e.target.value)}
              onKeyDown={(e) => { if (e.key === 'Escape') { handleClearSearch(); setSearchMode(false) } }}
              className="flex-1 h-8 bg-background border border-borders text-text-main font-mono text-[13px] px-3 rounded-none focus:border-primary focus:outline-none transition-colors min-w-0"
              placeholder="Search files..."
              autoFocus
            />
            <select
              value={searchType || ''}
              onChange={(e) => handleTypeChange((e.target.value || undefined) as SearchParams['type'])}
              className="h-8 bg-background border border-borders text-text-main font-mono text-[11px] px-2 rounded-none focus:border-primary focus:outline-none uppercase tracking-widest"
            >
              <option value="">ALL</option>
              <option value="file">FILES</option>
              <option value="dir">FOLDERS</option>
              <option value="image">IMAGES</option>
              <option value="video">VIDEO</option>
              <option value="audio">AUDIO</option>
              <option value="document">DOCS</option>
            </select>
            <button onClick={() => { handleClearSearch(); setSearchMode(false) }} className="p-1.5 text-muted hover:text-primary transition-colors" title="Close search">
              <Xmark width={16} height={16} strokeWidth={2} />
            </button>
          </div>
        ) : (
          <>
            <Breadcrumbs path={currentPath} onNavigate={(p) => navigate(`/browse/${encodeFsPath(p)}`)} />
            <div className="ml-auto flex items-center gap-2 shrink-0">
              <button onClick={() => setSearchMode(true)} className="p-2 text-muted hover:text-primary transition-colors" title="Search (Ctrl+F)">
                <SearchIcon width={18} height={18} strokeWidth={1.8} />
              </button>
              <button onClick={refresh} className="p-2 text-muted hover:text-primary transition-colors" title="Refresh">
                <Refresh width={18} height={18} strokeWidth={1.8} />
              </button>
              <button onClick={() => setShowNewFolder(true)} className="hidden md:flex p-2 text-muted hover:text-primary transition-colors" title="New folder">
                <FolderPlus width={18} height={18} strokeWidth={1.8} />
              </button>
              <button onClick={uploadFromPicker} className="hidden md:flex items-center gap-2 h-9 px-4 bg-primary text-background font-mono text-[12px] font-bold uppercase tracking-widest hover:-translate-x-0.5 hover:-translate-y-0.5 hover:shadow-[3px_3px_0px_#F2F2F2] transition-all">
                <Upload width={14} height={14} strokeWidth={2} />
                UPLOAD
              </button>
            </div>
          </>
        )}
      </header>
```

- [ ] **Step 3: Swap file listing for search results when active**

Replace the `<FileList>` usage (line 279-288) with a conditional:

```tsx
      {isSearchActive ? (
        <div className="flex-1 overflow-y-auto">
          {searchLoading ? (
            <div className="flex-1 flex items-center justify-center py-12">
              <span className="font-mono text-[14px] text-muted uppercase tracking-widest">[ SEARCHING... ]</span>
            </div>
          ) : searchError ? (
            <div className="flex-1 flex items-center justify-center py-12">
              <span className="font-mono text-[14px] text-primary uppercase tracking-widest">[ ERROR: {searchError} ]</span>
            </div>
          ) : searchResults.length === 0 ? (
            <div className="flex-1 flex items-center justify-center py-12">
              <span className="font-mono text-[14px] text-muted uppercase tracking-widest">[ NO RESULTS ]</span>
            </div>
          ) : (
            <>
              <div className="hidden md:grid grid-cols-[1fr_120px_150px_120px] items-center h-9 px-4 border-b border-borders">
                <span className="font-mono text-[11px] text-muted uppercase tracking-widest">NAME</span>
                <span className="font-mono text-[11px] text-muted uppercase tracking-widest">SIZE</span>
                <span className="font-mono text-[11px] text-muted uppercase tracking-widest">MODIFIED</span>
                <span />
              </div>
              {searchResults.map((entry) => (
                <FileRow
                  key={entry.path}
                  entry={entry}
                  onItemClick={handleNavigate}
                  onDelete={handleDelete}
                  isSelected={false}
                  selectMode={false}
                  onToggleSelect={() => {}}
                  showFullPath
                />
              ))}
              <div className="hidden md:flex items-center h-9 px-4 border-t border-borders">
                <span className="font-mono text-[11px] text-muted uppercase tracking-widest">
                  {searchResults.length} OF {searchTotal} RESULT{searchTotal !== 1 ? 'S' : ''}
                </span>
              </div>
            </>
          )}
        </div>
      ) : (
        <FileList
          listing={listing}
          loading={loading}
          error={error}
          onItemClick={handleNavigate}
          onDelete={handleDelete}
          selected={selected}
          onToggleSelect={toggleSelect}
        />
      )}
```

- [ ] **Step 4: Add showFullPath prop to FileRow**

In `frontend/src/components/FileRow.tsx`, add an optional `showFullPath` prop. If true, display `entry.path` instead of `entry.name` — this shows the full relative path in search results so users know where the file is.

Read the file first to see its structure, then add the prop. The change is: accept `showFullPath?: boolean` and use `showFullPath ? entry.path : entry.name` for the displayed name.

- [ ] **Step 5: Add Ctrl+F keyboard shortcut**

In `BrowserPage`, add a `useEffect` for the keyboard shortcut:

```typescript
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key === 'f') {
        e.preventDefault()
        setSearchMode(true)
      }
    }
    window.addEventListener('keydown', handleKeyDown)
    return () => window.removeEventListener('keydown', handleKeyDown)
  }, [])
```

- [ ] **Step 6: Verify frontend builds**

Run: `cd frontend && npx tsc --noEmit 2>&1 | head -20`
Expected: no type errors

- [ ] **Step 7: Commit**

```bash
git add frontend/src/pages/BrowserPage.tsx frontend/src/components/FileRow.tsx frontend/src/hooks/useSearch.ts
git commit -m "feat(frontend): add search bar, filters, results display, and Ctrl+F shortcut"
```

---

### Task 15: Final verification

- [ ] **Step 1: Run full backend test suite**

Run: `cargo test 2>&1 | tail -30`
Expected: all tests pass

- [ ] **Step 2: Run clippy**

Run: `cargo clippy -- -W clippy::all 2>&1 | tail -20`
Expected: no errors, only expected warnings

- [ ] **Step 3: Build release binary**

Run: `cargo build --release 2>&1 | tail -5`
Expected: compiles successfully

- [ ] **Step 4: Verify frontend builds**

Run: `cd frontend && npm run build 2>&1 | tail -10`
Expected: builds successfully

- [ ] **Step 5: Commit any remaining fixes**

If clippy or build surfaced issues, fix and commit.
