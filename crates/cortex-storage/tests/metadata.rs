//! Integration tests for `cortex_storage::metadata`.

use chrono::{TimeZone, Utc};
use cortex_storage::metadata::SCHEMA_VERSION;
use cortex_storage::{hour_bucket_rfc3339, MetadataStore};

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
        "classifier_spend_hourly",
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
fn hour_bucket_rfc3339_truncates_minutes_seconds_and_nanos() {
    let ts = Utc.with_ymd_and_hms(2026, 4, 28, 23, 47, 12).unwrap()
        + chrono::Duration::nanoseconds(987_654_321);
    assert_eq!(hour_bucket_rfc3339(ts), "2026-04-28T23:00:00Z");
    let exact_hour = Utc.with_ymd_and_hms(2026, 4, 28, 5, 0, 0).unwrap();
    assert_eq!(hour_bucket_rfc3339(exact_hour), "2026-04-28T05:00:00Z");
}

#[test]
fn classifier_spend_hourly_accumulates() {
    let store = MetadataStore::open_in_memory().unwrap();
    let hour = "2026-04-28T17:00:00Z";
    store.record_classifier_spend_hourly(hour, 1, 100, 50, 1).unwrap();
    store.record_classifier_spend_hourly(hour, 2, 250, 75, 4).unwrap();
    let rows = store
        .classifier_spend_hourly_window(hour, hour)
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].hour, hour);
    assert_eq!(rows[0].spend.calls, 3);
    assert_eq!(rows[0].spend.tokens_in, 350);
    assert_eq!(rows[0].spend.tokens_out, 125);
    assert_eq!(rows[0].spend.est_usd_cents, 5);
}

#[test]
fn classifier_spend_hourly_window_filters_to_range_and_sorts_ascending() {
    let store = MetadataStore::open_in_memory().unwrap();
    // Out-of-order writes — both must come back sorted.
    store.record_classifier_spend_hourly("2026-04-28T17:00:00Z", 1, 0, 0, 12).unwrap();
    store.record_classifier_spend_hourly("2026-04-28T15:00:00Z", 1, 0, 0, 7).unwrap();
    // Outside the requested window — must NOT be returned.
    store.record_classifier_spend_hourly("2026-04-28T10:00:00Z", 1, 0, 0, 99).unwrap();
    store.record_classifier_spend_hourly("2026-04-28T20:00:00Z", 1, 0, 0, 99).unwrap();

    let rows = store
        .classifier_spend_hourly_window(
            "2026-04-28T15:00:00Z",
            "2026-04-28T17:00:00Z",
        )
        .unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].hour, "2026-04-28T15:00:00Z");
    assert_eq!(rows[0].spend.est_usd_cents, 7);
    assert_eq!(rows[1].hour, "2026-04-28T17:00:00Z");
    assert_eq!(rows[1].spend.est_usd_cents, 12);
}

#[test]
fn classifier_spend_hourly_empty_window_returns_no_rows() {
    let store = MetadataStore::open_in_memory().unwrap();
    let rows = store
        .classifier_spend_hourly_window(
            "2026-04-28T00:00:00Z",
            "2026-04-28T23:00:00Z",
        )
        .unwrap();
    assert!(rows.is_empty());
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
