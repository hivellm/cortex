use cortex_local::*;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fs, path::Path};

const DAY: &str = "2026-01-01T00:00:00Z";
const HALF_LIFE: &str = "2026-01-31T00:00:00Z";
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn memory(store: &LocalStore, scope: &str, id: &str) -> Record {
    store
        .capture(CaptureRequest {
            id: Some(id.into()),
            repository: scope.into(),
            kind: RecordKind::Lesson,
            title: "guidance".into(),
            body: "needle guidance".into(),
            ..Default::default()
        })
        .unwrap()
        .record
}
fn query(scope: &str, text: &str) -> SearchQuery {
    SearchQuery {
        repository: scope.into(),
        text: text.into(),
        limit: 100,
        include_inactive: false,
    }
}
fn ranked(store: &LocalStore, scope: &str, text: &str, time: &str) -> Vec<SearchHit> {
    store
        .search_with_feedback(
            query(scope, text),
            RankingOptions {
                enabled: true,
                as_of: Some(time.into()),
            },
        )
        .unwrap()
}
fn explanation(hit: &SearchHit) -> Value {
    serde_json::to_value(hit.feedback.as_ref().unwrap()).unwrap()
}
fn close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() < 1e-10,
        "actual={actual}, expected={expected}"
    );
}

// Historical fixtures reseal canonical records, then rebuild through production validation.
fn set_time(root: &Path, directory: &str, scope: &str, id: &str, time: &str) {
    let path = root
        .join(directory)
        .join(hash(scope.as_bytes()))
        .join(format!("{}.json", hash(id.as_bytes())));
    let mut value: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    value["recorded_at"] = json!(time);
    value["content_sha256"] = json!("");
    value["content_sha256"] = json!(hash(
        &cortex_core::canonical_json::canonicalize(&value).unwrap()
    ));
    fs::write(
        path,
        cortex_core::canonical_json::canonicalize(&value).unwrap(),
    )
    .unwrap();
}
fn delivery(store: &LocalStore, record: &Record, id: &str, query_id: &str, text: &str, time: &str) {
    store
        .append_audit(AuditRequest {
            id: Some(id.into()),
            repository: record.repository.clone(),
            payload: AuditPayload::Delivery(DeliveryDraft {
                query_id: query_id.into(),
                origin: AuditOrigin::Mcp,
                attribution: AuditAttribution::default(),
                query_sha256: hash(text.as_bytes()),
                context_sha256: hash(b"context"),
                context_bytes: 7,
                selected_count: 1,
                omitted_count: 0,
                references: vec![AuditReference::from_record(record, false).unwrap()],
                outcome: PreparationOutcome::Prepared,
                latency_us: 1,
                error: None,
            }),
        })
        .unwrap();
    set_time(store.root(), "audit", &record.repository, id, time);
    store.reindex().unwrap();
}
#[allow(clippy::too_many_arguments)]
fn feedback(
    store: &LocalStore,
    record: &Record,
    id: &str,
    delivery_id: &str,
    query_id: &str,
    judgment: FeedbackJudgment,
    evidence: FeedbackEvidence,
    time: &str,
) {
    store
        .record_feedback(FeedbackRequest {
            id: id.into(),
            repository: record.repository.clone(),
            source: FeedbackSource {
                kind: "user".into(),
                reference: "fixture".into(),
                actor: format!("actor-{id}"),
            },
            evidence,
            judgment,
            query_id: Some(query_id.into()),
            delivery_id: Some(delivery_id.into()),
            reference: Some(AuditReference::from_record(record, false).unwrap()),
            comment: None,
        })
        .unwrap();
    set_time(store.root(), "feedback", &record.repository, id, time);
    store.reindex().unwrap();
}
fn sample(store: &LocalStore, record: &Record, n: usize, judgment: FeedbackJudgment) {
    let id = format!("sample-{n}");
    delivery(store, record, &id, &id, "needle", DAY);
    feedback(
        store,
        record,
        &id,
        &id,
        &id,
        judgment,
        FeedbackEvidence::Explicit,
        DAY,
    );
}
fn snapshot(root: &Path) -> BTreeMap<std::path::PathBuf, Vec<u8>> {
    fn walk(root: &Path, path: &Path, out: &mut BTreeMap<std::path::PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(path).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(root, &path, out);
            } else if path
                .extension()
                .is_some_and(|extension| extension == "json")
            {
                out.insert(
                    path.strip_prefix(root).unwrap().into(),
                    fs::read(path).unwrap(),
                );
            }
        }
    }
    let mut result = BTreeMap::new();
    walk(root, root, &mut result);
    result
}

