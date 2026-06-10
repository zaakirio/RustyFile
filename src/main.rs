use std::collections::HashSet;
use std::sync::Arc;

use tokio::net::TcpListener;
use tokio::signal;
use tokio_util::sync::CancellationToken;
use tracing_subscriber::EnvFilter;

use rustyfile::api;
use rustyfile::config::AppConfig;
use rustyfile::db;
use rustyfile::services::search_index::SearchIndex;
use rustyfile::state::{AppState, SetupGuard};

/// Login attempts allowed per IP within the window.
const LOGIN_LIMITER_BURST: u32 = 10;
const LOGIN_LIMITER_WINDOW_SECS: u64 = 15 * 60;

/// Fallback API rate limit when the configured value is zero.
const API_LIMITER_FALLBACK: u32 = 60;
const API_LIMITER_WINDOW_SECS: u64 = 60;

const DIR_CACHE_CAPACITY: u64 = 1000;
const DIR_CACHE_TTL_SECS: u64 = 30;

/// Max concurrent thumbnail generations.
const THUMB_WORKER_CONCURRENCY: usize = 4;
/// Max thumbnail dimension in pixels.
const THUMB_MAX_DIMENSION: u32 = 300;

/// Max concurrent ffmpeg transcodes.
const TRANSCODER_CONCURRENCY: usize = 2;
/// HLS segment duration in seconds.
const TRANSCODER_SEGMENT_SECS: u32 = 10;

const HLS_SOURCES_CAPACITY: u64 = 1000;
/// HLS source mappings expire after this much idle time.
const HLS_SOURCES_TIME_TO_IDLE_SECS: u64 = 2 * 60 * 60;

const TOKEN_BLOCKLIST_CAPACITY: u64 = 10_000;

