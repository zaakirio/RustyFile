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
    tracing::info!(root = %config.root, data_dir = %config.data_dir, "Directories ensured");

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