#[test]
fn disabled_is_identical_and_rollback_reindex_preserve_canonical_bytes() {
    let root = tempfile::tempdir().unwrap();
    let store = LocalStore::open(root.path()).unwrap();
    let record = memory(&store, "Cortex", "one");
    memory(&store, "Cortex", "two");
    for n in 0..3 {
        sample(&store, &record, n, FeedbackJudgment::Helpful);
    }
    let before = snapshot(root.path());
    let baseline = serde_json::to_value(store.search(query("Cortex", "needle")).unwrap()).unwrap();
    let disabled = store
        .search_with_feedback(
            query("Cortex", "needle"),
            RankingOptions {
                enabled: false,
                as_of: Some("deliberately invalid and ignored".into()),
            },
        )
        .unwrap();
    assert_eq!(serde_json::to_value(disabled).unwrap(), baseline);
    let enabled = serde_json::to_value(ranked(&store, "Cortex", "needle", DAY)).unwrap();
    assert_eq!(enabled[0]["record"]["id"], "one");
    store.reindex().unwrap();
    assert_eq!(
        serde_json::to_value(ranked(&store, "Cortex", "needle", DAY)).unwrap(),
        enabled
    );
    assert_eq!(
        serde_json::to_value(
            store
                .search_with_feedback(
                    query("Cortex", "needle"),
                    RankingOptions {
                        enabled: false,
                        as_of: None
                    }
                )
                .unwrap()
        )
        .unwrap(),
        baseline
    );
    assert_eq!(snapshot(root.path()), before);
}

#[test]
fn threshold_decay_and_positive_negative_bounds_follow_fixed_policy() {
    for judgment in [FeedbackJudgment::Helpful, FeedbackJudgment::Unhelpful] {
        let root = tempfile::tempdir().unwrap();
        let store = LocalStore::open(root.path()).unwrap();
        let record = memory(&store, "Cortex", "one");
        for n in 0..2 {
            sample(&store, &record, n, judgment);
        }
        assert_eq!(
            explanation(&ranked(&store, "Cortex", "needle", DAY)[0])["reason"],
            "insufficient_samples"
        );
        sample(&store, &record, 2, judgment);
        let sign = if judgment == FeedbackJudgment::Helpful {
            1.0
        } else {
            -1.0
        };
        let now = ranked(&store, "Cortex", "needle", DAY);
        let info = explanation(&now[0]);
        assert_eq!(info["samples"], 3);
        assert_eq!(info["reason"], "applied");
        close(info["adjustment"].as_f64().unwrap(), sign * 0.25);
        let half = explanation(&ranked(&store, "Cortex", "needle", HALF_LIFE)[0]);
        close(half["effective_weight"].as_f64().unwrap(), 1.5);
        close(half["adjustment"].as_f64().unwrap(), sign * 0.125);
        let old = explanation(&ranked(&store, "Cortex", "needle", "2026-02-01T00:00:00Z")[0]);
        assert_eq!(old["reason"], "insufficient_weight");
        close(old["adjustment"].as_f64().unwrap(), 0.0);
    }
}

#[test]
fn duplicate_actors_deliveries_and_late_replays_do_not_amplify_or_refresh() {
    let root = tempfile::tempdir().unwrap();
    let store = LocalStore::open(root.path()).unwrap();
    let record = memory(&store, "Cortex", "one");
    for n in 0..3 {
        sample(&store, &record, n, FeedbackJudgment::Helpful);
    }
    let before = explanation(&ranked(&store, "Cortex", "needle", HALF_LIFE)[0]);
    delivery(
        &store,
        &record,
        "duplicate-delivery",
        "sample-0",
        "needle",
        DAY,
    );
    for n in 0..4 {
        feedback(
            &store,
            &record,
            &format!("duplicate-{n}"),
            "duplicate-delivery",
            "sample-0",
            FeedbackJudgment::Helpful,
            FeedbackEvidence::Explicit,
            HALF_LIFE,
        );
    }
    let after = explanation(&ranked(&store, "Cortex", "needle", HALF_LIFE)[0]);
    assert_eq!(after["samples"], 3);
    assert_eq!(after["adjustment"], before["adjustment"]);
    assert_eq!(after["effective_weight"], before["effective_weight"]);
}

