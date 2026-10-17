use std::sync::{Arc, Barrier};

use chrono::TimeDelta;

use super::*;
use crate::daily_memory::{MemoryCapture, MemoryCategory, MemoryOrigin, MemoryScope};

fn time() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-10-16T12:00:00Z")
        .unwrap()
        .with_timezone(&Utc)
}

fn key(index: usize) -> String {
    format!("{index:064x}")
}

fn reserve(store: &mut MetadataStore, index: usize, now: DateTime<Utc>) -> AdviceReservation {
    let AdviceAdmission::Reserved(reservation) =
        reserve_at(store, &key(index), "jev-1.13.0", "candidate-v1", now).unwrap()
    else {
        panic!("expected uncached reservation {index}");
    };
    reservation
}

#[test]
fn reservation_budget_and_unknown_usage_survive_restart_and_reset_only_at_utc_day() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("metadata.sqlite");
    let mut store = MetadataStore::open(&path).unwrap();
    for index in 0..100 {
        let attempt = reserve(&mut store, index, time());
        if index < 99 {
            store
                .finish_daily_memory_advice(&attempt.attempt_id, "timeout", None, 2000, None)
                .unwrap();
        }
    }
    drop(store);
    let mut reopened = MetadataStore::open(&path).unwrap();
    let expired = time() + TimeDelta::milliseconds(LEASE_MS);
    assert_eq!(
        reserve_at(
            &mut reopened,
            &key(100),
            "jev-1.13.0",
            "candidate-v1",
            expired
        )
        .unwrap(),
        AdviceAdmission::BudgetExhausted
    );
    let stats = reopened.daily_memory_advice_stats("2026-10-16").unwrap();
    assert_eq!(stats.reserved, 100);
    assert_eq!(stats.finished, 99);
    assert_eq!(stats.usage_known, 0);
    assert_eq!(stats.usage_unknown, 100);
    assert_eq!(stats.budget_exhausted, 1);
    assert_eq!(stats.fallbacks["timeout"], 99);
    assert_eq!(stats.fallbacks["budget_exhausted"], 1);
    reserve(&mut reopened, 100, time() + TimeDelta::days(1));
    assert_eq!(
        reopened
            .daily_memory_advice_stats("2026-10-17")
            .unwrap()
            .reserved,
        1
    );
    assert_eq!(
        reopened
            .daily_memory_revision(&MemoryScope::Repository("repo".into()))
            .unwrap(),
        0
    );
}

#[test]
fn concurrent_connections_share_one_global_lease_and_expired_crash_releases_it() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("metadata.sqlite");
    drop(MetadataStore::open(&path).unwrap());
    let barrier = Arc::new(Barrier::new(2));
    let handles: Vec<_> = (0..2)
        .map(|index| {
            let path = path.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let mut store = MetadataStore::open(&path).unwrap();
                barrier.wait();
                reserve_at(
                    &mut store,
                    &key(index),
                    "jev-1.13.0",
                    "candidate-v1",
                    time(),
                )
                .unwrap()
            })
        })
        .collect();
    let results: Vec<_> = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .collect();
    assert_eq!(
        results
            .iter()
            .filter(|r| matches!(r, AdviceAdmission::Reserved(_)))
            .count(),
        1
    );
    assert_eq!(
        results
            .iter()
            .filter(|r| matches!(r, AdviceAdmission::Busy))
            .count(),
        1
    );
    let mut reopened = MetadataStore::open(&path).unwrap();
    assert_eq!(
        reserve_at(
            &mut reopened,
            &key(9),
            "other-model",
            "other-schema",
            time()
        )
        .unwrap(),
        AdviceAdmission::Busy
    );
    reserve(&mut reopened, 9, time() + TimeDelta::milliseconds(LEASE_MS));
    let stats = reopened.daily_memory_advice_stats("2026-10-16").unwrap();
    assert_eq!(stats.reserved, 2);
    assert_eq!(stats.busy, 2);
    assert_eq!(stats.usage_unknown, 2);
}

