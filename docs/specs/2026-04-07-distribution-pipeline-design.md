# RustyFile Distribution Pipeline Design

**Date:** 2026-04-07
**Status:** Approved
**Scope:** Frontend embedding, package manager migration, Docker multi-arch, GHCR publishing, release automation

## Problem

RustyFile's backend and frontend are separate — the Rust binary serves only `/api/*` and the React frontend requires a separate dev server or reverse proxy. There is no published Docker image, no multi-arch support, no release automation, and no way for an end user to `docker run` a single container and get a working application.

## Goals

1. Single binary serves both API and frontend (like filebrowser, filebrowser-quantum, filestash)
2. `docker run -p 8080:80 -v ./files:/data ghcr.io/zaakir/rustyfile:latest` works on amd64 and arm64
3. Pre-built binaries available via GitHub Releases for Linux and macOS
4. Developer experience: `make dev` starts everything, `make build` produces the full production binary

## Non-Goals

- Kubernetes manifests / Helm chart (future work)
- Prometheus metrics endpoint (future work)
- `RUSTYFILE_JWT_SECRET` env var for multi-instance clustering (future work)
- Frontend CI (lint/test in pipeline — can be added later)
- `build.rs` auto-triggering frontend builds (explicit sequencing via Makefile/Dockerfile preferred)

---

## 1. Frontend Embedding

### Crate: rust-embed v8

Add `rust-embed` as an optional dependency behind an `embed-frontend` feature flag (default on).

**Cargo.toml additions:**

```toml
[features]
default = ["embed-frontend"]
embed-frontend = ["dep:rust-embed"]

[dependencies]
rust-embed = { version = "8", features = ["compression"], optional = true }
```

The `compression` feature deflate-compresses embedded files to reduce binary size. HTTP-level gzip/brotli is handled by the existing `tower_http::compression::CompressionLayer`.

### New file: `src/frontend.rs`

A handler function `static_handler` that serves embedded assets with SPA catch-all logic:

