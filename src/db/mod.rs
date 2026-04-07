pub mod user_repo;

use deadpool_sqlite::{Config, Pool, Runtime};
use rand::Rng;
use rusqlite::params;

use crate::config::AppConfig;
use crate::error::AppError;

/// Create a deadpool-sqlite connection pool.
pub fn create_pool(config: &AppConfig) -> anyhow::Result<Pool> {
    let db_path = config.db_path();

    let cfg = Config::new(db_path);
    let pool = cfg.create_pool(Runtime::Tokio1)?;

    Ok(pool)
}

/// Run database migrations: set PRAGMAs and execute the initial schema.
pub async fn run_migrations(pool: &Pool) -> anyhow::Result<()> {
    let conn = pool.get().await?;

    conn.interact(|conn| {
        // Set recommended PRAGMAs for SQLite
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=NORMAL;
             PRAGMA foreign_keys=ON;
             PRAGMA busy_timeout=5000;",
        )?;

        // Run the initial schema migration inline
        let migration_sql = include_str!("../../migrations/V1__initial_schema.sql");
        conn.execute_batch(migration_sql)?;

        Ok::<_, rusqlite::Error>(())
    })
    .await
    .map_err(|e| anyhow::anyhow!("Migration interact error: {e}"))?
    .map_err(|e: rusqlite::Error| anyhow::anyhow!("Migration SQL error: {e}"))?;

    tracing::info!("Database migrations applied successfully");
    Ok(())
}

/// Retrieve or create the JWT signing secret stored in the settings table.
///
/// If a secret already exists, it is returned. Otherwise, 64 random bytes
/// are generated, stored, and returned.
pub async fn get_or_create_jwt_secret(pool: &Pool) -> Result<Vec<u8>, AppError> {
    let conn = pool.get().await.map_err(|e| AppError::Internal(e.to_string()))?;

    let secret = conn
        .interact(|conn| {
            // Try to fetch existing secret
            let existing: Option<Vec<u8>> = conn
                .query_row(
                    "SELECT value FROM settings WHERE key = ?1",
                    params!["jwt_secret"],
                    |row| row.get(0),
                )
                .ok();

            if let Some(secret) = existing {
                return Ok(secret);
            }

            // Generate new 64-byte secret
            let mut rng = rand::thread_rng();
            let secret: Vec<u8> = (0..64).map(|_| rng.gen()).collect();

            conn.execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2)",
                params!["jwt_secret", &secret],
            )?;

            tracing::info!("Generated new JWT signing secret");
            Ok::<Vec<u8>, rusqlite::Error>(secret)
        })
        .await
        .map_err(|e| AppError::Internal(format!("JWT secret interact error: {e}")))?
        .map_err(AppError::Database)?;

    Ok(secret)
}