#[test]
fn mixed_judgments_use_signed_weights_over_all_nonconflicting_samples() {
    let root = tempfile::tempdir().unwrap();
    let store = LocalStore::open(root.path()).unwrap();
    let record = memory(&store, "Cortex", "one");
    sample(&store, &record, 0, FeedbackJudgment::Helpful);
    sample(&store, &record, 1, FeedbackJudgment::Helpful);
    sample(&store, &record, 2, FeedbackJudgment::Unhelpful);
    let info = explanation(&ranked(&store, "Cortex", "needle", DAY)[0]);
    assert_eq!(info["samples"], 3);
    assert_eq!(info["reason"], "applied");
    close(info["adjustment"].as_f64().unwrap(), 0.25 / 3.0);
}

#[test]
fn temporal_filter_precedes_conflicts_and_query_hash_consistency() {
    let root = tempfile::tempdir().unwrap();
    let store = LocalStore::open(root.path()).unwrap();
    let record = memory(&store, "Cortex", "one");
    for n in 0..4 {
        sample(&store, &record, n, FeedbackJudgment::Helpful);
    }
    feedback(
        &store,
        &record,
        "inferred",
        "sample-0",
        "sample-0",
        FeedbackJudgment::Unhelpful,
        FeedbackEvidence::Inferred,
        DAY,
    );
    feedback(
        &store,
        &record,
        "unknown",
        "sample-0",
        "sample-0",
        FeedbackJudgment::Unknown,
        FeedbackEvidence::Explicit,
        DAY,
    );
    feedback(
        &store,
        &record,
        "future-conflict",
        "sample-0",
        "sample-0",
        FeedbackJudgment::Unhelpful,
        FeedbackEvidence::Explicit,
        HALF_LIFE,
    );
    delivery(
        &store,
        &record,
        "future-hash-conflict",
        "sample-1",
        "different",
        HALF_LIFE,
    );
    let early = explanation(&ranked(&store, "Cortex", "needle", DAY)[0]);
    assert_eq!(early["samples"], 4);
    assert_eq!(early["conflicting_groups"], 0);
    assert_eq!(early["reason"], "applied");
    let late = explanation(&ranked(&store, "Cortex", "needle", HALF_LIFE)[0]);
    assert_eq!(late["samples"], 2);
    assert_eq!(late["conflicting_groups"], 1);
    assert_eq!(late["reason"], "insufficient_samples");
}

#[test]
fn exact_scope_query_text_and_current_revision_are_required() {
    let root = tempfile::tempdir().unwrap();
    let store = LocalStore::open(root.path()).unwrap();
    let record = memory(&store, "Cortex", "one");
    memory(&store, "cortex", "one");
    for n in 0..3 {
        sample(&store, &record, n, FeedbackJudgment::Helpful);
    }
    for (scope, text) in [
        ("cortex", "needle"),
        ("Cortex", "NEEDLE"),
        ("Cortex", "needle "),
    ] {
        let hits = ranked(&store, scope, text, DAY);
        assert!(!hits.is_empty());
        close(explanation(&hits[0])["adjustment"].as_f64().unwrap(), 0.0);
    }
    store
        .revise(
            "Cortex",
            "one",
            1,
            RevisionRequest {
                title: record.title,
                body: record.body,
                authority: Authority::Accepted,
                links: vec![],
                metadata: json!({}),
            },
        )
        .unwrap();
    let hits = ranked(&store, "Cortex", "needle", DAY);
    assert_eq!(hits[0].record.revision, 2);
    close(explanation(&hits[0])["adjustment"].as_f64().unwrap(), 0.0);
}

