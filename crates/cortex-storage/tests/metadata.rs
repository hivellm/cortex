//! Integration tests for `cortex_storage::metadata`.

use chrono::Utc;
use cortex_storage::metadata::SCHEMA_VERSION;
use cortex_storage::MetadataStore;

#[test]
fn open_in_memory_runs_migrations() {
    let store = MetadataStore::open_in_memory().unwrap();
    let tables: Vec<String> = store
        .conn()
        .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
        .unwrap()
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    for required in [
        "api_keys",
        "bootstrap_jobs",
        "classifier_spend",
        "laws",
        "meta",
        "repos",
        "retention_sweeps",
        "sessions",
        "trust_scores",
    ] {
        assert!(
            tables.iter().any(|t| t == required),
            "missing required table `{required}`"
        );
    }
}

#[test]
fn session_upsert_is_idempotent() {
    let store = MetadataStore::open_in_memory().unwrap();
    let now = Utc::now();
    store
        .upsert_session("01H", "claude-code", Some("claude-opus-4-7"), Some("Cortex"), Some("andre"), now)
        .unwrap();
    store
        .upsert_session("01H", "claude-code", None, None, None, now)
        .unwrap();
    let rows: i64 = store
        .conn()
        .query_row("SELECT count(*) FROM sessions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(rows, 1);
}

#[test]
fn classifier_spend_accumulates() {
    let store = MetadataStore::open_in_memory().unwrap();
    store.record_classifier_spend("2026-04-17", 1, 100, 50, 5).unwrap();
    store.record_classifier_spend("2026-04-17", 2, 200, 100, 10).unwrap();
    let s = store.classifier_spend("2026-04-17").unwrap().unwrap();
    assert_eq!(s.calls, 3);
    assert_eq!(s.tokens_in, 300);
    assert_eq!(s.tokens_out, 150);
    assert_eq!(s.est_usd_cents, 15);
}

#[test]
fn classifier_spend_missing_day_returns_none() {
    let store = MetadataStore::open_in_memory().unwrap();
    assert_eq!(store.classifier_spend("1999-01-01").unwrap(), None);
}

#[test]
fn schema_version_is_recorded() {
    let store = MetadataStore::open_in_memory().unwrap();
    let v: u32 = store
        .conn()
        .query_row("SELECT version FROM meta WHERE key = 'schema'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(v, SCHEMA_VERSION);
}
