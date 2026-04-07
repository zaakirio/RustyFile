.PHONY: dev build test docker docker-run clean docker-multi

dev:
	cargo run -- --root ./test-data --data-dir ./tmp-data --port 3001

build:
	cargo build --release

test:
	cargo test

docker:
	docker build -t rustyfile:latest .

docker-run: docker
	docker run -p 8080:80 -v $$(pwd)/test-data:/data rustyfile:latest

clean:
	cargo clean
	rm -rf tmp-data rustyfile-data

docker-multi:
	docker buildx build --platform linux/amd64,linux/arm64 -t rustyfile:latest .
