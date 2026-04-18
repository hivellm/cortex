-- CAS blob store (SQLite single-node default).

PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS cas_blobs (
    hash             TEXT PRIMARY KEY,           -- "sha256:<hex>"
    size             INTEGER NOT NULL,
    content_type     TEXT NOT NULL,
    blob             BLOB NOT NULL,               -- Zstd-compressed
    refcount         INTEGER NOT NULL DEFAULT 0,
    first_seen       TEXT NOT NULL,
    last_referenced  TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS cas_blobs_last_referenced ON cas_blobs (last_referenced);
CREATE INDEX IF NOT EXISTS cas_blobs_refcount_zero ON cas_blobs (refcount) WHERE refcount = 0;
