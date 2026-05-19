-- Cortex metadata store (SQLite). Applied at every process startup.
-- Tables are deliberately small and derived; the authoritative event log
-- lives in the Parquet archive + Synap + Nexus.

PRAGMA foreign_keys = ON;

-- Schema metadata.
CREATE TABLE IF NOT EXISTS meta (
    key         TEXT PRIMARY KEY,
    version     INTEGER NOT NULL,
    updated_at  TEXT NOT NULL
);

-- Session lifecycle mirror.
CREATE TABLE IF NOT EXISTS sessions (
    session_id  TEXT PRIMARY KEY,
    tool        TEXT NOT NULL,
    model       TEXT,
    repo        TEXT,
    user        TEXT,
    started_at  TEXT NOT NULL,
    ended_at    TEXT,
    event_count INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS sessions_started ON sessions (started_at);
CREATE INDEX IF NOT EXISTS sessions_repo ON sessions (repo);

-- Repo registry (one row per managed repo).
CREATE TABLE IF NOT EXISTS repos (
    path                 TEXT PRIMARY KEY,
    name                 TEXT NOT NULL,
    vcs_url              TEXT,
    last_bootstrapped_at TEXT,
    last_synced_at       TEXT,
    config_json          TEXT
);

-- Bootstrap progress (one row per run).
CREATE TABLE IF NOT EXISTS bootstrap_jobs (
    job_id          TEXT PRIMARY KEY,
    repo_path       TEXT NOT NULL,
    started_at      TEXT NOT NULL,
    finished_at     TEXT,
    files_processed INTEGER NOT NULL DEFAULT 0,
    chunks_emitted  INTEGER NOT NULL DEFAULT 0,
    status          TEXT NOT NULL DEFAULT 'running',
    FOREIGN KEY (repo_path) REFERENCES repos(path) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS bootstrap_jobs_repo ON bootstrap_jobs (repo_path);
CREATE INDEX IF NOT EXISTS bootstrap_jobs_status ON bootstrap_jobs (status);

-- Classifier spend (one row per UTC day).
CREATE TABLE IF NOT EXISTS classifier_spend (
    day            TEXT PRIMARY KEY,
    calls          INTEGER NOT NULL DEFAULT 0,
    tokens_in      INTEGER NOT NULL DEFAULT 0,
    tokens_out     INTEGER NOT NULL DEFAULT 0,
    est_usd_cents  INTEGER NOT NULL DEFAULT 0
);

-- Classifier spend bucketed by UTC hour. The dashboard's
-- `series.classifier_cost_usd_today` ribbon needs hour-granular data
-- to draw 24 buckets; the daily roll-up above is too coarse for that.
-- `hour` is RFC3339 truncated to the hour (`YYYY-MM-DDTHH:00:00Z`) so
-- both the writer and the reader sort/range it lexicographically.
CREATE TABLE IF NOT EXISTS classifier_spend_hourly (
    hour           TEXT PRIMARY KEY,
    calls          INTEGER NOT NULL DEFAULT 0,
    tokens_in      INTEGER NOT NULL DEFAULT 0,
    tokens_out     INTEGER NOT NULL DEFAULT 0,
    est_usd_cents  INTEGER NOT NULL DEFAULT 0
);

-- Law registry mirror (authoritative copy is on disk; this supports fast queries).
CREATE TABLE IF NOT EXISTS laws (
    law_id         TEXT PRIMARY KEY,
    version        INTEGER NOT NULL,
    severity       TEXT NOT NULL,
    title          TEXT NOT NULL,
    body           TEXT NOT NULL,
    detector_path  TEXT,
    introduced_at  TEXT NOT NULL,
    retired_at     TEXT
);

-- Per-(model, repo) trust scores.
CREATE TABLE IF NOT EXISTS trust_scores (
    model        TEXT NOT NULL,
    repo         TEXT NOT NULL,
    score        REAL NOT NULL,
    computed_at  TEXT NOT NULL,
    PRIMARY KEY (model, repo)
);

-- Retention sweep bookkeeping.
-- `status` is the phase9a concurrency lock — 'running' is held by an
-- in-flight sweep; 'success' / 'failed' / 'abandoned' are terminal.
-- The sweep CLI exits with code 2 when it finds a 'running' row whose
-- `started_at` is newer than the abandon-grace window (1 h).
CREATE TABLE IF NOT EXISTS retention_sweeps (
    sweep_id              TEXT PRIMARY KEY,
    started_at            TEXT NOT NULL,
    finished_at           TEXT,
    records_demoted       INTEGER NOT NULL DEFAULT 0,
    records_dropped       INTEGER NOT NULL DEFAULT 0,
    tier_transitions_json TEXT,
    status                TEXT NOT NULL DEFAULT 'success'
);

-- API key registry.
CREATE TABLE IF NOT EXISTS api_keys (
    key_hash      TEXT PRIMARY KEY,
    name          TEXT NOT NULL,
    scopes        TEXT NOT NULL,
    created_at    TEXT NOT NULL,
    last_used_at  TEXT,
    revoked_at    TEXT
);

-- Phase9k — cron scheduler registry. One row per scheduled job;
-- the cortex-ops daemon ticks every 30 s, picks rows where
-- `enabled=1 AND next_run_at <= now`, spawns the configured command,
-- and updates the row with the run's outcome.
CREATE TABLE IF NOT EXISTS cron_jobs (
    name           TEXT PRIMARY KEY,
    schedule       TEXT NOT NULL,
    command        TEXT NOT NULL,
    enabled        INTEGER NOT NULL DEFAULT 1,
    last_run_at    TEXT,
    last_status    TEXT,
    next_run_at    TEXT,
    last_error     TEXT,
    last_stdout    TEXT,
    last_stderr    TEXT,
    failure_streak INTEGER NOT NULL DEFAULT 0,
    last_warning_at TEXT
);

CREATE INDEX IF NOT EXISTS cron_jobs_due ON cron_jobs (enabled, next_run_at);

-- Phase9g — metadata reaper rollups.
-- Bootstrap success rows ≥ 30 d collapse here (one row per day+repo).
-- Failed rows stay in `bootstrap_jobs` for full-detail debugging.
CREATE TABLE IF NOT EXISTS bootstrap_jobs_daily (
    day          TEXT NOT NULL,
    repo_path    TEXT NOT NULL,
    runs         INTEGER NOT NULL DEFAULT 0,
    total_files  INTEGER NOT NULL DEFAULT 0,
    total_chunks INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (day, repo_path)
);

-- Sessions older than 365 d collapse here (one row per
-- year-month + tool + repo). The `repo` and `tool` columns may be
-- empty strings when the source `sessions` row left them NULL —
-- SQLite primary keys do not allow NULL columns.
CREATE TABLE IF NOT EXISTS sessions_monthly (
    year_month         TEXT NOT NULL,
    tool               TEXT NOT NULL,
    repo               TEXT NOT NULL,
    count              INTEGER NOT NULL DEFAULT 0,
    total_event_count  INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (year_month, tool, repo)
);

-- `classifier_spend` rows older than 365 d collapse here (one row
-- per year-month). Aggregates the daily counters losslessly.
CREATE TABLE IF NOT EXISTS classifier_spend_monthly (
    year_month     TEXT PRIMARY KEY,
    calls          INTEGER NOT NULL DEFAULT 0,
    tokens_in      INTEGER NOT NULL DEFAULT 0,
    tokens_out     INTEGER NOT NULL DEFAULT 0,
    est_usd_cents  INTEGER NOT NULL DEFAULT 0
);

-- Phase10c — bootstrap dedup ledger. The walker keys on
-- `(repo, path)` and stamps the redacted-body content hash; a
-- subsequent run that sees the same hash skips publication and
-- only refreshes `last_run_id`. The 2026-04-29 audit caught the
-- pre-phase10c walker re-emitting every file under fresh ULIDs on
-- every run (26 decisions for 2 ADRs on disk, 37 laws for 12 rule
-- files, etc.); the ledger is the structural fix.
CREATE TABLE IF NOT EXISTS bootstrap_seen (
    repo            TEXT NOT NULL,
    path            TEXT NOT NULL,
    content_hash    TEXT NOT NULL,
    last_run_id     TEXT,
    last_emitted_at TEXT NOT NULL,
    PRIMARY KEY (repo, path)
);

CREATE INDEX IF NOT EXISTS bootstrap_seen_hash ON bootstrap_seen (repo, content_hash);

-- Phase11s §2.3 — durable consumer-offset ledger. Synap SDK 0.12
-- ships `queue::ack` but no durable consumer-group surface on
-- the stream API the workers consume from; the §2.3 fallback path
-- persists the last successfully-processed `(stream, consumer_id)
-- → event_id + offset` here so a worker restart resumes from the
-- correct point instead of defaulting to "latest" and silently
-- dropping the rebuild window. Composite primary key lets multiple
-- workers (`cortex-graph-workerN`, `cortex-fulltext-workerN`)
-- share the same ledger without colliding.
CREATE TABLE IF NOT EXISTS consumer_offsets (
    consumer_id     TEXT NOT NULL,
    stream          TEXT NOT NULL,
    last_offset     INTEGER NOT NULL,
    last_event_id   TEXT,
    updated_at      TEXT NOT NULL,
    PRIMARY KEY (consumer_id, stream)
);

CREATE INDEX IF NOT EXISTS consumer_offsets_stream ON consumer_offsets (stream);

-- Phase13b — producer checkpoint ledger (ADR-010). Append-only:
-- every emit batch from every `impl EnvelopeProducer` writes one
-- row; resume picks the row with the maximum `accumulated_at` for
-- the (producer_name, scope) pair. The composite primary key
-- includes `accumulated_at` so two concurrent invocations cannot
-- collide.
CREATE TABLE IF NOT EXISTS producer_checkpoints (
    producer_name    TEXT NOT NULL,
    scope            TEXT NOT NULL,
    last_event_id    TEXT NOT NULL,
    last_occurred_at TEXT NOT NULL,
    accumulated_at   TEXT NOT NULL,
    PRIMARY KEY (producer_name, scope, accumulated_at)
);

CREATE INDEX IF NOT EXISTS producer_checkpoints_latest
    ON producer_checkpoints (producer_name, scope, accumulated_at DESC);