- `/` or `/index.html` -> serve index.html, `Cache-Control: no-cache`
- `/assets/*` (Vite hashed output) -> serve file, `Cache-Control: public, max-age=31536000, immutable`
- Other files with extensions (favicon.svg, icons.svg, noise.svg) -> serve file, `Cache-Control: no-cache`
- Paths without extensions (SPA routes: /browse/*, /login, /edit/*) -> serve index.html
- Paths with extensions that don't match any file -> 404

When the `embed-frontend` feature is disabled, the handler returns 404 with an explanatory message.

### Router integration

In `src/api/mod.rs`, add `.fallback(frontend::static_handler)` after the `/api` nests. The `/api/*` routes match first because `nest` takes priority over `fallback`.

### Dev workflow

Unchanged. `vite dev` on port 5173 proxies `/api` to the Rust backend on 8080. No embedding involved during development.

For testing integrated serving without Vite: `cd frontend && pnpm build && cd .. && cargo run`. rust-embed reads from disk in debug builds (without the `debug-embed` feature flag), so frontend changes are picked up without recompiling Rust.

Backend-only development: `cargo build --no-default-features` skips the rust-embed dependency entirely. No Node.js required.

---

## 2. Package Manager Migration (npm -> pnpm)

### Rationale

- 3-5x faster installs than npm
- Strict non-flat node_modules prevents phantom dependencies
- Superior Docker layer caching via `pnpm fetch` (fetches from lockfile alone, before package.json is copied)
- Used by Vite itself — upstream compatibility guaranteed
- Human-readable YAML lockfile (diffable in PRs)

### Changes

- Delete `frontend/package-lock.json`
- Generate `frontend/pnpm-lock.yaml` via `pnpm import` (reads existing package-lock.json) or `pnpm install`
- Add `frontend/.npmrc` with `shamefully-hoist=false`
- No `packageManager` field in `package.json` — pnpm version is managed by the Dockerfile and Makefile explicitly, not corepack. This avoids friction for contributors.
- No changes to `package.json` scripts — `pnpm run build` calls `tsc -b && vite build` identically

### What doesn't change

All dependencies (React 19, Vite 8, Tailwind 4, TypeScript 6) work identically with pnpm. Build output is the same. No code changes.

---

## 3. Dockerfile (3-Stage)

### Stage 1: Frontend (node:22-alpine)

```dockerfile
FROM node:22-alpine AS frontend
RUN npm install -g pnpm
WORKDIR /app/frontend
COPY frontend/pnpm-lock.yaml .
RUN pnpm fetch --frozen-lockfile
COPY frontend/package.json .
RUN pnpm install --offline --frozen-lockfile
COPY frontend/ .
RUN pnpm run build
```

`pnpm fetch` creates an optimal cache layer — only re-runs when the lockfile changes. `--offline` ensures the install uses only what was fetched (deterministic).

### Stage 2: Rust (rust:1.83-alpine)

```dockerfile
FROM rust:1.83-alpine AS builder
RUN apk add --no-cache musl-dev
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo 'fn main(){}' > src/main.rs && cargo build --release 2>/dev/null ; rm -rf src
COPY src/ src/
COPY migrations/ migrations/
COPY --from=frontend /app/frontend/dist frontend/dist
RUN cargo build --release
```

The dummy `main.rs` trick pre-compiles dependencies (cache layer). `frontend/dist` is copied in so rust-embed finds the files at compile time.

### Stage 3: Runtime (alpine:3.21)

Identical to current — ca-certificates, non-root user (UID 1000), copy binary, healthcheck, expose 80.

Expected image size: ~15-20MB.

### .dockerignore additions

```
frontend/node_modules/
```

---

## 4. CI/CD Workflows

### 4.1 Docker + GHCR (`.github/workflows/docker.yml`)

**Trigger:** push tags `v*.*.*`

**Single job:**

1. Checkout
2. Login to GHCR (`docker/login-action` using `GITHUB_TOKEN`)
3. Setup QEMU (`docker/setup-qemu-action`)
4. Setup Docker Buildx (`docker/setup-buildx-action`)
5. Extract metadata (`docker/metadata-action`):
   - Tags: `latest`, `1.2.3`, `1.2`, `1`, `sha-abc1234`
6. Build and push (`docker/build-push-action`):
   - Platforms: `linux/amd64`, `linux/arm64`
   - Cache: `type=gha,mode=max`
7. Generate build provenance attestation

All building (frontend + Rust) happens inside the Dockerfile. Docker layer caching avoids redundant work.

**Permissions required:** `contents: read`, `packages: write`, `attestations: write`, `id-token: write`

### 4.2 Pre-built Binaries (`.github/workflows/release.yml`)

**Trigger:** push tags `v*.*.*`

**Job 1: build-frontend**
- Setup Node 22 + pnpm
- `pnpm install --frozen-lockfile && pnpm run build`
- Upload `frontend/dist/` as artifact

**Job 2: build-binaries** (matrix, needs build-frontend)

| Target | Runner | Method |
|---|---|---|
| x86_64-unknown-linux-gnu | ubuntu-latest | native cargo |
| aarch64-unknown-linux-gnu | ubuntu-latest | cross |
| x86_64-apple-darwin | macos-latest | native cargo |
| aarch64-apple-darwin | macos-latest | native cargo |

Each job:
- Downloads frontend artifact into `frontend/dist/`
- Builds release binary
- Packages as `rustyfile-{version}-{target}.tar.gz` with SHA256

**Job 3: release** (needs build-binaries)
- Downloads all artifacts
- Creates GitHub Release (`softprops/action-gh-release`)
- Attaches all `.tar.gz` + `.sha256` files
- Auto-generates release notes from commits

### 4.3 Existing CI (`.github/workflows/ci.yml`)

Unchanged. Runs `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test` on every push/PR.

Note: `cargo test` and `cargo clippy` will run without the frontend built. Since `embed-frontend` is a default feature, we need `frontend/dist/` to exist for these to compile. Two options:

- **Option A:** CI creates an empty `frontend/dist/index.html` stub before Rust steps. Tests don't exercise the frontend handler, so content doesn't matter.
- **Option B:** CI builds the frontend before Rust steps. More correct but slower.

Recommended: **Option A** for PR CI speed. The Docker workflow validates the full build on release.

### Estimated build times

| Workflow | Time |
|---|---|
| CI (existing, per PR) | ~3-5 min |
| Docker (on tag, both arches) | ~40 min |
| Release binaries (on tag, 4 targets parallel) | ~15 min |

---

## 5. Makefile

```makefile
.PHONY: dev build test lint docker clean

dev:
    cd frontend && pnpm dev &
    cargo run -- --root ./test-data --data-dir ./tmp-data

build:
    cd frontend && pnpm install && pnpm run build
    cargo build --release

test:
    cargo test

lint:
    cargo fmt --check
    cargo clippy -- -D warnings
    cd frontend && pnpm lint

docker:
    docker buildx build -t rustyfile:latest .

clean:
    cargo clean
    rm -rf frontend/dist frontend/node_modules
```

No `build.rs`. Frontend build sequencing is explicit in the Makefile (for local dev) and the Dockerfile (for production). This keeps `cargo build` predictable and doesn't require Node.js as a hard dependency for Rust compilation.

---

## 6. File Changes Summary

### New files

| File | Purpose |
|---|---|
| `src/frontend.rs` | Embedded SPA handler |
| `Makefile` | Dev/build/test/lint/docker/clean targets |
| `.github/workflows/docker.yml` | Multi-arch Docker + GHCR |
| `.github/workflows/release.yml` | Pre-built binaries + GitHub Release |
| `frontend/.npmrc` | pnpm strict hoisting config |
| `frontend/pnpm-lock.yaml` | pnpm lockfile |

### Modified files

| File | Change |
|---|---|
| `Cargo.toml` | Add rust-embed (optional), embed-frontend feature |
| `src/lib.rs` | Add `pub mod frontend;` |
| `src/api/mod.rs` | Add `.fallback(frontend::static_handler)` |
| `Dockerfile` | 3-stage: node/pnpm + rust + alpine |
| `.dockerignore` | Add `frontend/node_modules/` |
| `.gitignore` | Ensure `frontend/dist/`, `frontend/node_modules/` ignored |
| `.github/workflows/ci.yml` | Add frontend dist stub for compilation |

### Deleted files

| File | Reason |
|---|---|
| `frontend/package-lock.json` | Replaced by pnpm-lock.yaml |

---

## 7. End-User Experience

After this ships:

```bash
# Docker (any arch):
docker run -d -p 8080:80 -v ./files:/data ghcr.io/zaakir/rustyfile:latest

# Docker Compose:
docker compose up -d

# Pre-built binary:
curl -LsSf https://github.com/.../releases/download/v1.0.0/rustyfile-1.0.0-x86_64-unknown-linux-gnu.tar.gz | tar xz
./rustyfile --root /srv/files

# From source:
make build
./target/release/rustyfile --root /srv/files
```

Single binary. Single container. No nginx sidecar. No separate frontend process.
