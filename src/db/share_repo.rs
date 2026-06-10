//! Repository for share links (anonymous download/drop access).
//!
//! Tokens are 32 bytes from `OsRng`, base64url-encoded without padding —
//! the same entropy source as the JWT secret. Expiry is stored as unix
//! seconds; an expired share is treated exactly like a missing one by the
//! public API (no oracle).

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use deadpool_sqlite::Pool;
use rand::RngCore;
use serde::Serialize;

use crate::db;
use crate::error::AppError;

#[derive(Debug, Clone, Serialize)]
pub struct Share {
    pub token: String,
    pub path: String,
    pub kind: String,
    #[serde(skip)]
    pub password_hash: Option<String>,
    pub expires_at: Option<i64>,
    pub created_at: i64,
    pub download_count: i64,
}

impl Share {
    pub fn has_password(&self) -> bool {
        self.password_hash.is_some()
    }

    pub fn is_expired(&self, now: i64) -> bool {
        self.expires_at.is_some_and(|exp| exp <= now)
    }
}

/// 32 bytes of OS randomness, base64url without padding (43 chars).
pub fn generate_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

fn row_to_share(row: &rusqlite::Row) -> rusqlite::Result<Share> {
    Ok(Share {
        token: row.get(0)?,
        path: row.get(1)?,
        kind: row.get(2)?,
        password_hash: row.get(3)?,
        expires_at: row.get(4)?,
        created_at: row.get(5)?,
        download_count: row.get(6)?,
    })
}

const SHARE_COLUMNS: &str =
    "token, path, kind, password_hash, expires_at, created_at, download_count";

pub async fn create(
    pool: &Pool,
    path: &str,
    kind: &str,
    password_hash: Option<String>,
    expires_at: Option<i64>,
) -> Result<Share, AppError> {
    let token = generate_token();
    let path = path.to_string();
    let kind = kind.to_string();
    let created_at = chrono::Utc::now().timestamp();

    db::interact(pool, move |conn| {
        conn.execute(
            "INSERT INTO shares (token, path, kind, password_hash, expires_at, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![token, path, kind, password_hash, expires_at, created_at],
        )?;

        conn.query_row(
            &format!("SELECT {SHARE_COLUMNS} FROM shares WHERE token = ?1"),
            rusqlite::params![token],
            row_to_share,
        )
    })
    .await
}

pub async fn list(pool: &Pool) -> Result<Vec<Share>, AppError> {
    db::interact(pool, move |conn| {
        let mut stmt = conn.prepare(&format!(
            "SELECT {SHARE_COLUMNS} FROM shares ORDER BY created_at DESC"
        ))?;
        let shares = stmt
            .query_map([], row_to_share)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(shares)
    })
    .await
}

pub async fn find_by_token(pool: &Pool, token: &str) -> Result<Option<Share>, AppError> {
    let token = token.to_string();

    db::interact(pool, move |conn| {
        let result = conn.query_row(
            &format!("SELECT {SHARE_COLUMNS} FROM shares WHERE token = ?1"),
            rusqlite::params![token],
            row_to_share,
        );

        match result {
            Ok(share) => Ok(Some(share)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    })
    .await
}

/// Returns the share only if it exists and has not expired. The public API
/// treats expired and missing tokens identically (404).
pub async fn find_valid_by_token(pool: &Pool, token: &str) -> Result<Option<Share>, AppError> {
    let now = chrono::Utc::now().timestamp();
    Ok(find_by_token(pool, token)
        .await?
        .filter(|share| !share.is_expired(now)))
}

/// Returns `true` when a row was deleted.
pub async fn delete(pool: &Pool, token: &str) -> Result<bool, AppError> {
    let token = token.to_string();

    db::interact(pool, move |conn| {
        let rows = conn.execute(
            "DELETE FROM shares WHERE token = ?1",
            rusqlite::params![token],
        )?;
        Ok(rows > 0)
    })
    .await
}

/// Best-effort download counter; callers ignore failures.
pub async fn increment_download_count(pool: &Pool, token: &str) -> Result<(), AppError> {
    let token = token.to_string();

    db::interact(pool, move |conn| {
        conn.execute(
            "UPDATE shares SET download_count = download_count + 1 WHERE token = ?1",
            rusqlite::params![token],
        )?;
        Ok(())
    })
    .await
}

/// Deletes all expired rows; returns the number removed.
pub async fn delete_expired(pool: &Pool) -> Result<usize, AppError> {
    let now = chrono::Utc::now().timestamp();

    db::interact(pool, move |conn| {
        let rows = conn.execute(
            "DELETE FROM shares WHERE expires_at IS NOT NULL AND expires_at <= ?1",
            rusqlite::params![now],
        )?;
        Ok(rows)
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_is_base64url_no_pad() {
        let token = generate_token();
        // 32 bytes -> ceil(32 * 4 / 3) = 43 chars without padding.
        assert_eq!(token.len(), 43);
        assert!(token
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
    }

    #[test]
    fn tokens_are_unique() {
        assert_ne!(generate_token(), generate_token());
    }

    #[test]
    fn expiry_check() {
        let share = Share {
            token: "t".into(),
            path: "p".into(),
            kind: "download".into(),
            password_hash: None,
            expires_at: Some(100),
            created_at: 0,
            download_count: 0,
        };
        assert!(share.is_expired(100));
        assert!(share.is_expired(101));
        assert!(!share.is_expired(99));

        let never = Share {
            expires_at: None,
            ..share
        };
        assert!(!never.is_expired(i64::MAX));
    }
}
