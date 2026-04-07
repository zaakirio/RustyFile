pub mod user_repo;

use deadpool_sqlite::{Config, Pool, PoolConfig, Runtime};
use rand::RngCore;
use rusqlite::params;

use crate::config::AppConfig;
use crate::error::AppError;

pub fn create_pool(config: &AppConfig) -> anyhow::Result<Pool> {
    let db_path = config.db_path();

    let mut cfg = Config::new(db_path);
    cfg.pool = Some(PoolConfig {
        max_size: 4, // SQLite WAL: 1 writer + 3 readers is sufficient
        ..Default::default()
    });
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
        // Always set pragmas first.
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=NORMAL;
             PRAGMA foreign_keys=ON;
             PRAGMA busy_timeout=5000;",
        )?;

        // Ensure settings table exists (needed to read schema_version).
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS settings (
                key TEXT PRIMARY KEY,
                value BLOB NOT NULL
            );",
        )?;

        // Read current schema version (0 if not set).
        let current_version: i64 = conn
            .query_row(
                "SELECT CAST(value AS INTEGER) FROM settings WHERE key = 'schema_version'",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);

        if current_version < 1 {
            tracing::info!("Applying migration V1: initial schema");
            let sql = include_str!("../../migrations/V1__initial_schema.sql");
            conn.execute_batch(sql)?;
            conn.execute(
                "INSERT OR REPLACE INTO settings (key, value) VALUES ('schema_version', 1)",
                [],
            )?;
        }

        // Future migrations go here:
        // if current_version < 2 {
        //     tracing::info!("Applying migration V2: ...");
        //     conn.execute_batch(include_str!("../../migrations/V2__....sql"))?;
        //     conn.execute("INSERT OR REPLACE INTO settings (key, value) VALUES ('schema_version', 2)", [])?;
        // }

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
        // Generate a candidate secret.
        let mut candidate = vec![0u8; 64];
        rand::rngs::OsRng.fill_bytes(&mut candidate);

        // INSERT OR IGNORE: if jwt_secret already exists, this is a no-op.
        conn.execute(
            "INSERT OR IGNORE INTO settings (key, value) VALUES ('jwt_secret', ?1)",
            params![&candidate],
        )?;

        // Always SELECT: either our candidate was inserted, or the existing one is returned.
        let secret: Vec<u8> = conn.query_row(
            "SELECT value FROM settings WHERE key = 'jwt_secret'",
            [],
            |row| row.get(0),
        )?;

        Ok(secret)
    })
    .await
}
