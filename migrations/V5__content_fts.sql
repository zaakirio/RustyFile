-- Full-text content search over text-like files. Plain FTS5 table (not
-- external-content): rows are inserted/removed alongside file_index
-- maintenance, and the periodic full reindex rebuilds it from scratch.
-- `path` is UNINDEXED: it is only used for joins/deletes, never matched.
CREATE VIRTUAL TABLE IF NOT EXISTS files_fts USING fts5(
    path UNINDEXED,
    content,
    tokenize='unicode61'
);
