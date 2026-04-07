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
    tracing::info!(root = %config.root, data_dir = %config.data_dir, "Directories ensured");

    // Clean up orphaned temp files from interrupted writes.
    cleanup_orphan_temp_files(&config.root).await;

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

    let login_limiter = rustyfile::state::new_login_limiter(10, 15 * 60);

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

    let state = AppState {
        db: pool,
        config: config.clone(),
        setup_guard,
        jwt_secret,
        canonical_root,
        login_limiter,
        dummy_hash,
    };

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