#[test]
fn event_feedback_never_changes_event_score() {
    let root = tempfile::tempdir().unwrap();
    let store = LocalStore::open(root.path()).unwrap();
    let record = store
        .capture(CaptureRequest {
            id: Some("event".into()),
            repository: "Cortex".into(),
            kind: RecordKind::Event,
            body: "needle event".into(),
            ..Default::default()
        })
        .unwrap()
        .record;
    delivery(
        &store,
        &record,
        "event-delivery",
        "event-query",
        "needle",
        DAY,
    );
    assert!(store
        .record_feedback(FeedbackRequest {
            id: "event-feedback".into(),
            repository: "Cortex".into(),
            source: FeedbackSource {
                kind: "user".into(),
                reference: "fixture".into(),
                actor: "actor".into()
            },
            evidence: FeedbackEvidence::Explicit,
            judgment: FeedbackJudgment::Helpful,
            query_id: Some("event-query".into()),
            delivery_id: Some("event-delivery".into()),
            reference: Some(AuditReference::from_record(&record, false).unwrap()),
            comment: None,
        })
        .is_err());
    let baseline = store.search(query("Cortex", "needle")).unwrap();
    let hits = ranked(&store, "Cortex", "needle", DAY);
    assert_eq!(hits[0].score, baseline[0].score);
    assert_eq!(explanation(&hits[0])["reason"], "event_excluded");
}

#[test]
fn evidence_overflow_returns_baseline_instead_of_scoring_a_partial_set() {
    for table in ["feedback_records", "audit_records"] {
        let root = tempfile::tempdir().unwrap();
        let store = LocalStore::open(root.path()).unwrap();
        let record = memory(&store, "Cortex", "one");
        for n in 0..3 {
            sample(&store, &record, n, FeedbackJudgment::Helpful);
        }
        let baseline = store.search(query("Cortex", "needle")).unwrap();
        // Derived-index fixture tests the count guard without 10,001 filesystem writes.
        // Rows deliberately cannot be decoded: fallback must happen before payload loading.
        let connection = rusqlite::Connection::open(root.path().join("index.sqlite3")).unwrap();
        let payload = if table == "audit_records" {
            r#"{"payload":{"type":"delivery"}}"#
        } else {
            "{}"
        };
        connection.execute_batch(&format!("WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<10001) INSERT INTO {table}(repository,id,recorded_at,record) SELECT 'Cortex','overflow-'||x,'2026-01-01T00:00:00.000000000Z','{payload}' FROM n;")).unwrap();
        drop(connection);
        let hits = ranked(&store, "Cortex", "needle", DAY);
        assert_eq!(hits[0].score, baseline[0].score);
        assert_eq!(explanation(&hits[0])["reason"], "evidence_limit");
        let disabled = store
            .search_with_feedback(
                query("Cortex", "needle"),
                RankingOptions {
                    enabled: false,
                    as_of: None,
                },
            )
            .unwrap();
        assert_eq!(
            serde_json::to_value(disabled).unwrap(),
            serde_json::to_value(baseline).unwrap()
        );
    }
}

#[test]
fn candidates_remain_baseline_top_hundred_and_ties_keep_baseline_order() {
    let root = tempfile::tempdir().unwrap();
    let store = LocalStore::open(root.path()).unwrap();
    for n in 0..102 {
        memory(&store, "Cortex", &format!("memory-{n:03}"));
    }
    let baseline = store.search(query("Cortex", "needle")).unwrap();
    assert_eq!(baseline.len(), 100);
    let hits = ranked(&store, "Cortex", "needle", DAY);
    assert_eq!(
        hits.iter()
            .map(|h| (&h.record.id, h.score))
            .collect::<Vec<_>>(),
        baseline
            .iter()
            .map(|h| (&h.record.id, h.score))
            .collect::<Vec<_>>()
    );
    assert!(hits
        .iter()
        .all(|h| explanation(h)["candidate_limit"] == 100));
    let mut limited = query("Cortex", "needle");
    limited.limit = 3;
    let short = store
        .search_with_feedback(
            limited,
            RankingOptions {
                enabled: true,
                as_of: Some(DAY.into()),
            },
        )
        .unwrap();
    assert_eq!(
        short.iter().map(|h| &h.record.id).collect::<Vec<_>>(),
        baseline[..3]
            .iter()
            .map(|h| &h.record.id)
            .collect::<Vec<_>>()
    );
}
