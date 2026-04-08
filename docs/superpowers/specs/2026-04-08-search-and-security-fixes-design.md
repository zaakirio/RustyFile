# Search Feature + Security Fixes Design

**Date:** 2026-04-08
**Scope:** SQLite-indexed file search, two security fixes, one refactor

---

## 1. Security Fix: Secure Cookie Flag

**File:** `src/api/auth.rs:152-155`

**Problem:** The `rustyfile_token` cookie is set without the `Secure` flag. In production over HTTPS, the cookie could be sent over an insecure HTTP connection.

**Fix:** Add a `secure_cookie` config field (default `true`). When true, append `; Secure` to the Set-Cookie string. The login and logout handlers both construct cookie strings — both must be updated.

**Config addition to `AppConfig`:**
- `secure_cookie: bool` — defaults to `true`, overridable via `RUSTYFILE_SECURE_COOKIE=false` for local dev

---

## 2. Security Fix: Trusted Proxy Enforcement

**File:** `src/api/mod.rs:33-46`

**Problem:** `extract_client_ip()` trusts `X-Forwarded-For` and `X-Real-IP` unconditionally. The `trusted_proxies` config field exists but is not enforced. An attacker can spoof their IP to bypass rate limiting.

**Fix:** `extract_client_ip()` gains a second parameter: `ConnectInfo<SocketAddr>` (axum's real peer address). Logic:
1. If `trusted_proxies` is empty (default), trust proxy headers unconditionally (backwards compatible — assumes reverse proxy strips them).
2. If `trusted_proxies` is set, only read `X-Forwarded-For`/`X-Real-IP` when the peer address matches a trusted proxy CIDR.
3. If the peer is not trusted, use the raw peer socket address.

This requires adding `ConnectInfo` to the axum server setup (`.into_make_service_with_connect_info::<SocketAddr>()`). The login handler and trace span both call `extract_client_ip` — both get the updated signature.

---

## 3. Refactor: FileEntry Constructor Deduplication

**File:** `src/services/file_ops.rs`

**Problem:** `file_info()` (line 174) and `list_directory()` (line 86) duplicate the FileEntry construction logic — metadata extraction, mime detection, extension extraction, relative path computation.

**Fix:** Add `FileEntry::from_path_and_metadata(canonical_root: &Path, entry_path: &Path, metadata: &Metadata) -> FileEntry`. Both functions call this shared constructor. This follows yazi's `File::from()` and rustdesk's pattern of centralizing data construction.

---

## 4. Search Feature: SQLite-Indexed File Search

### 4.1 Database Schema (V3 Migration)

New migration file: `migrations/V3__search_index.sql`

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

- `path` is the relative path from root (primary key, unique naturally)
- `name` is the filename component only — `LIKE '%query%' COLLATE NOCASE` for search
- `modified` stored as ISO 8601 text (lexicographic sort works correctly)
- No separate `id` column — `path` is the natural key

### 4.2 SearchIndexer Service

New file: `src/services/search_index.rs`

**Struct:**
```rust
#[derive(Clone)]
pub struct SearchIndexer {
    db: Pool,
    canonical_root: PathBuf,
}
```

**Methods:**

- `new(db, canonical_root) -> Self`
- `full_reindex(&self) -> anyhow::Result<()>` — walks the file tree, batch-inserts 500 rows per transaction, then deletes stale entries (paths in index but not on disk). Runs once at startup as a background task.
- `upsert(&self, rel_path: &str) -> anyhow::Result<()>` — reads metadata for a single path, inserts or replaces the index row. Called by the watcher and mutation handlers.
- `remove(&self, rel_path: &str) -> anyhow::Result<()>` — deletes the index row. Called on file/directory deletion.
- `remove_prefix(&self, rel_prefix: &str) -> anyhow::Result<()>` — deletes all rows under a directory prefix. Called on directory deletion.
- `rename(&self, old_prefix: &str, new_prefix: &str) -> anyhow::Result<()>` — updates path prefixes in the index. Called on rename/move.
- `search(&self, query: &SearchQuery) -> anyhow::Result<SearchResults>` — builds and executes the SQL query.

**SearchQuery struct:**
```rust
pub struct SearchQuery {
    pub q: String,                    // filename search term
    pub file_type: Option<FileType>,  // file, dir, image, video, audio, document
    pub min_size: Option<u64>,
    pub max_size: Option<u64>,
    pub after: Option<DateTime<Utc>>,
    pub before: Option<DateTime<Utc>>,
    pub path: Option<String>,         // scope to subdirectory
    pub limit: u32,                   // default 50, max 200
    pub offset: u32,                  // pagination offset
}

pub enum FileType {
    File,
    Dir,
    Image,
    Video,
    Audio,
    Document,
}
```

**FileType mapping to MIME prefixes / extensions:**
- `Image` → `mime_type LIKE 'image/%'`
- `Video` → `mime_type LIKE 'video/%'`
- `Audio` → `mime_type LIKE 'audio/%'`
- `Document` → extension IN ('pdf', 'doc', 'docx', 'xls', 'xlsx', 'ppt', 'pptx', 'txt', 'md', 'csv', 'json', 'xml', 'yaml', 'yml', 'toml')
- `File` → `is_dir = 0`
- `Dir` → `is_dir = 1`

**SQL query construction:** Parameterized query built dynamically. The `q` parameter uses `name LIKE '%' || ?1 || '%' COLLATE NOCASE` (not string interpolation — avoids SQL injection). Filters are ANDed. Results ordered by relevance: exact name match first, then prefix match, then contains — achieved via `ORDER BY CASE WHEN name = ?1 THEN 0 WHEN name LIKE ?1 || '%' THEN 1 ELSE 2 END, name`.

**SearchResults struct:**
```rust
pub struct SearchResults {
    pub results: Vec<FileEntry>,  // reuses existing FileEntry
    pub total: usize,
    pub query: String,
}
```

Total count uses a separate `SELECT COUNT(*)` with the same WHERE clause (needed for pagination).

### 4.3 Watcher Integration

The existing filesystem watcher in `main.rs` currently sends events to `dir_cache_watcher`. Extend it to also update the search index:

- Create a second `tokio::sync::mpsc::channel` for index events
- The watcher callback sends to both channels
- A second `tokio::spawn` task consumes index events and calls `search_indexer.upsert()` / `search_indexer.remove()` as appropriate
- Event types: `Create`/`Modify` → `upsert`, `Remove` → `remove`, `Rename` → `remove` old + `upsert` new

### 4.4 Mutation Handler Integration

All existing file mutation handlers must notify the indexer after successful operations:

| Handler | Index Operation |
|---------|----------------|
| `files::save_file` | `upsert` the saved path |
| `files::create_dir` | `upsert` the new directory |
| `files::delete` | `remove` (file) or `remove_prefix` (directory) |
| `files::rename` | `rename` prefix update |
| `tus::complete_upload` | `upsert` the completed file |

These are fire-and-forget — use `tokio::spawn` so mutation handlers don't block on index updates. Follow rustdesk's `allow_err!()` pattern for logging failures without propagating.

### 4.5 Search API Endpoint

New file: `src/api/search.rs`

```
GET /api/fs/search?q=test&type=image&min_size=1024&max_size=10485760&after=2026-01-01&before=2026-12-31&path=photos&limit=50&offset=0
```

- Requires authentication (behind `require_auth` middleware)
- `q` is required, minimum 1 character
- All filter parameters are optional
- Returns JSON: `{ results: [...FileEntry...], total: 123, query: "test" }`
- Registered in `api/mod.rs` under the `cached_api_routes` group (gets `no-cache` header)

### 4.6 Frontend

**Search bar:** Added to `BrowserPage` header area. Input with search icon, debounced at 300ms. Pressing Enter or typing 2+ characters triggers search.

**Search results view:** Replaces the file listing when a search is active. Uses the same file entry rendering (icons, names, sizes, dates). Each result shows the full relative path (since results span directories). Clicking a result navigates to the file (same logic as existing file click handling).

**Filter controls:** Collapsible filter bar below the search input:
- Type: dropdown (All, Files, Folders, Images, Videos, Audio, Documents)
- Size: min/max inputs with unit selector (KB, MB, GB)
- Date: after/before date pickers

**State management:** Search state lives in `BrowserPage` component state. A new `useSearch` hook wraps the API call with debouncing and filter state. Clearing the search input returns to the normal directory listing.

**API integration:** New function in `api/client.ts`:
```typescript
search(params: SearchParams): Promise<SearchResponse>
```

---

## 5. AppState Changes

`AppState` gains one new field:
```rust
pub search_indexer: SearchIndexer,
```

Constructed in `main.rs` after pool creation. `full_reindex()` spawned as a background task (non-blocking startup).

---

## 6. Error Handling Convention

Following rustdesk's pattern for this work:
- Internal service logic (SearchIndexer methods) returns `anyhow::Result<T>`
- HTTP handlers convert to `AppError` at the boundary via `From<anyhow::Error>`
- Fire-and-forget index updates use a logging macro (inspired by rustdesk's `allow_err!`)

---

## 7. Testing

- **Unit test:** `safe_resolve` edge cases, `FileEntry::from_path_and_metadata` correctness
- **Integration test:** `tests/search_test.rs` — create files, wait for index, search by name, verify results. Filter by type, size, date. Pagination.
- **Integration test:** Verify index updates on file mutation (create → searchable, delete → gone, rename → updated path)
