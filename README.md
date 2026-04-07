# RustyFile

A fast, lightweight, self-hosted file browser built in Rust.

[![CI](https://github.com/zaakirio/RustyFile/actions/workflows/ci.yml/badge.svg)](https://github.com/zaakirio/RustyFile/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

## Why RustyFile?

Existing file browsers (FileBrowser, Filestash) are built in Go. RustyFile brings Rust's zero-cost abstractions to the table:

- **Sub-50ms startup** -- no runtime, no GC
- **Zero-copy file streaming** -- video seeking works instantly via HTTP Range requests
- **~15 MB memory baseline** -- runs on anything from a Raspberry Pi to a NAS
- **Single binary** -- drop it on a server and go, or `docker run`
- **Seamless onboarding** -- first visit creates admin account, no config files needed

## Features

- **File browsing** -- Navigate directories, view metadata, sort by name/size/date/type
- **Video playback** -- Stream MP4/WebM with native HTML5 player, auto-detects subtitles (.vtt, .srt)
- **Text editing** -- Edit code and text files in-browser, saves atomically
- **Drag-and-drop upload** -- TUS resumable upload protocol (planned)
- **Search** -- Full-text filename search via SQLite FTS5 (planned)
- **Share links** -- Expiring, password-protected public links (planned)
- **Authentication** -- JWT + Argon2id, with cookie and Bearer token support
- **ETag caching** -- Conditional requests (304 Not Modified) for efficient bandwidth
- **Security headers** -- CSP, X-Content-Type-Options, restrictive CORS

## Quick Start

### Binary

```bash
# Download (replace with actual release URL)
curl -fsSL https://github.com/zaakirio/RustyFile/releases/latest/download/rustyfile-linux-amd64.tar.gz | tar xz

# Run -- serves files from ./my-files, stores DB in ./data
./rustyfile --root ./my-files --data-dir ./data --port 8080

# Visit http://localhost:8080 to create your admin account
```

### Docker

```bash
docker run -d \
  -p 8080:80 \
  -v /path/to/your/files:/data \
  -v rustyfile_data:/config \
  rustyfile:latest
```

### Docker Compose

```yaml
services:
  rustyfile:
    image: rustyfile:latest
    ports:
      - "8080:80"
    volumes:
      - ./files:/data
      - rustyfile_data:/config
    restart: unless-stopped

volumes:
  rustyfile_data:
```

### From Source

```bash
git clone https://github.com/zaakirio/RustyFile.git
cd rustyfile
cargo build --release
./target/release/rustyfile --root ./test-data --port 8080
```

## Configuration

RustyFile uses layered configuration (highest priority wins):

```
CLI flags  >  Environment variables (RUSTYFILE_*)  >  config.toml  >  Defaults
```

### Options

| Option | Env Var | Default | Description |
|--------|---------|---------|-------------|
| `--host` | `RUSTYFILE_HOST` | `0.0.0.0` | Listen address |
| `--port` | `RUSTYFILE_PORT` | `8080` | Listen port |
| `--root` | `RUSTYFILE_ROOT` | `/data` | Directory to serve |
| `--data-dir` | `RUSTYFILE_DATA_DIR` | `/var/lib/rustyfile` | Database & cache directory |
| `--log-level` | `RUSTYFILE_LOG_LEVEL` | `info` | Log level (trace/debug/info/warn/error) |
| `--log-format` | `RUSTYFILE_LOG_FORMAT` | `pretty` | Log format (`json` or `pretty`) |
| `--jwt-expiry-hours` | `RUSTYFILE_JWT_EXPIRY_HOURS` | `2` | JWT token lifetime |
| `--min-password-length` | `RUSTYFILE_MIN_PASSWORD_LENGTH` | `10` | Minimum password length |
| `--setup-timeout-minutes` | `RUSTYFILE_SETUP_TIMEOUT_MINUTES` | `5` | Setup wizard timeout |
| `--config` | -- | `config.toml` | Config file path |

See [`config.toml.example`](config.toml.example) for a full example.

## API Reference

### Public Endpoints

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/api/health` | Health check |
| `GET` | `/api/setup/status` | Setup required? `{ setup_required: bool }` |
| `POST` | `/api/setup/admin` | Create initial admin (first run only) |
| `POST` | `/api/auth/login` | Login with username/password |
| `POST` | `/api/auth/logout` | Logout |
| `POST` | `/api/auth/refresh` | Refresh JWT token |

### Protected Endpoints (require `Authorization: Bearer <token>`)

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/api/fs` | List root directory |
| `GET` | `/api/fs/{path}` | List directory or get file info |
| `GET` | `/api/fs/{path}?content=true` | Get file info with text content |
| `PUT` | `/api/fs/{path}` | Save file content |
| `POST` | `/api/fs/{path}` | Create directory (`{"type":"directory"}`) |
| `DELETE` | `/api/fs/{path}` | Delete file or directory |
| `PATCH` | `/api/fs/{path}` | Rename/move (`{"destination":"new/path"}`) |
| `GET` | `/api/fs/download/{path}` | Download file (supports Range requests) |
| `GET` | `/api/fs/download/{path}?inline=true` | View file inline in browser |

### Video Streaming

The download endpoint supports HTTP Range requests for video seeking:

```bash
# Full download
curl -H "Authorization: Bearer $TOKEN" \
  http://localhost:8080/api/fs/download/video.mp4

# Seek to byte offset (video scrubbing)
curl -H "Authorization: Bearer $TOKEN" -H "Range: bytes=1000000-" \
  http://localhost:8080/api/fs/download/video.mp4

# Browser <video> element handles this automatically
```

Conditional requests are also supported -- the server returns `304 Not Modified` if the file hasn't changed (via `ETag` and `If-Modified-Since` headers).

### Authentication Flow

```
1. POST /api/setup/admin        -- First run: create admin, get JWT
2. POST /api/auth/login          -- Subsequent: login, get JWT
3. Use JWT as: Authorization: Bearer <token>
4. POST /api/auth/refresh        -- Renew before expiry
```

### Error Format

```json
{
  "error": "Human-readable message",
  "code": "ERROR_CODE"
}
```

| HTTP Status | Code | When |
|-------------|------|------|
| 400 | `VALIDATION_ERROR` | Invalid input |
| 401 | `UNAUTHORIZED` | Missing or invalid token |
| 403 | `FORBIDDEN` | Insufficient permissions |
| 404 | `NOT_FOUND` | Resource doesn't exist |
| 409 | `CONFLICT` | Resource already exists |
| 410 | `SETUP_EXPIRED` | Setup timeout elapsed |
| 416 | -- | Invalid Range header |
| 500 | `INTERNAL_ERROR` | Server error |

## Architecture

```
src/
  main.rs               -- Entry: config, logging, DB, server startup
  config.rs             -- Layered config (figment + clap)
  error.rs              -- AppError -> HTTP response mapping
  state.rs              -- Shared state (DB pool, config, JWT secret)
  api/
    mod.rs              -- Router assembly, middleware stack
    health.rs           -- Health check
    setup.rs            -- Onboarding (Portainer-style)
    auth.rs             -- JWT login/logout/refresh
    files.rs            -- File CRUD (browse, edit, create, delete, rename)
    download.rs         -- Streaming downloads + Range requests + ETag
    middleware/auth.rs   -- JWT validation middleware
  db/
    mod.rs              -- SQLite pool, migrations, interact() helper
    user_repo.rs        -- User CRUD queries
  services/
    file_ops.rs         -- Path resolution, dir listing, atomic writes
```

### Tech Stack

| Component | Choice | Why |
|-----------|--------|-----|
| Web framework | Axum 0.8 | Tower middleware ecosystem, Tokio-native |
| Database | SQLite (rusqlite bundled) | Zero-dependency, single-file, FTS5 ready |
| Auth | JWT (HS256) + Argon2id | Stateless tokens, modern password hashing |
| Config | Figment + Clap | Layered: CLI > env > file > defaults |
| Logging | tracing + tracing-subscriber | Structured, async-aware, JSON output |
| File I/O | tokio::fs | Async, non-blocking |

### Security

- **Path traversal prevention** -- All paths canonicalized against root at startup
- **Content-Security-Policy** -- `script-src 'none'` on all file downloads
- **X-Content-Type-Options** -- `nosniff` prevents MIME sniffing attacks
- **ETag / conditional requests** -- 304 Not Modified for unchanged files
- **Cache-Control** -- `no-store` on API responses, `private` on downloads
- **Atomic writes** -- File saves use temp file + rename (no corruption on crash)
- **Setup timeout** -- 5-minute window to create admin, then locked until restart
- **CORS** -- Explicit method/header allowlist
- **Client IP logging** -- X-Forwarded-For / X-Real-IP in structured log spans

## Development

### Prerequisites

- Rust 1.80+ (`rustup update stable`)
- (Optional) Node 22+ and pnpm for frontend development

### Commands

```bash
# Development server with hot data
make dev

# Run all 23 integration tests
make test

# Build optimized release binary
make build

# Build Docker image
make docker

# Format and lint
cargo fmt
cargo clippy -- -D warnings
```

### Running Tests

```bash
# All tests
cargo test

# Specific test file
cargo test --test setup_test

# With output
cargo test -- --nocapture
```

### Adding a New Endpoint

1. Create handler in `src/api/your_module.rs`
2. Register route in that module's `routes()` function
3. Nest into router in `src/api/mod.rs`
4. Add integration test in `tests/`

See [CONTRIBUTING.md](CONTRIBUTING.md) for detailed guidelines.

## Deployment

### Reverse Proxy (nginx)

```nginx
server {
    listen 443 ssl;
    server_name files.example.com;

    client_max_body_size 10G;

    location / {
        proxy_pass http://127.0.0.1:8080;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
        proxy_request_buffering off;
    }
}
```

### Production Checklist

- [ ] Set a strong admin password on first run
- [ ] Configure TLS (directly or via reverse proxy)
- [ ] Set `RUSTYFILE_ROOT` to the directory you want to serve
- [ ] Ensure `RUSTYFILE_DATA_DIR` is on persistent storage
- [ ] Set `log_level` to `warn` for reduced noise
- [ ] Set `log_format` to `json` for log aggregation

## Roadmap

- [ ] Frontend (Vue 3 SPA with mobile-first design)
- [ ] TUS resumable uploads with progress tracking
- [ ] SQLite FTS5 search with type/size/date filters
- [ ] Share links (expiring, password-protected)
- [ ] Image thumbnails (SIMD-accelerated via fast_image_resize)
- [ ] OpenAPI docs (utoipa + Scalar UI at `/docs`)
- [ ] User management (admin panel, roles)
- [ ] Multi-platform releases via GitHub Actions + cargo-dist

## License

MIT
