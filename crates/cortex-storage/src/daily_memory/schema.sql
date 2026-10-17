CREATE TABLE IF NOT EXISTS daily_memory_records (
    rowid INTEGER PRIMARY KEY,
    scope TEXT NOT NULL,
    id TEXT NOT NULL,
    title TEXT NOT NULL,
    body TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('proposed','accepted','superseded','deprecated')),
    stale INTEGER NOT NULL CHECK (stale IN (0,1)),
    pinned INTEGER NOT NULL CHECK (pinned IN (0,1)),
    branch TEXT,
    path TEXT,
    data TEXT NOT NULL,
    UNIQUE(scope,id)
);
CREATE INDEX IF NOT EXISTS daily_memory_eligible
    ON daily_memory_records(scope,status,stale,pinned);
CREATE VIRTUAL TABLE IF NOT EXISTS daily_memory_fts USING fts5(
    title,body,content='daily_memory_records',content_rowid='rowid',
    tokenize='unicode61 remove_diacritics 2'
);
CREATE TRIGGER IF NOT EXISTS daily_memory_fts_insert AFTER INSERT ON daily_memory_records BEGIN
    INSERT INTO daily_memory_fts(rowid,title,body) VALUES(new.rowid,new.title,new.body);
END;
CREATE TRIGGER IF NOT EXISTS daily_memory_fts_delete AFTER DELETE ON daily_memory_records BEGIN
    INSERT INTO daily_memory_fts(daily_memory_fts,rowid,title,body)
        VALUES('delete',old.rowid,old.title,old.body);
END;
CREATE TRIGGER IF NOT EXISTS daily_memory_fts_update AFTER UPDATE ON daily_memory_records BEGIN
    INSERT INTO daily_memory_fts(daily_memory_fts,rowid,title,body)
        VALUES('delete',old.rowid,old.title,old.body);
    INSERT INTO daily_memory_fts(rowid,title,body) VALUES(new.rowid,new.title,new.body);
END;
CREATE TABLE IF NOT EXISTS daily_memory_revisions (
    scope TEXT PRIMARY KEY,
    revision INTEGER NOT NULL CHECK (revision >= 0)
);
CREATE TABLE IF NOT EXISTS daily_memory_evidence (
    event_id TEXT PRIMARY KEY,
    scope TEXT NOT NULL,
    memory_id TEXT NOT NULL,
    version INTEGER NOT NULL CHECK (version > 0),
    data TEXT NOT NULL,
    UNIQUE(scope,memory_id,version),
    FOREIGN KEY(scope,memory_id) REFERENCES daily_memory_records(scope,id)
);
CREATE TABLE IF NOT EXISTS daily_memory_retries (
    scope TEXT NOT NULL,
    caller TEXT NOT NULL,
    retry_key TEXT NOT NULL,
    request_hash TEXT NOT NULL,
    receipt TEXT NOT NULL,
    PRIMARY KEY(scope,caller,retry_key)
);
CREATE TABLE IF NOT EXISTS daily_memory_deliveries (
    id TEXT PRIMARY KEY,
    repo TEXT NOT NULL,
    caller TEXT NOT NULL,
    prepared_at TEXT NOT NULL,
    data TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS daily_memory_delivery_repo_time
    ON daily_memory_deliveries(repo,prepared_at);
CREATE TABLE IF NOT EXISTS daily_memory_advice_attempts (
    attempt_id TEXT PRIMARY KEY,
    day TEXT NOT NULL,
    cache_key TEXT NOT NULL,
    model TEXT NOT NULL,
    schema_version TEXT NOT NULL,
    reserved_at_ms INTEGER NOT NULL,
    lease_until_ms INTEGER NOT NULL,
    finished_at_ms INTEGER,
    outcome TEXT,
    input_tokens INTEGER CHECK (input_tokens >= 0),
    output_tokens INTEGER CHECK (output_tokens >= 0),
    latency_ms INTEGER CHECK (latency_ms >= 0),
    CHECK ((input_tokens IS NULL) = (output_tokens IS NULL))
);
CREATE INDEX IF NOT EXISTS daily_memory_advice_day
    ON daily_memory_advice_attempts(day);
CREATE INDEX IF NOT EXISTS daily_memory_advice_lease
    ON daily_memory_advice_attempts(lease_until_ms) WHERE finished_at_ms IS NULL;
CREATE TABLE IF NOT EXISTS daily_memory_advice_cache (
    cache_key TEXT NOT NULL,
    model TEXT NOT NULL,
    schema_version TEXT NOT NULL,
    data TEXT NOT NULL CHECK (length(CAST(data AS BLOB)) <= 16384 AND json_valid(data) AND json_type(data)='object'),
    used_at_ms INTEGER NOT NULL,
    PRIMARY KEY(cache_key,model,schema_version)
);
CREATE TABLE IF NOT EXISTS daily_memory_advice_counters (
    day TEXT NOT NULL,
    reason TEXT NOT NULL,
    count INTEGER NOT NULL CHECK (count >= 0),
    PRIMARY KEY(day,reason)
);
