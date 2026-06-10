-- The V1 schema shipped a speculative `shares` table that was never wired to
-- any code. Replace it with the real share-links schema (token primary key,
-- unix-seconds expiry, download/drop kinds).
DROP TABLE IF EXISTS shares;

CREATE TABLE shares (
    token TEXT PRIMARY KEY,
    path TEXT NOT NULL,
    kind TEXT NOT NULL CHECK(kind IN ('download','drop')),
    password_hash TEXT NULL,
    expires_at INTEGER NULL,
    created_at INTEGER NOT NULL,
    download_count INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_shares_expires ON shares(expires_at) WHERE expires_at IS NOT NULL;
