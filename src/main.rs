use std::sync::Arc;

use tokio::net::TcpListener;
use tokio::signal;
use tracing_subscriber::EnvFilter;

use rustyfile::api;
use rustyfile::config::AppConfig;
use rustyfile::db;
use rustyfile::state::{AppState, LoginRateLimiter, SetupGuard};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = AppConfig::load()?;
    init_logging(&config);

    tracing::info!("Starting RustyFile v{}", env!("CARGO_PKG_VERSION"));

    tokio::fs::create_dir_all(&config.root).await?;
    tokio::fs::create_dir_all(&config.data_dir).await?;
    // TUS upload temp directory
    let tus_upload_dir = std::path::PathBuf::from(&config.cache_dir).join("uploads");
    tokio::fs::create_dir_all(&tus_upload_dir).await?;
    tracing::info!(root = %config.root, data_dir = %config.data_dir, cache_dir = %config.cache_dir, "Directories ensured");

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

    // Canonicalize once at startup to avoid per-request syscalls.
    let canonical_root = std::path::PathBuf::from(&config.root)
        .canonicalize()
        .expect("Root directory must exist and be accessible");
    tracing::info!(canonical_root = %canonical_root.display(), "Root path canonicalized");

    let dir_cache = rustyfile::services::cache::DirCache::new(1000, 30);

    // Spawn filesystem watcher for cache invalidation
    {
        use notify::RecursiveMode;
        use notify_debouncer_full::new_debouncer;

        let dir_cache_watcher = dir_cache.clone();
        let watch_root = canonical_root.clone();

        let (tx, mut rx) = tokio::sync::mpsc::channel(256);

        let mut debouncer = new_debouncer(
            std::time::Duration::from_millis(500),
            None,
            move |result: notify_debouncer_full::DebounceEventResult| {
                let _ = tx.blocking_send(result);
            },
        )
        .expect("Failed to create filesystem watcher");

        debouncer
            .watch(&watch_root, RecursiveMode::Recursive)
            .expect("Failed to watch root directory");

        tokio::spawn(async move {
            let _debouncer = debouncer; // Keep alive
            while let Some(Ok(events)) = rx.recv().await {
                for event in events {
                    for path in &event.paths {
                        if let Some(parent) = path.parent() {
                            let key = parent.to_string_lossy().to_string();
                            dir_cache_watcher.invalidate(&key).await;
                        }
                    }
                }
            }
        });

        tracing::info!("Filesystem watcher active for cache invalidation");
    }

    let thumb_cache_dir = std::path::PathBuf::from(&config.data_dir)
        .join("cache")
        .join("thumbs");
    tokio::fs::create_dir_all(&thumb_cache_dir).await?;
    let thumb_worker = rustyfile::services::thumbnail::ThumbWorker::new(
        4, // max concurrent thumbnail generations
        thumb_cache_dir,
        300, // 300px max dimension
    );

    let hls_dir = std::path::PathBuf::from(&config.data_dir)
        .join("cache")
        .join("hls");
    tokio::fs::create_dir_all(&hls_dir).await?;
    let transcoder = rustyfile::services::transcoder::HlsTranscoder::new(hls_dir, 2, 10);
    let hls_sources: Arc<dashmap::DashMap<String, std::path::PathBuf>> =
        Arc::new(dashmap::DashMap::new());
    let login_limiter = Arc::new(LoginRateLimiter::new(
        10, // max 10 attempts
        std::time::Duration::from_secs(15 * 60), // per 15-minute window
    ));

    let state = AppState {
        db: pool,
        config: config.clone(),
        setup_guard,
        jwt_secret,
        canonical_root,
        login_limiter,
        dir_cache,
        thumb_worker,
        transcoder,
        hls_sources,
    };

    api::tus::spawn_cleanup_task(state.db.clone(), state.config.cache_dir.clone());

    let app = api::build_router(state);

    let addr = format!("{}:{}", config.host, config.port);
    let listener = TcpListener::bind(&addr).await?;
    tracing::info!("Listening on http://{addr}");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    tracing::info!("Server shut down gracefully");
    Ok(())
}

fn init_logging(config: &AppConfig) {
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(&config.log_level));

    match config.log_format.as_str() {
        "json" => {
            tracing_subscriber::fmt()
                .with_env_filter(env_filter)
                .json()
                .init();
        }
        _ => {
            tracing_subscriber::fmt()
                .with_env_filter(env_filter)
                .init();
        }
    }
}

async fn shutdown_signal() {
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
}
