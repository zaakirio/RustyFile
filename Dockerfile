# Stage 1: Build frontend
FROM node:22-alpine AS frontend
RUN npm install -g pnpm
WORKDIR /app/frontend
COPY frontend/pnpm-lock.yaml .
RUN pnpm fetch --frozen-lockfile
COPY frontend/package.json frontend/.npmrc ./
RUN CI=true pnpm install --offline --frozen-lockfile
COPY frontend/ .
RUN pnpm run build

# Stage 2: Build Rust binary
FROM rust:1.88-alpine AS builder
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
RUN apk add --no-cache ca-certificates ffmpeg \
    && adduser -D -u 1000 rustyfile \
    && mkdir -p /data /config \
    && chown rustyfile:rustyfile /data /config
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
