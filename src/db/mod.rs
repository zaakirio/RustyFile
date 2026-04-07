pub mod user_repo;

use deadpool_sqlite::{Config, Pool, Runtime};
use rand::RngCore;
use rusqlite::params;

use crate::config::AppConfig;
use crate::error::AppError;

pub fn create_pool(config: &AppConfig) -> anyhow::Result<Pool> {
    let db_path = config.db_path();

    let cfg = Config::new(db_path);
    let pool = cfg.create_pool(Runtime::Tokio1)?;

    Ok(pool)
}

pub async fn interact<F, T>(pool: &Pool, f: F) -> Result<T, AppError>
where
    F: FnOnce(&mut rusqlite::Connection) -> Result<T, rusqlite::Error> + Send + 'static,
    T: Send + 'static,
{
    let conn = pool
        .get()
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;
    conn.interact(f)
        .await
        .map_err(|e| AppError::Internal(format!("interact error: {e}")))?
        .map_err(AppError::Database)
}

pub async fn run_migrations(pool: &Pool) -> anyhow::Result<()> {
    let conn = pool.get().await?;

    conn.interact(|conn| {
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=NORMAL;
             PRAGMA foreign_keys=ON;
             PRAGMA busy_timeout=5000;",
        )?;

        let migration_sql = include_str!("../../migrations/V1__initial_schema.sql");
        conn.execute_batch(migration_sql)?;

        conn.execute(
            "INSERT OR IGNORE INTO settings (key, value) VALUES ('schema_version', X'01')",
            [],
        )?;

        Ok::<_, rusqlite::Error>(())
    })
    .await
    .map_err(|e| anyhow::anyhow!("Migration interact error: {e}"))?
    .map_err(|e: rusqlite::Error| anyhow::anyhow!("Migration SQL error: {e}"))?;

    tracing::info!("Database migrations applied successfully");
    Ok(())
}

pub async fn get_or_create_jwt_secret(pool: &Pool) -> Result<Vec<u8>, AppError> {
    interact(pool, |conn| {
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

        let mut secret = vec![0u8; 64];
        rand::rngs::OsRng.fill_bytes(&mut secret);

        conn.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)",
            params!["jwt_secret", &secret],
        )?;

        tracing::info!("Generated new JWT signing secret");
        Ok(secret)
    })
    .await
}