#[test]
fn successful_cache_bypasses_busy_and_exhausted_budget_and_preserves_model_schema() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("metadata.sqlite");
    let mut store = MetadataStore::open(&path).unwrap();
    let first = reserve(&mut store, 0, time());
    store
        .finish_daily_memory_advice(
            &first.attempt_id,
            "success",
            None,
            25,
            Some("{\"durability\":0.75}"),
        )
        .unwrap();
    let mut last = None;
    for index in 1..100 {
        let attempt = reserve(&mut store, index, time());
        if index < 99 {
            store
                .finish_daily_memory_advice(&attempt.attempt_id, "unavailable", None, 5, None)
                .unwrap();
        }
        last = Some(attempt);
    }
    drop(store);
    let mut reopened = MetadataStore::open(&path).unwrap();
    assert_eq!(
        reserve_at(&mut reopened, &key(0), "jev-1.13.0", "candidate-v1", time()).unwrap(),
        AdviceAdmission::Cached("{\"durability\":0.75}".into())
    );
    assert_eq!(
        reserve_at(&mut reopened, &key(0), "jev-1.13.0", "candidate-v2", time()).unwrap(),
        AdviceAdmission::Busy
    );
    reopened
        .finish_daily_memory_advice(&last.unwrap().attempt_id, "timeout", None, 2000, None)
        .unwrap();
    assert_eq!(
        reserve_at(
            &mut reopened,
            &key(0),
            "other-model",
            "candidate-v1",
            time()
        )
        .unwrap(),
        AdviceAdmission::BudgetExhausted
    );
    let stats = reopened.daily_memory_advice_stats("2026-10-16").unwrap();
    assert_eq!(stats.reserved, 100);
    assert_eq!(stats.cache_hits, 1);
    assert_eq!(stats.succeeded, 1);
}

#[test]
fn finish_is_idempotent_and_known_usage_latency_and_failure_are_accounted_once() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("metadata.sqlite");
    let mut store = MetadataStore::open(&path).unwrap();
    let attempt = reserve(&mut store, 1, time());
    store
        .finish_daily_memory_advice(
            &attempt.attempt_id,
            "success",
            Some(AdviceUsage {
                input_tokens: 200,
                output_tokens: 30,
            }),
            88,
            Some("{\"ok\":true}"),
        )
        .unwrap();
    store
        .finish_daily_memory_advice(
            &attempt.attempt_id,
            "unavailable",
            Some(AdviceUsage {
                input_tokens: 999,
                output_tokens: 999,
            }),
            999,
            None,
        )
        .unwrap();
    let failed = reserve(&mut store, 2, time());
    store
        .finish_daily_memory_advice(
            &failed.attempt_id,
            "invalid_response",
            Some(AdviceUsage {
                input_tokens: 10,
                output_tokens: 2,
            }),
            33,
            None,
        )
        .unwrap();
    store
        .record_daily_memory_advice_fallback("missing_key")
        .unwrap();
    drop(store);
    let reopened = MetadataStore::open(&path).unwrap();
    let stats = reopened.daily_memory_advice_stats("2026-10-16").unwrap();
    assert_eq!((stats.reserved, stats.finished, stats.succeeded), (2, 2, 1));
    assert_eq!((stats.usage_known, stats.usage_unknown), (2, 0));
    assert_eq!(
        (stats.input_tokens, stats.output_tokens, stats.latency_ms),
        (210, 32, 121)
    );
    assert_eq!(stats.fallbacks["invalid_response"], 1);
    assert!(!stats.fallbacks.contains_key("unavailable"));
    let today = Utc::now().date_naive().to_string();
    assert_eq!(
        reopened
            .daily_memory_advice_stats(&today)
            .unwrap()
            .fallbacks["missing_key"],
        1
    );
}

