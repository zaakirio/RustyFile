# Distribution Pipeline Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship RustyFile as a single binary with embedded frontend, multi-arch Docker image on GHCR, and pre-built binaries via GitHub Releases.

**Architecture:** rust-embed compiles the Vite build output into the Rust binary at compile time. Axum serves embedded assets via a fallback handler (API routes take priority). A 3-stage Dockerfile (node/pnpm -> rust -> alpine) produces a ~15-20MB multi-arch image. Two GitHub Actions workflows handle Docker/GHCR and binary releases on tag push.

**Tech Stack:** rust-embed v8, pnpm, Axum fallback handler, Docker Buildx + QEMU, GitHub Actions, `cross` (cross-rs)

**Spec:** `docs/specs/2026-04-07-distribution-pipeline-design.md`

---

## File Map

### New files

| File | Responsibility |
|---|---|
| `src/frontend.rs` | Axum handler: serve embedded SPA assets with cache-control and SPA catch-all |
| `Makefile` | Convenience targets: dev, build, test, lint, docker, clean |
| `frontend/.npmrc` | pnpm config: strict hoisting |
| `.github/workflows/docker.yml` | Multi-arch Docker build + GHCR publish on tag |
| `.github/workflows/release.yml` | Pre-built binaries + GitHub Release on tag |
| `tests/frontend_test.rs` | Integration tests for frontend serving |

### Modified files

| File | Change |
|---|---|
| `Cargo.toml` | Add rust-embed (optional dep), embed-frontend feature flag |
| `src/lib.rs:1` | Add `pub mod frontend;` |
| `src/api/mod.rs:100-108` | Add `.fallback()` to router |
| `Dockerfile` | Replace 2-stage with 3-stage (add node/pnpm stage) |
| `.dockerignore` | Add `frontend/node_modules/` |
| `.gitignore` | Add `frontend/dist/` entry (already ignored via `/docs/` but be explicit) |
| `.github/workflows/ci.yml` | Add frontend dist stub step before Rust steps |

### Deleted files

| File | Reason |
|---|---|
| `frontend/package-lock.json` | Replaced by pnpm-lock.yaml |

---

## Task 1: Migrate npm to pnpm

**Files:**
- Delete: `frontend/package-lock.json`
- Create: `frontend/pnpm-lock.yaml` (generated)
- Create: `frontend/.npmrc`

- [ ] **Step 1: Install pnpm globally (if not already present)**

Run:
```bash
npm install -g pnpm
```

Verify:
```bash
pnpm --version
```
Expected: a version number (9.x or 10.x)

- [ ] **Step 2: Create `frontend/.npmrc`**

```ini
shamefully-hoist=false
```

- [ ] **Step 3: Import lockfile and install**

```bash
cd frontend
pnpm import
rm package-lock.json
pnpm install --frozen-lockfile
```

Expected: `pnpm-lock.yaml` created, `node_modules/` populated, no errors.

- [ ] **Step 4: Verify the build works identically**

```bash
cd frontend
pnpm run build
```

Expected: `frontend/dist/` contains `index.html`, `assets/` with hashed JS/CSS files. No errors.

- [ ] **Step 5: Verify lint works**

```bash
cd frontend
pnpm lint
```

Expected: clean output, no errors.

- [ ] **Step 6: Commit**

```bash
git add frontend/.npmrc frontend/pnpm-lock.yaml
git rm frontend/package-lock.json
git commit -m "chore: migrate frontend from npm to pnpm

Strict dependency resolution, faster installs, better Docker layer caching."
```

---

## Task 2: Add rust-embed dependency and feature flag

**Files:**
- Modify: `Cargo.toml`

- [ ] **Step 1: Add feature flag and dependency to `Cargo.toml`**

After the existing `[dependencies]` section (line 31, after `dashmap = "6"`), add nothing — the dependency goes inline. Modify the file as follows:

In `Cargo.toml`, add at the top level (after `[package]` block, before `[dependencies]`):

