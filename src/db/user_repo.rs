use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::SaltString;
use argon2::{Argon2, PasswordHasher};
use deadpool_sqlite::Pool;
use serde::Serialize;

use crate::db;
use crate::error::AppError;

/// Represents a user record from the database.
#[derive(Debug, Clone, Serialize)]
pub struct User {
    pub id: i64,
    pub username: String,
    #[serde(skip)]
    pub password_hash: String,
    pub role: String,
    pub created_at: String,
    pub updated_at: String,
}

/// Shared helper: map a rusqlite row to a User struct.
fn row_to_user(row: &rusqlite::Row) -> rusqlite::Result<User> {
    Ok(User {
        id: row.get(0)?,
        username: row.get(1)?,
        password_hash: row.get(2)?,
        role: row.get(3)?,
        created_at: row.get(4)?,
        updated_at: row.get(5)?,
    })
}

/// Check whether at least one admin user exists in the database.
pub async fn admin_exists(pool: &Pool) -> Result<bool, AppError> {
    db::interact(pool, |conn| {
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM users WHERE role = 'admin'",
            [],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    })
    .await
}

/// Hash a plaintext password using Argon2id with a random salt.
pub fn hash_password(password: &str) -> Result<String, AppError> {
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    let hash = argon2
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| AppError::Internal(format!("Password hashing error: {e}")))?;
    Ok(hash.to_string())
}

/// Insert a new user into the database and return the full user record.
pub async fn create_user(
    pool: &Pool,
    username: &str,
    password_hash: &str,
    role: &str,
) -> Result<User, AppError> {
    let username = username.to_string();
    let password_hash = password_hash.to_string();
    let role = role.to_string();

    db::interact(pool, move |conn| {
        conn.execute(
            "INSERT INTO users (username, password_hash, role) VALUES (?1, ?2, ?3)",
            rusqlite::params![username, password_hash, role],
        )?;

        let id = conn.last_insert_rowid();

        conn.query_row(
            "SELECT id, username, password_hash, role, created_at, updated_at \
             FROM users WHERE id = ?1",
            rusqlite::params![id],
            row_to_user,
        )
    })
    .await
}

/// Find a user by username. Returns `None` if no matching user exists.
pub async fn find_by_username(pool: &Pool, username: &str) -> Result<Option<User>, AppError> {
    let username = username.to_string();

    db::interact(pool, move |conn| {
        let result = conn.query_row(
            "SELECT id, username, password_hash, role, created_at, updated_at \
             FROM users WHERE username = ?1",
            rusqlite::params![username],
            row_to_user,
        );

        match result {
            Ok(user) => Ok(Some(user)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    })
    .await
}

/// Find a user by their ID. Returns `None` if no matching user exists.
pub async fn find_by_id(pool: &Pool, user_id: i64) -> Result<Option<User>, AppError> {
    db::interact(pool, move |conn| {
        let result = conn.query_row(
            "SELECT id, username, password_hash, role, created_at, updated_at \
             FROM users WHERE id = ?1",
            rusqlite::params![user_id],
            row_to_user,
        );

        match result {
            Ok(user) => Ok(Some(user)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    })
    .await
}