#[test]
fn invalid_and_oversized_cache_or_accounting_cannot_finish_or_store_a_success() {
    let directory = tempfile::tempdir().unwrap();
    let mut store = MetadataStore::open(&directory.path().join("metadata.sqlite")).unwrap();
    for key in ["bad", &"z".repeat(64)] {
        assert!(store
            .reserve_daily_memory_advice(key, "jev-1.13.0", "v1")
            .is_err());
    }
    for model in ["", "secret\nmodel", &"m".repeat(129)] {
        assert!(store
            .reserve_daily_memory_advice(&key(0), model, "v1")
            .is_err());
    }
    let attempt = reserve(&mut store, 0, time());
    let unicode = serde_json::json!({"answer": "界".repeat(6000)}).to_string();
    for data in ["invalid", "[]", "null", "\"text\"", &unicode] {
        assert!(store
            .finish_daily_memory_advice(&attempt.attempt_id, "success", None, 1, Some(data))
            .is_err());
    }
    assert!(store
        .finish_daily_memory_advice(&attempt.attempt_id, "timeout", None, 1, Some("{}"))
        .is_err());
    assert!(store
        .finish_daily_memory_advice(&attempt.attempt_id, "raw provider secret", None, 1, None)
        .is_err());
    assert!(store
        .finish_daily_memory_advice(
            &attempt.attempt_id,
            "success",
            Some(AdviceUsage {
                input_tokens: u64::MAX,
                output_tokens: 0
            }),
            1,
            None
        )
        .is_err());
    assert!(store
        .finish_daily_memory_advice(&attempt.attempt_id, "success", None, u64::MAX, None)
        .is_err());
    assert!(store
        .record_daily_memory_advice_fallback("raw provider secret")
        .is_err());
    assert!(store
        .record_daily_memory_advice_fallback("success")
        .is_err());
    assert!(store.daily_memory_advice_stats("2026-02-30").is_err());
    let stats = store.daily_memory_advice_stats("2026-10-16").unwrap();
    assert_eq!(
        (stats.reserved, stats.finished, stats.usage_unknown),
        (1, 0, 1)
    );
    store
        .finish_daily_memory_advice(&attempt.attempt_id, "invalid_response", None, 5, None)
        .unwrap();
    assert_eq!(
        store
            .conn()
            .query_row(
                "SELECT count(*) FROM daily_memory_advice_cache",
                [],
                |row| unsigned(row, 0)
            )
            .unwrap(),
        0
    );
    assert!(matches!(
        store.finish_daily_memory_advice("missing", "timeout", None, 1, None),
        Err(MemoryError::NotFound)
    ));
}

#[test]
fn success_cache_has_a_global_512_entry_bound_without_discarding_attempt_ledger() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("metadata.sqlite");
    let mut store = MetadataStore::open(&path).unwrap();
    for index in 0..513 {
        let day = time() + TimeDelta::days((index / 100) as i64);
        let attempt = reserve(&mut store, index, day);
        store
            .finish_daily_memory_advice(
                &attempt.attempt_id,
                "success",
                None,
                1,
                Some("{\"ok\":true}"),
            )
            .unwrap();
    }
    drop(store);
    let reopened = MetadataStore::open(&path).unwrap();
    assert_eq!(
        reopened
            .conn()
            .query_row(
                "SELECT count(*) FROM daily_memory_advice_cache",
                [],
                |row| unsigned(row, 0)
            )
            .unwrap(),
        512
    );
    assert_eq!(
        reopened
            .conn()
            .query_row(
                "SELECT count(*) FROM daily_memory_advice_attempts",
                [],
                |row| unsigned(row, 0)
            )
            .unwrap(),
        513
    );
    assert_eq!(
        reopened
            .conn()
            .query_row(
                "SELECT count(*) FROM daily_memory_advice_cache WHERE cache_key=?1",
                [key(0)],
                |row| unsigned(row, 0)
            )
            .unwrap(),
        0
    );
    assert_eq!(
        reopened
            .conn()
            .query_row(
                "SELECT count(*) FROM daily_memory_advice_cache WHERE cache_key=?1",
                [key(512)],
                |row| unsigned(row, 0)
            )
            .unwrap(),
        1
    );
}

