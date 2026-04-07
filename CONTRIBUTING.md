# Contributing to RustyFile

## Development Setup

```bash
git clone https://github.com/yourorg/rustyfile.git
cd rustyfile
cargo build          # Verify it compiles
cargo test           # Run the test suite (23 integration tests)
```

### Prerequisites

- **Rust 1.80+** -- install via [rustup](https://rustup.rs/)
- **cargo-watch** (optional) -- `cargo install cargo-watch` for auto-reload

### Running Locally

```bash
# Start dev server (creates ./test-data and ./tmp-data)
make dev

# Or manually:
cargo run -- --root ./test-data --data-dir ./tmp-data --port 3001 --log-level debug
```

Visit `http://localhost:3001` -- the setup wizard will prompt you to create an admin account.

## Code Style

### Formatting and Linting

```bash
cargo fmt            # Auto-format
cargo clippy -- -D warnings   # Lint (warnings are errors in CI)
```

Both run in CI on every push/PR. PRs with format or lint failures will not pass.

### Conventions

- **Error handling** -- Use `AppError` variants, never `.unwrap()` in handlers. The `?` operator is your friend.
- **DB access** -- Use `db::interact(pool, |conn| { ... }).await?` helper (not raw pool.get + interact).
- **File paths** -- Always use `file_ops::safe_resolve()` for user-provided paths. Never construct paths manually.
- **Async I/O** -- Use `tokio::fs` for file operations. Use `spawn_blocking` for CPU-bound or sync I/O work.
- **Naming** -- Follow Rust conventions: `snake_case` for functions/variables, `PascalCase` for types.
- **Comments** -- Only where the "why" isn't obvious. Don't comment "gets the user" above `get_user()`.

### Architecture Principles

- **Single responsibility** -- Each file has one job. Handlers are thin (extract, call service, respond).
- **DRY** -- Shared logic lives in `services/` or `db/`. Don't duplicate across handlers.
- **YAGNI** -- Don't build for hypothetical future needs. Ship what's needed now.
- **Security by default** -- All file paths validated, all downloads get CSP headers, all API responses get no-cache.

## Making Changes

### Workflow

1. **Fork and branch** -- Create a feature branch from `main`
2. **Write tests first** -- Add integration tests in `tests/` before or alongside your implementation
3. **Implement** -- Follow existing patterns in the codebase
4. **Verify** -- `cargo fmt && cargo clippy -- -D warnings && cargo test`
5. **Commit** -- Use [Conventional Commits](https://www.conventionalcommits.org/):
   - `feat: add thumbnail generation`
   - `fix: handle empty directories in listing`
   - `refactor: extract db interact helper`
   - `test: add range request edge cases`
   - `docs: update API reference`
6. **PR** -- Open a pull request with a clear description of what and why

### Adding a New API Endpoint

1. **Handler** -- Create or modify a file in `src/api/`:

```rust
async fn my_handler(
    State(state): State<AppState>,
    Extension(user): Extension<user_repo::User>,
    Path(path): Path<String>,
) -> Result<Json<Value>, AppError> {
    let resolved = file_ops::safe_resolve(&state.canonical_root, &path)?;
    // ... business logic ...
    Ok(Json(json!({ "result": "ok" })))
}
```

2. **Route** -- Register in the module's `routes()` function
3. **Auth** -- Protected routes need `route_layer(middleware::from_fn_with_state(state, require_auth))`
4. **Test** -- Add integration test using the `TestApp` helper:

```rust
#[tokio::test]
async fn my_new_endpoint_works() {
    let app = helpers::TestApp::spawn().await;
    let token = app.create_admin().await;
    // ... test assertions ...
}
```

### Adding a Database Table

1. Create a new migration: `migrations/V2__add_bookmarks.sql`
2. Add it to `run_migrations` in `src/db/mod.rs`
3. Create a repo module: `src/db/bookmark_repo.rs`
4. Use `db::interact()` for all queries

### Testing

- Integration tests live in `tests/` and use `TestApp` (spawns a real server per test)
- Each test gets isolated temp directories and a fresh database
- Tests run in parallel -- don't use hardcoded ports
- Assert HTTP status codes AND response bodies

## Project Structure

```
src/
  main.rs              -- Server startup
  config.rs            -- Configuration loading
  error.rs             -- Error types
  state.rs             -- Shared application state
  api/                 -- HTTP handlers (thin layer)
    mod.rs             -- Router wiring
    middleware/auth.rs  -- JWT middleware
  db/                  -- Database access (queries only)
    mod.rs             -- Pool + migrations + interact() helper
  services/            -- Business logic (file operations)
```

**Key principle:** `api/` handlers extract request data and return responses. `services/` contains the logic. `db/` contains the queries. Handlers should not contain SQL or file system logic directly.

## Questions?

Open an issue or start a discussion on GitHub.
