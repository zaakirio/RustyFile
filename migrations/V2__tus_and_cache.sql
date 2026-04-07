ALTER TABLE uploads ADD COLUMN expires_at TEXT;
ALTER TABLE uploads ADD COLUMN completed INTEGER NOT NULL DEFAULT 0;
CREATE INDEX IF NOT EXISTS idx_uploads_expires ON uploads(expires_at) WHERE completed = 0;