#[test]
fn proposed_candidate_can_receive_advice_without_promotion_or_revision_change() {
    let directory = tempfile::tempdir().unwrap();
    let mut store = MetadataStore::open(&directory.path().join("metadata.sqlite")).unwrap();
    let mut candidate = MemoryCapture::new(
        "repo",
        "Proposal",
        "Explicit candidate",
        MemoryCategory::Lesson,
    );
    candidate.status = MemoryStatus::Proposed;
    let receipt = store.capture_daily_memory(&candidate).unwrap();
    let scope = MemoryScope::Repository("repo".into());
    let snapshot = AdviceSnapshot {
        scope: scope.clone(),
        receipt: receipt.clone(),
    };
    let attempt = reserve(&mut store, 0, time());
    assert!(store
        .finish_daily_memory_advice_if_current(
            &attempt.attempt_id,
            "success",
            None,
            1,
            Some("{\"durability\":0.99}"),
            &snapshot
        )
        .unwrap());
    let record = store.daily_memory(&scope, &receipt.id).unwrap().unwrap();
    assert_eq!(record.status, MemoryStatus::Proposed);
    assert_eq!(record.version, receipt.version);
    assert_eq!(
        store.daily_memory_revision(&scope).unwrap(),
        receipt.scope_revision
    );
    assert_eq!(
        reserve_at(&mut store, &key(0), "jev-1.13.0", "candidate-v1", time()).unwrap(),
        AdviceAdmission::Cached("{\"durability\":0.99}".into())
    );
}

#[test]
fn changed_corpus_or_stale_candidate_discards_result_atomically_but_keeps_usage() {
    let directory = tempfile::tempdir().unwrap();
    let mut store = MetadataStore::open(&directory.path().join("metadata.sqlite")).unwrap();
    let candidate = MemoryCapture::new(
        "repo",
        "Candidate",
        "Explicit candidate",
        MemoryCategory::Lesson,
    );
    let receipt = store.capture_daily_memory(&candidate).unwrap();
    let scope = MemoryScope::Repository("repo".into());
    let snapshot = AdviceSnapshot {
        scope: scope.clone(),
        receipt,
    };
    let first = reserve(&mut store, 0, time());
    let next = store
        .capture_daily_memory(&MemoryCapture::new(
            "repo",
            "New neighbor",
            "Relevant changed context",
            MemoryCategory::Lesson,
        ))
        .unwrap();
    let usage = Some(AdviceUsage {
        input_tokens: 30,
        output_tokens: 10,
    });
    assert!(!store
        .finish_daily_memory_advice_if_current(
            &first.attempt_id,
            "success",
            usage,
            50,
            Some("{\"ok\":true}"),
            &snapshot
        )
        .unwrap());
    // A repeat cannot charge usage again or change the first recorded fallback.
    assert!(!store
        .finish_daily_memory_advice_if_current(
            &first.attempt_id,
            "success",
            usage,
            50,
            Some("{\"ok\":true}"),
            &snapshot
        )
        .unwrap());
    let next_snapshot = AdviceSnapshot {
        scope: scope.clone(),
        receipt: next.clone(),
    };
    let second = reserve(&mut store, 1, time());
    store
        .mark_daily_memory_stale(
            &scope,
            &next.id,
            true,
            next.scope_revision,
            &MemoryOrigin::default(),
        )
        .unwrap();
    assert!(!store
        .finish_daily_memory_advice_if_current(
            &second.attempt_id,
            "success",
            usage,
            50,
            Some("{\"ok\":true}"),
            &next_snapshot
        )
        .unwrap());
    let stats = store.daily_memory_advice_stats("2026-10-16").unwrap();
    assert_eq!((stats.reserved, stats.finished, stats.succeeded), (2, 2, 0));
    assert_eq!(stats.fallbacks["stale"], 2);
    assert_eq!((stats.input_tokens, stats.output_tokens), (60, 20));
    assert_eq!(
        store
            .conn()
            .query_row(
                "SELECT count(*) FROM daily_memory_advice_cache",
                [],
                |row| unsigned(row, 0)
            )
            .unwrap(),
        0
    );
}