/// The filesystem watcher keeps the index fresh in real time; the periodic
/// full reindex is a safety net for events missed under load.
const FULL_REINDEX_INTERVAL_SECS: u64 = 6 * 60 * 60;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = AppConfig::load()?;
    init_logging(&config);

    tracing::info!("Starting RustyFile v{}", env!("CARGO_PKG_VERSION"));
    config.log_security_warnings();

    tokio::fs::create_dir_all(&config.root).await?;
    tokio::fs::create_dir_all(&config.data_dir).await?;
    let tus_upload_dir = std::path::PathBuf::from(&config.cache_dir).join("uploads");
    tokio::fs::create_dir_all(&tus_upload_dir).await?;
    tracing::info!(root = %config.root, data_dir = %config.data_dir, cache_dir = %config.cache_dir, "Directories ensured");

    let cleanup_root = config.root.clone();
    tokio::spawn(async move {
        cleanup_orphan_temp_files(&cleanup_root).await;
    });

    let pool = db::create_pool(&config)?;
    db::run_migrations(&pool).await?;

    let setup_guard = Arc::new(SetupGuard::new(config.setup_timeout_minutes));
    let admin_exists = db::user_repo::admin_exists(&pool).await?;
    if admin_exists {
        setup_guard.mark_complete();
        tracing::info!("Admin account found — setup already complete");
    } else {
        tracing::warn!(
            "No admin account found — setup wizard available for {} minutes",
            config.setup_timeout_minutes
        );
    }

    let jwt_secret = db::get_or_create_jwt_secret(&pool).await?;

    // Avoid per-request syscalls.
    let canonical_root = Arc::new(
        std::path::PathBuf::from(&config.root)
            .canonicalize()
            .expect("Root directory must exist and be accessible"),
    );
    tracing::info!(canonical_root = %canonical_root.display(), "Root path canonicalized");

    let login_limiter = rustyfile::state::new_rate_limiter(
        std::num::NonZeroU32::new(LOGIN_LIMITER_BURST).unwrap(),
        LOGIN_LIMITER_WINDOW_SECS,
    );

    // Constant-time login failure (timing-attack mitigation).
    let dummy_hash: Arc<str> = {
        use argon2::password_hash::SaltString;
        use argon2::PasswordHasher;
        let salt = SaltString::generate(&mut rand::rngs::OsRng);
        argon2::Argon2::default()
            .hash_password(b"rustyfile_dummy_timing_password", &salt)
            .expect("Failed to hash dummy password")
            .to_string()
            .into()
    };

    // Parse blocked upload extensions once at startup.
    let blocked_extensions: Arc<HashSet<String>> = Arc::new(
        config
            .blocked_upload_extensions
            .split(',')
            .map(|s| s.trim().to_lowercase())
            .filter(|s| !s.is_empty())
            .collect(),
    );

    let dir_cache =
        rustyfile::services::cache::DirCache::new(DIR_CACHE_CAPACITY, DIR_CACHE_TTL_SECS);

    let thumb_cache_dir = std::path::PathBuf::from(&config.data_dir)
        .join("cache")
        .join("thumbs");
    tokio::fs::create_dir_all(&thumb_cache_dir).await?;
    let thumb_worker = rustyfile::services::thumbnail::ThumbWorker::new(
        THUMB_WORKER_CONCURRENCY,
        thumb_cache_dir,
        THUMB_MAX_DIMENSION,
    );

    let hls_dir = std::path::PathBuf::from(&config.data_dir)
        .join("cache")
        .join("hls");
    tokio::fs::create_dir_all(&hls_dir).await?;
    let transcoder = rustyfile::services::transcoder::HlsTranscoder::new(
        hls_dir,
        TRANSCODER_CONCURRENCY,
        TRANSCODER_SEGMENT_SECS,
    );
    let hls_sources: moka::future::Cache<String, std::path::PathBuf> =
        moka::future::Cache::builder()
            .max_capacity(HLS_SOURCES_CAPACITY)
            .time_to_idle(std::time::Duration::from_secs(
                HLS_SOURCES_TIME_TO_IDLE_SECS,
            ))
            .build();

    let api_limiter = rustyfile::state::new_rate_limiter(
        std::num::NonZeroU32::new(config.api_rate_limit)
            .unwrap_or(std::num::NonZeroU32::new(API_LIMITER_FALLBACK).unwrap()),
        API_LIMITER_WINDOW_SECS,
    );

    let token_blocklist: moka::future::Cache<String, ()> = moka::future::Cache::builder()
        .max_capacity(TOKEN_BLOCKLIST_CAPACITY)
        .time_to_live(std::time::Duration::from_secs(
            config.jwt_expiry_hours * 3600,
        ))
        .build();

    let search_indexer =
        rustyfile::services::search_index::SearchIndexer::new(pool.clone(), canonical_root.clone());

    let (fs_events, _) =
        tokio::sync::broadcast::channel(rustyfile::services::watcher::FS_EVENTS_CAPACITY);

    let config = Arc::new(config);

    let state = AppState {
        db: pool,
        config: config.clone(),
        setup_guard,
        jwt_secret: jwt_secret.into(),
        canonical_root,
        login_limiter,
        dummy_hash,
        dir_cache,
        thumb_worker,
        transcoder,
        hls_sources,
        search_indexer,
        token_blocklist,
        api_limiter,
        blocked_extensions,
        fs_events,
    };

    // ── Graceful shutdown token ───────────────────────────────────────────────
    let shutdown_token = CancellationToken::new();

    // Full reindex at startup (the interval's first tick fires immediately),
    // then periodically as a safety net for missed watcher events.
    {
        let indexer = state.search_indexer.clone();
        let token = shutdown_token.clone();
        tokio::spawn(async move {
            let mut interval =
                tokio::time::interval(std::time::Duration::from_secs(FULL_REINDEX_INTERVAL_SECS));
            loop {
                tokio::select! {
                    _ = token.cancelled() => {
                        tracing::info!("Search reindex task shutting down");
                        break;
                    }
                    _ = interval.tick() => {
                        if let Err(e) = indexer.full_reindex().await {
                            tracing::error!("Search index build failed: {e:#}");
                        }
                    }
                }
            }
        });
    }

    // Debounced filesystem watcher: cache invalidation, search indexing, and
    // live SSE events.
    rustyfile::services::watcher::spawn(&state, shutdown_token.clone());

    // ── Background cleanup tasks ──────────────────────────────────────────────
    api::tus::spawn_cleanup_task(
        state.db.clone(),
        state.config.cache_dir.clone(),
        shutdown_token.clone(),
    );

    // Expired share links 404 lazily; this daily sweep removes dead rows.
    api::shares::spawn_cleanup_task(state.db.clone(), shutdown_token.clone());

    // HLS segment cleanup (every 30 min, removes dirs older than 2h).
    {
        let hls_dir = state.transcoder.segment_dir().to_path_buf();
        let token = shutdown_token.clone();
        tokio::spawn(rustyfile::services::transcoder::cleanup_hls_segments(
            hls_dir, token,
        ));
    }

    // Thumbnail cleanup (every 2h, removes files older than 7 days).
    {
        let thumb_dir = state.thumb_worker.cache_dir().to_path_buf();
        let token = shutdown_token.clone();
        tokio::spawn(rustyfile::services::thumbnail::cleanup_thumbnails(
            thumb_dir, token,
        ));
    }

    // Rate-limiter key eviction (every 5 min, drops idle per-IP buckets).
    {
        let limiters = vec![state.login_limiter.clone(), state.api_limiter.clone()];
        let token = shutdown_token.clone();
        tokio::spawn(rustyfile::state::rate_limiter_maintenance(limiters, token));
    }

    let app = api::build_router(state);

    let addr = format!("{}:{}", config.host, config.port);
    let listener = TcpListener::bind(&addr).await?;
    tracing::info!("Listening on http://{addr}");

    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal(shutdown_token))
    .await?;

    tracing::info!("Server shut down gracefully");
    Ok(())
}

fn init_logging(config: &AppConfig) {
    let env_filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(&config.log_level));

    match config.log_format.as_str() {
        "json" => {
            tracing_subscriber::fmt()
                .with_env_filter(env_filter)
                .json()
                .init();
        }
        _ => {
            tracing_subscriber::fmt().with_env_filter(env_filter).init();
        }
    }
}

async fn cleanup_orphan_temp_files(root: &str) {
    use tokio::fs;
    let root = std::path::Path::new(root);
    let mut stack = vec![root.to_path_buf()];
    let mut count = 0u32;

    while let Some(dir) = stack.pop() {
        let Ok(mut entries) = fs::read_dir(&dir).await else {
            continue;
        };
        while let Ok(Some(entry)) = entries.next_entry().await {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.starts_with(".rustyfile_tmp_") {
                let _ = fs::remove_file(entry.path()).await;
                count += 1;
            } else if let Ok(ft) = entry.file_type().await {
                if ft.is_dir() {
                    stack.push(entry.path());
                }
            }
        }
    }
    if count > 0 {
        tracing::info!(count, "Cleaned up orphaned temp files");
    }
}

async fn shutdown_signal(shutdown_token: CancellationToken) {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("Failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("Failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {
            tracing::info!("Received SIGINT, shutting down...");
        }
        _ = terminate => {
            tracing::info!("Received SIGTERM, shutting down...");
        }
    }

    // Signal all background tasks to stop.
    shutdown_token.cancel();
}
