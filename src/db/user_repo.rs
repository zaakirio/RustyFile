use deadpool_sqlite::Pool;

use crate::error::AppError;

/// Check whether at least one admin user exists in the database.
pub async fn admin_exists(pool: &Pool) -> Result<bool, AppError> {
    let conn = pool.get().await.map_err(|e| AppError::Internal(e.to_string()))?;

    let exists = conn
        .interact(|conn| {
            let count: i64 = conn.query_row(
                "SELECT COUNT(*) FROM users WHERE role = 'admin'",
                [],
                |row| row.get(0),
            )?;
            Ok::<bool, rusqlite::Error>(count > 0)
        })
        .await
        .map_err(|e| AppError::Internal(format!("admin_exists interact error: {e}")))?
        .map_err(AppError::Database)?;

    Ok(exists)
}