```toml
[features]
default = ["embed-frontend"]
embed-frontend = ["dep:rust-embed"]
```

In the `[dependencies]` section, add:

```toml
rust-embed = { version = "8", features = ["compression"], optional = true }
```

- [ ] **Step 2: Verify it compiles without frontend**

```bash
cargo check --no-default-features
```

Expected: compiles successfully (rust-embed is optional, not used in any code yet).

- [ ] **Step 3: Create a stub `frontend/dist/index.html` for development compilation**

The `frontend/dist/` directory must exist for rust-embed to compile when the feature is enabled. If you haven't built the frontend yet:

```bash
mkdir -p frontend/dist
echo '<!DOCTYPE html><html><body>stub</body></html>' > frontend/dist/index.html
```

- [ ] **Step 4: Verify it compiles with the default feature**

```bash
cargo check
```

Expected: compiles successfully. rust-embed finds `frontend/dist/` and embeds its contents.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml Cargo.lock
git commit -m "feat: add rust-embed with embed-frontend feature flag"
```

---

## Task 3: Implement frontend static handler

**Files:**
- Create: `src/frontend.rs`
- Modify: `src/lib.rs:1`

- [ ] **Step 1: Add module declaration to `src/lib.rs`**

Add `pub mod frontend;` so the file reads:

```rust
pub mod api;
pub mod config;
pub mod db;
pub mod error;
pub mod frontend;
pub mod services;
pub mod state;
```

- [ ] **Step 2: Create `src/frontend.rs`**

```rust
use axum::http::{header, StatusCode, Uri};
use axum::response::{IntoResponse, Response};

#[cfg(feature = "embed-frontend")]
mod embedded {
    use super::*;
    use rust_embed::Embed;

    #[derive(Embed)]
    #[folder = "frontend/dist"]
    struct Assets;

    /// Serve embedded frontend assets with SPA catch-all.
    ///
    /// - Exact file match in `frontend/dist/` -> serve with Content-Type + Cache-Control
    /// - Path without file extension (SPA route) -> serve index.html
    /// - Path with extension but no match -> 404
    pub async fn static_handler(uri: Uri) -> Response {
        let path = uri.path().trim_start_matches('/');

        if path.is_empty() {
            return serve_index();
        }

        match Assets::get(path) {
            Some(file) => serve_file(path, &file.data),
            None => {
                // No dot = SPA route -> serve index.html for client-side routing
                // Has dot = actual missing asset -> 404
                if path.contains('.') {
                    StatusCode::NOT_FOUND.into_response()
                } else {
                    serve_index()
                }
            }
        }
    }

    fn serve_index() -> Response {
        match Assets::get("index.html") {
            Some(file) => (
                StatusCode::OK,
                [
                    (header::CONTENT_TYPE, "text/html; charset=utf-8"),
                    (header::CACHE_CONTROL, "no-cache"),
                ],
                file.data.to_vec(),
            )
                .into_response(),
            None => StatusCode::NOT_FOUND.into_response(),
        }
    }

    fn serve_file(path: &str, data: &[u8]) -> Response {
        let mime = mime_guess::from_path(path)
            .first_or_octet_stream()
            .to_string();

        // Vite hashed assets (assets/ dir) are immutable — cache aggressively.
        // Everything else (index.html, favicon, etc.) must revalidate.
        let cache_control = if path.starts_with("assets/") {
            "public, max-age=31536000, immutable"
        } else {
            "no-cache"
        };

        (
            StatusCode::OK,
            [
                (header::CONTENT_TYPE, mime.as_str()),
                (header::CACHE_CONTROL, cache_control),
            ],
            data.to_vec(),
        )
            .into_response()
    }
}

#[cfg(not(feature = "embed-frontend"))]
mod embedded {
    use super::*;

    pub async fn static_handler(_uri: Uri) -> Response {
        (
            StatusCode::NOT_FOUND,
            "Frontend not embedded. Build with: cd frontend && pnpm build && cd .. && cargo build",
        )
            .into_response()
    }
}

