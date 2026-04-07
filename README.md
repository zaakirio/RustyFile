# RustyFile

A fast, lightweight, self-hosted file browser built in Rust.

[![CI](https://github.com/zaakirio/RustyFile/actions/workflows/ci.yml/badge.svg)](https://github.com/zaakirio/RustyFile/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

## Why RustyFile?

Most self-hosted file browsers are written in Go or PHP. RustyFile takes a different approach — Rust's zero-cost abstractions, no garbage collector, and a single static binary that starts in milliseconds and idles at ~15 MB.

### How It Compares

| | RustyFile | [FileBrowser](https://github.com/filebrowser/filebrowser) | [Filestash](https://github.com/mickael-kerjean/filestash) | [Nextcloud](https://nextcloud.com) |
|---|---|---|---|---|
| **Language** | Rust | Go | Go | PHP |
| **Memory baseline** | ~15 MB | Not documented | 128–512 MB | 512 MB+ |
| **Startup** | Sub-50 ms | Seconds | Seconds | Seconds (PHP init) |
| **Single binary** | Yes | Yes | Yes | No (requires LAMP stack) |
| **License** | MIT | Apache 2.0 | AGPL-3.0 | AGPL-3.0 |
| **Storage backends** | Local filesystem | Local filesystem | 20+ (S3, FTP, WebDAV…) | Local + integrations |
| **Project status** | Active | Maintenance-only | Active | Active |

**Where RustyFile wins:** startup speed, memory footprint, and deployment simplicity. Drop a single binary on a Raspberry Pi or NAS and go.

**Where others win:** Filestash supports 20+ remote storage backends. Nextcloud is a full collaboration suite (calendar, contacts, document editing). If you need those, RustyFile isn't the right tool.

## Features

- **File browsing** — navigate directories, view metadata, sort by name/size/date/type
- **Video streaming** — MP4/WebM via HTML5 player with HTTP Range requests and auto-detected subtitles (.vtt, .srt)
- **In-browser text editing** — edit code and config files, saved atomically
- **Authentication** — JWT (HS256) + Argon2id password hashing, cookie and Bearer token support
- **ETag caching** — conditional requests return 304 Not Modified for unchanged files
- **Security headers** — CSP, X-Content-Type-Options, restrictive CORS
- **Zero-config onboarding** — first visit creates the admin account, no config files needed

### Planned

- TUS resumable uploads with progress tracking
- Full-text filename search via SQLite FTS5

## Quick Start

### Binary

```bash
curl -fsSL https://github.com/zaakirio/RustyFile/releases/latest/download/rustyfile-linux-amd64.tar.gz | tar xz
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
cd RustyFile
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

The download endpoint supports HTTP Range requests for seeking:

```bash
# Seek to byte offset
curl -H "Authorization: Bearer $TOKEN" -H "Range: bytes=1000000-" \
  http://localhost:8080/api/fs/download/video.mp4
```

The browser `<video>` element handles this automatically. Conditional requests (`ETag`, `If-Modified-Since`) return `304 Not Modified` for unchanged files.

### Authentication Flow

```
1. POST /api/setup/admin   — First run: create admin, get JWT
2. POST /api/auth/login    — Subsequent: login, get JWT
3. Authorization: Bearer <token>
4. POST /api/auth/refresh  — Renew before expiry
```

### Error Format

All errors return `{ "error": "message", "code": "ERROR_CODE" }`.

| Status | Code | When |
|--------|------|------|
| 400 | `VALIDATION_ERROR` | Invalid input |
| 401 | `UNAUTHORIZED` | Missing or invalid token |
| 403 | `FORBIDDEN` | Insufficient permissions |
| 404 | `NOT_FOUND` | Resource doesn't exist |
| 409 | `CONFLICT` | Resource already exists |
| 410 | `SETUP_EXPIRED` | Setup timeout elapsed |
| 500 | `INTERNAL_ERROR` | Server error |

## Architecture

```
src/
  main.rs              — config, logging, DB, server startup
  config.rs            — layered config (Figment + Clap)
  error.rs             — AppError → HTTP response mapping
  state.rs             — shared state (DB pool, config, JWT secret)
  api/
    mod.rs             — router assembly, middleware stack
    health.rs          — health check
    setup.rs           — first-run onboarding
    auth.rs            — JWT login/logout/refresh
    files.rs           — file CRUD (browse, edit, create, delete, rename)
    download.rs        — streaming downloads + Range requests + ETag
    middleware/auth.rs  — JWT validation middleware
  db/
    mod.rs             — SQLite pool, migrations, interact() helper
    user_repo.rs       — user CRUD queries
  services/
    file_ops.rs        — path resolution, dir listing, atomic writes
```

### Tech Stack

| Component | Choice | Why |
|-----------|--------|-----|
| Web framework | Axum 0.8 | Tower middleware, Tokio-native |
| Database | SQLite (rusqlite, bundled) | Zero-dependency, single-file, FTS5-ready |
| Auth | JWT (HS256) + Argon2id | Stateless tokens, modern password hashing |
| Config | Figment + Clap | Layered: CLI > env > file > defaults |
| Logging | tracing | Structured, async-aware, JSON output |
| File I/O | tokio::fs | Async, non-blocking |

### Security

- Path traversal prevention — all paths canonicalized against root
- `Content-Security-Policy: script-src 'none'` on file downloads
- `X-Content-Type-Options: nosniff`
- `Cache-Control: no-store` on API responses, `private` on downloads
- Atomic writes via temp file + rename (no corruption on crash)
- 5-minute setup timeout, then locked until restart
- Explicit CORS method/header allowlist
- Client IP logging via X-Forwarded-For / X-Real-IP

## Development

**Prerequisites:** Rust 1.80+ (`rustup update stable`). Optional: Node 22+ and pnpm for frontend work.

```bash
make dev        # development server
make test       # integration tests
make build      # optimized release binary
make docker     # Docker image
cargo fmt && cargo clippy -- -D warnings   # format + lint
```

See [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines on adding endpoints and writing tests.

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

## License

MIT
