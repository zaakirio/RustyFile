.PHONY: dev build test lint docker clean docker-multi

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

# Build multi-arch Docker image.
docker-multi:
	docker buildx build --platform linux/amd64,linux/arm64 -t rustyfile:latest .

# Remove all build artifacts.
clean:
	cargo clean
	rm -rf tmp-data rustyfile-data frontend/dist frontend/node_modules