pub use embedded::static_handler;
```

- [ ] **Step 3: Verify it compiles**

```bash
cargo check
```

Expected: compiles with no errors.

- [ ] **Step 4: Verify it compiles without the feature**

```bash
cargo check --no-default-features
```

Expected: compiles with no errors (uses the stub handler).

- [ ] **Step 5: Commit**

```bash
git add src/frontend.rs src/lib.rs
git commit -m "feat: add frontend static handler with SPA catch-all

Serves embedded Vite assets via rust-embed. Hashed assets get immutable
cache headers. Non-file paths fall through to index.html for React Router.
Compiles to a no-op 404 when embed-frontend feature is disabled."
```

---

## Task 4: Integrate frontend handler into router

**Files:**
- Modify: `src/api/mod.rs:100-108`

- [ ] **Step 1: Add the fallback to the router**

In `src/api/mod.rs`, change the router construction (lines 100-108) from:

```rust
    let app = Router::new()
        .nest("/api", download_routes)
        .nest("/api", cached_api_routes)
        .layer(trace_layer)
        .layer(CompressionLayer::new())
        .layer(cors)
        // Global body size limit — configurable, defaults to 50 MB.
        .layer(DefaultBodyLimit::max(max_upload))
        .with_state(state);
```

to:

```rust
    let app = Router::new()
        .nest("/api", download_routes)
        .nest("/api", cached_api_routes)
        .fallback(crate::frontend::static_handler)
        .layer(trace_layer)
        .layer(CompressionLayer::new())
        .layer(cors)
        // Global body size limit — configurable, defaults to 50 MB.
        .layer(DefaultBodyLimit::max(max_upload))
        .with_state(state);
