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
CREATE TABLE IF NOT EXISTS retention_sweeps (
    sweep_id              TEXT PRIMARY KEY,
    started_at            TEXT NOT NULL,
    finished_at           TEXT,
    records_demoted       INTEGER NOT NULL DEFAULT 0,
    records_dropped       INTEGER NOT NULL DEFAULT 0,
    tier_transitions_json TEXT
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
