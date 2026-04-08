use std::sync::Arc;

use tokio::net::TcpListener;
use tokio::signal;
use tracing_subscriber::EnvFilter;

use rustyfile::api;
use rustyfile::config::AppConfig;
use rustyfile::db;
use rustyfile::state::{AppState, SetupGuard};

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

    // Clean up orphaned temp files from interrupted writes without delaying startup.
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

    // Canonicalize once at startup to avoid per-request syscalls.
    let canonical_root = std::path::PathBuf::from(&config.root)
        .canonicalize()
        .expect("Root directory must exist and be accessible");
    tracing::info!(canonical_root = %canonical_root.display(), "Root path canonicalized");

    let login_limiter = rustyfile::state::new_login_limiter(
        std::num::NonZeroU32::new(10).unwrap(),
        15 * 60,
    );

    // Pre-hash a dummy password for constant-time login failure.
    let dummy_hash = {
        use argon2::password_hash::SaltString;
        use argon2::PasswordHasher;
        let salt = SaltString::generate(&mut rand::rngs::OsRng);
        argon2::Argon2::default()
            .hash_password(b"rustyfile_dummy_timing_password", &salt)
            .expect("Failed to hash dummy password")
            .to_string()
    };

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
            .watch(&watch_root, RecursiveMode::NonRecursive)
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

    let state = AppState {
        db: pool,
        config: config.clone(),
        setup_guard,
        jwt_secret,
        canonical_root,
        login_limiter,
        dummy_hash,
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

    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
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