```

- [ ] **Step 2: Verify it compiles**

```bash
cargo check
```

Expected: compiles with no errors.

- [ ] **Step 3: Run existing tests to verify no regressions**

```bash
cargo test
```

Expected: all 23 existing tests pass. The fallback handler does not interfere with `/api/*` routes.

- [ ] **Step 4: Commit**

```bash
git add src/api/mod.rs
git commit -m "feat: wire frontend fallback into Axum router

API routes take priority via nest(). All other paths fall through to
the embedded SPA handler for frontend serving."
```

---

## Task 5: Write frontend serving integration tests

**Files:**
- Create: `tests/frontend_test.rs`

- [ ] **Step 1: Create `tests/frontend_test.rs`**

```rust
mod helpers;

use helpers::TestApp;

#[tokio::test]
async fn root_serves_index_html() {
    let app = TestApp::spawn().await;

    let resp = app
        .client
        .get(app.url("/"))
        .send()
        .await
        .expect("Failed to send request");

    assert_eq!(resp.status(), 200);
    let content_type = resp
        .headers()
        .get("content-type")
        .expect("Missing content-type")
        .to_str()
        .unwrap();
    assert!(
        content_type.contains("text/html"),
        "Expected text/html, got {content_type}"
    );

    let body = resp.text().await.unwrap();
    assert!(body.contains("<!DOCTYPE html>") || body.contains("<!doctype html>"));
}

#[tokio::test]
async fn spa_route_serves_index_html() {
    let app = TestApp::spawn().await;

    // React Router paths like /browse/foo should return index.html
    let resp = app
        .client
        .get(app.url("/browse/some/path"))
        .send()
        .await
        .expect("Failed to send request");

    assert_eq!(resp.status(), 200);
    let content_type = resp
        .headers()
        .get("content-type")
        .expect("Missing content-type")
        .to_str()
        .unwrap();
    assert!(
        content_type.contains("text/html"),
        "Expected text/html for SPA route, got {content_type}"
    );
}

#[tokio::test]
async fn missing_asset_returns_404() {
    let app = TestApp::spawn().await;

    let resp = app
        .client
        .get(app.url("/nonexistent.js"))
        .send()
        .await
        .expect("Failed to send request");

    assert_eq!(resp.status(), 404);
}

#[tokio::test]
async fn api_routes_still_work() {
    let app = TestApp::spawn().await;

    let resp = app
        .client
        .get(app.url("/api/health"))
        .send()
        .await
        .expect("Failed to send request");

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["status"], "ok");
}
```

- [ ] **Step 2: Run the tests**

```bash
cargo test --test frontend_test
```

Expected: all 4 tests pass. `root_serves_index_html` and `spa_route_serves_index_html` serve the stub (or real) index.html from `frontend/dist/`. `missing_asset_returns_404` gets 404. `api_routes_still_work` confirms the fallback doesn't shadow API routes.

Note: these tests rely on `frontend/dist/index.html` existing. The stub from Task 2 Step 3 is sufficient. In CI, this stub is created before Rust steps.

- [ ] **Step 3: Run the full test suite**

```bash
cargo test
```

Expected: all tests pass (existing 23 + new 4 = 27).

- [ ] **Step 4: Commit**

```bash
git add tests/frontend_test.rs
git commit -m "test: add integration tests for frontend serving

Covers root path, SPA catch-all routes, missing asset 404, and
verification that API routes are not shadowed by the fallback."
```

---

## Task 6: Update Dockerfile to 3-stage build

**Files:**
- Modify: `Dockerfile`
- Modify: `.dockerignore`

- [ ] **Step 1: Replace the Dockerfile**

Replace the entire contents of `Dockerfile` with:

```dockerfile
# Stage 1: Build frontend
FROM node:22-alpine AS frontend
RUN npm install -g pnpm
WORKDIR /app/frontend
COPY frontend/pnpm-lock.yaml .
RUN pnpm fetch --frozen-lockfile
COPY frontend/package.json frontend/.npmrc ./
RUN pnpm install --offline --frozen-lockfile
COPY frontend/ .
RUN pnpm run build

# Stage 2: Build Rust binary
FROM rust:1.83-alpine AS builder
RUN apk add --no-cache musl-dev
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo 'fn main(){}' > src/main.rs && cargo build --release 2>/dev/null ; rm -rf src
COPY src/ src/
COPY migrations/ migrations/
COPY --from=frontend /app/frontend/dist frontend/dist
RUN cargo build --release

# Stage 3: Runtime
FROM alpine:3.21
RUN apk add --no-cache ca-certificates && adduser -D -u 1000 rustyfile
COPY --from=builder /app/target/release/rustyfile /usr/local/bin/rustyfile

USER rustyfile
ENV RUSTYFILE_HOST=0.0.0.0
ENV RUSTYFILE_PORT=80
ENV RUSTYFILE_ROOT=/data
ENV RUSTYFILE_DATA_DIR=/config

EXPOSE 80
VOLUME ["/data", "/config"]

HEALTHCHECK --interval=30s --timeout=3s CMD wget -q --spider http://localhost:80/api/health || exit 1

ENTRYPOINT ["rustyfile"]
```

- [ ] **Step 2: Add `frontend/node_modules/` to `.dockerignore`**

Append to `.dockerignore`:

```
frontend/node_modules/
frontend/dist/
```

The full `.dockerignore` should be:

```
target/
tmp-data/
rustyfile-data/
test-data/
*.db
.git/
docs/
tests/
frontend/node_modules/
frontend/dist/
```

`frontend/dist/` is excluded because the Dockerfile builds it fresh in Stage 1.

- [ ] **Step 3: Verify Docker build works**

```bash
docker build -t rustyfile:test .
```

Expected: all 3 stages complete successfully. Image is built.

- [ ] **Step 4: Smoke test the Docker image**

```bash
docker run --rm -d --name rustyfile-test -p 9090:80 -v /tmp/rf-test:/data rustyfile:test
sleep 2
curl -s http://localhost:9090/api/health
curl -s http://localhost:9090/ | head -5
docker stop rustyfile-test
```

Expected: `/api/health` returns `{"status":"ok"}`. `/` returns the HTML with `<!DOCTYPE html>` and RustyFile content.

- [ ] **Step 5: Commit**

```bash
git add Dockerfile .dockerignore
git commit -m "feat: 3-stage Dockerfile — pnpm frontend + Rust + Alpine runtime

Adds Node/pnpm build stage. Frontend dist is embedded into the Rust
binary via rust-embed. Optimized layer caching with pnpm fetch and
cargo dependency pre-compilation."
```

---

## Task 7: Update .gitignore and CI workflow

**Files:**
- Modify: `.gitignore`
- Modify: `.github/workflows/ci.yml`

- [ ] **Step 1: Ensure `frontend/dist/` is explicitly in `.gitignore`**

The current `.gitignore` has `node_modules/` which catches `frontend/node_modules/`. Add an explicit `frontend/dist/` entry. After the `# Node` section, the file should have:

```
# Node
node_modules/
frontend/dist/
```

- [ ] **Step 2: Update CI workflow to create frontend dist stub**

Replace `.github/workflows/ci.yml` with:

```yaml
name: CI
on: [push, pull_request]

jobs:
  check:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: clippy, rustfmt
      - uses: Swatinem/rust-cache@v2
      - name: Create frontend dist stub
        run: mkdir -p frontend/dist && echo '<!DOCTYPE html><html><body></body></html>' > frontend/dist/index.html
      - name: Format check
        run: cargo fmt --check
      - name: Clippy
        run: cargo clippy -- -D warnings
      - name: Tests
        run: cargo test
```

The stub ensures rust-embed compiles without needing a full Node.js build in CI for every PR.

- [ ] **Step 3: Commit**

```bash
git add .gitignore .github/workflows/ci.yml
git commit -m "chore: add frontend dist stub to CI, update gitignore

CI creates a minimal index.html stub so rust-embed compiles without
a full frontend build. Full build is validated by the Docker workflow
on release tags."
```

---

## Task 8: Create Makefile

**Files:**
- Create: `Makefile`

- [ ] **Step 1: Create `Makefile`**

```makefile
.PHONY: dev build test lint docker clean

# Start frontend dev server + backend in parallel.
# Vite proxies /api to the Rust backend on port 8080.
dev:
	cd frontend && pnpm dev &
	cargo run -- --root ./test-data --data-dir ./tmp-data

# Full production build: frontend first, then Rust with embedding.
build:
	cd frontend && pnpm install && pnpm run build
	cargo build --release

# Run Rust test suite.
test:
	cargo test

# Lint both frontend and backend.
lint:
	cargo fmt --check
	cargo clippy -- -D warnings
	cd frontend && pnpm lint

# Build Docker image locally.
docker:
	docker buildx build -t rustyfile:latest .

# Remove all build artifacts.
clean:
	cargo clean
	rm -rf frontend/dist frontend/node_modules
```

Note: Makefile indentation must use tabs, not spaces.

- [ ] **Step 2: Verify `make test` works**

```bash
make test
```

Expected: all tests pass.

- [ ] **Step 3: Verify `make lint` works**

```bash
make lint
```

Expected: no errors from cargo fmt, clippy, or pnpm lint.

- [ ] **Step 4: Verify `make build` works**

```bash
make build
```

Expected: frontend builds to `frontend/dist/`, then Rust compiles a release binary at `target/release/rustyfile`.

- [ ] **Step 5: Commit**

```bash
git add Makefile
git commit -m "chore: add Makefile for dev/build/test/lint/docker/clean

Convenience wrapper over cargo and pnpm commands. make dev starts
both Vite and the Rust backend. make build produces a full release
binary with embedded frontend."
```

---

## Task 9: Create Docker + GHCR workflow

**Files:**
- Create: `.github/workflows/docker.yml`

- [ ] **Step 1: Create `.github/workflows/docker.yml`**

```yaml
name: Docker

on:
  push:
    tags: ["v*.*.*"]

env:
  REGISTRY: ghcr.io
  IMAGE_NAME: ${{ github.repository }}

jobs:
  docker:
    runs-on: ubuntu-latest
    permissions:
      contents: read
      packages: write
      attestations: write
      id-token: write

    steps:
      - uses: actions/checkout@v4

      - name: Log in to GHCR
        uses: docker/login-action@v3
        with:
          registry: ${{ env.REGISTRY }}
          username: ${{ github.actor }}
          password: ${{ secrets.GITHUB_TOKEN }}

      - name: Set up QEMU
        uses: docker/setup-qemu-action@v3

      - name: Set up Docker Buildx
        uses: docker/setup-buildx-action@v3

      - name: Extract metadata
        id: meta
        uses: docker/metadata-action@v5
        with:
          images: ${{ env.REGISTRY }}/${{ env.IMAGE_NAME }}
          tags: |
            type=semver,pattern={{version}}
            type=semver,pattern={{major}}.{{minor}}
            type=semver,pattern={{major}}
            type=sha

      - name: Build and push
        id: push
        uses: docker/build-push-action@v6
        with:
          context: .
          platforms: linux/amd64,linux/arm64
          push: true
          tags: ${{ steps.meta.outputs.tags }}
          labels: ${{ steps.meta.outputs.labels }}
          cache-from: type=gha
          cache-to: type=gha,mode=max

      - name: Generate attestation
        uses: actions/attest-build-provenance@v2
        with:
          subject-name: ${{ env.REGISTRY }}/${{ env.IMAGE_NAME }}
          subject-digest: ${{ steps.push.outputs.digest }}
          push-to-registry: true
```

- [ ] **Step 2: Commit**

```bash
git add .github/workflows/docker.yml
git commit -m "ci: add multi-arch Docker build with GHCR publishing

Triggers on version tags. Builds linux/amd64 and linux/arm64 via
QEMU + Buildx. Publishes to ghcr.io with semver tags. Includes
build provenance attestation."
```

---

## Task 10: Create release workflow for pre-built binaries

**Files:**
- Create: `.github/workflows/release.yml`

- [ ] **Step 1: Create `.github/workflows/release.yml`**

```yaml
name: Release

on:
  push:
    tags: ["v*.*.*"]

permissions:
  contents: write

jobs:
  build-frontend:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - uses: actions/setup-node@v4
        with:
          node-version: "22"

      - name: Install pnpm
        run: npm install -g pnpm

      - name: Install dependencies
        working-directory: frontend
        run: pnpm install --frozen-lockfile

      - name: Build frontend
        working-directory: frontend
        run: pnpm run build

      - uses: actions/upload-artifact@v4
        with:
          name: frontend-dist
          path: frontend/dist/
          retention-days: 1

  build-binaries:
    needs: build-frontend
    runs-on: ${{ matrix.platform.os }}
    strategy:
      fail-fast: false
      matrix:
        platform:
          - name: linux-amd64
            os: ubuntu-latest
            target: x86_64-unknown-linux-gnu
            use_cross: false

          - name: linux-arm64
            os: ubuntu-latest
            target: aarch64-unknown-linux-gnu
            use_cross: true

          - name: macos-amd64
            os: macos-latest
            target: x86_64-apple-darwin
            use_cross: false

          - name: macos-arm64
            os: macos-latest
            target: aarch64-apple-darwin
            use_cross: false

    steps:
      - uses: actions/checkout@v4

      - uses: actions/download-artifact@v4
        with:
          name: frontend-dist
          path: frontend/dist/

      - uses: dtolnay/rust-toolchain@stable
        with:
          targets: ${{ matrix.platform.target }}

      - uses: Swatinem/rust-cache@v2
        with:
          key: ${{ matrix.platform.target }}

      - name: Install cross
        if: matrix.platform.use_cross
        run: cargo install cross --git https://github.com/cross-rs/cross

      - name: Build with cross
        if: matrix.platform.use_cross
        run: cross build --release --target ${{ matrix.platform.target }}

      - name: Build natively
        if: ${{ !matrix.platform.use_cross }}
        run: cargo build --release --target ${{ matrix.platform.target }}

      - name: Package binary
        shell: bash
        run: |
          VERSION="${GITHUB_REF#refs/tags/v}"
          ARCHIVE="rustyfile-${VERSION}-${{ matrix.platform.target }}"
          mkdir -p "dist/${ARCHIVE}"
          if [ -f "target/${{ matrix.platform.target }}/release/rustyfile" ]; then
            cp "target/${{ matrix.platform.target }}/release/rustyfile" "dist/${ARCHIVE}/"
          else
            cp "target/${{ matrix.platform.target }}/release/rustyfile.exe" "dist/${ARCHIVE}/"
          fi
          cp README.md LICENSE "dist/${ARCHIVE}/" 2>/dev/null || true
          cd dist && tar -czf "${ARCHIVE}.tar.gz" "${ARCHIVE}"
          shasum -a 256 "${ARCHIVE}.tar.gz" > "${ARCHIVE}.tar.gz.sha256"

      - uses: actions/upload-artifact@v4
        with:
          name: rustyfile-${{ matrix.platform.target }}
          path: dist/*.tar.gz*
          if-no-files-found: error

  release:
    needs: build-binaries
    runs-on: ubuntu-latest
    steps:
      - uses: actions/download-artifact@v4
        with:
          path: artifacts
          merge-multiple: true

      - name: Create GitHub Release
        uses: softprops/action-gh-release@v2
        with:
          files: artifacts/*
          generate_release_notes: true
```

- [ ] **Step 2: Commit**

```bash
git add .github/workflows/release.yml
git commit -m "ci: add release workflow for pre-built binaries

Triggers on version tags. Builds frontend once, then cross-compiles
Rust for linux-amd64, linux-arm64, macos-amd64, macos-arm64. Creates
GitHub Release with tar.gz archives and SHA256 checksums."
```

---

## Task 11: End-to-end verification

**Files:** None (verification only)

- [ ] **Step 1: Run full test suite**

```bash
cargo test
```

Expected: all tests pass (original 23 + 4 frontend tests).

- [ ] **Step 2: Run full lint**

```bash
make lint
```

Expected: no errors from cargo fmt, clippy, or pnpm lint.

- [ ] **Step 3: Build full production binary**

```bash
make build
```

Expected: frontend builds, then Rust compiles with embedded assets.

- [ ] **Step 4: Test the production binary serves frontend**

```bash
./target/release/rustyfile --root /tmp/rf-test --data-dir /tmp/rf-data --port 9090 &
sleep 1
curl -s http://localhost:9090/api/health
curl -s -o /dev/null -w "%{http_code} %{content_type}" http://localhost:9090/
curl -s -o /dev/null -w "%{http_code}" http://localhost:9090/browse/test
curl -s -o /dev/null -w "%{http_code}" http://localhost:9090/nonexistent.js
kill %1
```

Expected:
- `/api/health` -> `{"status":"ok"}`
- `/` -> `200 text/html; charset=utf-8`
- `/browse/test` -> `200` (SPA catch-all)
- `/nonexistent.js` -> `404`

- [ ] **Step 5: Docker build and smoke test**

```bash
make docker
docker run --rm -d --name rf-e2e -p 9091:80 -v /tmp/rf-e2e:/data rustyfile:latest
sleep 2
curl -s http://localhost:9091/api/health
curl -s http://localhost:9091/ | head -3
docker stop rf-e2e
```

Expected: health returns ok, root returns HTML.

- [ ] **Step 6: Verify backend-only build**

```bash
cargo build --no-default-features
```

Expected: compiles successfully without rust-embed or frontend assets.

- [ ] **Step 7: Final commit (if any fixups needed)**

If any fixes were required during verification, commit them:

```bash
git add -A
git commit -m "fix: address issues found during e2e verification"
```

If nothing needed fixing, skip this step.
