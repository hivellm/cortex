//! Phase11k §2.4 — global governance indexes integration test.
//!
//! Emits one Decision envelope for repo A and one for repo B, asserts
//! that BOTH land in the global `cortex_decisions` index in addition
//! to their per-repo targets. Same shape for `cortex_laws`. The
//! cross-repo overlay this enables is what makes `decision_lookup`
//! and `law_check` answerable without `scope.repo` set.

use std::sync::Arc;

use cortex_core::events::Kind;
use cortex_workers::classifier::{ClassifierOutput, ClassifierSource, PiiRisk, Severity};
use cortex_workers::fulltext::meili_client::MemoryCall;
use cortex_workers::fulltext::{
    EnrichedEvent, FulltextConfig, FulltextIndexer, MeiliFulltextIndexer, MemoryMeiliClient,
    Metrics,
};
use serde_json::json;

fn classifier(event_id: &str) -> ClassifierOutput {
    ClassifierOutput {
        event_id: event_id.to_string(),
        kind_refinement: None,
        topics: vec!["governance".into()],
        severity: Severity::Info,
        pii_risk: PiiRisk::Low,
        redaction_suggestions: vec![],
        summary: None,
        entities: Vec::new(),
        relations: Vec::new(),
        source: ClassifierSource::StaticFallback,
        prompt_version: "v1".into(),
        model: "static-v1".into(),
        latency_ms: 0,
        tokens_in: 0,
        tokens_out: 0,
    }
}

fn evt(event_id: &str, kind: Kind, repo: &str, payload: serde_json::Value) -> EnrichedEvent {
    EnrichedEvent {
        event_id: event_id.to_string(),
        kind,
        content_hash: format!("h-{event_id}"),
        redacted_payload: payload,
        classifier: classifier(event_id),
        context_repo: Some(repo.into()),
        context_path: None,
        parent_event_id: None,
        session_id: None,
        occurred_at_ms: 0,
    }
}

fn make_indexer() -> (Arc<MemoryMeiliClient>, Arc<MeiliFulltextIndexer>) {
    let client = Arc::new(MemoryMeiliClient::new());
    let metrics = Arc::new(Metrics::new());
    let cfg = FulltextConfig {
        upsert_batch: 16,
        ..FulltextConfig::default()
    };
    let indexer = Arc::new(MeiliFulltextIndexer::new(cfg, client.clone(), metrics));
    (client, indexer)
}

#[tokio::test]
async fn decisions_from_two_repos_both_land_in_global_cortex_decisions() {
    let (client, indexer) = make_indexer();
    let events = vec![
        evt(
            "dec-A-1",
            Kind::Decision,
            "Cortex",
            json!({
                "decision_id": "ADR-A-1",
                "title": "Adopt Meili",
                "status": "accepted",
                "body": "Cortex chose Meili over Lexum.",
                "tags": []
            }),
        ),
        evt(
            "dec-B-1",
            Kind::Decision,
            "Vectorizer",
            json!({
                "decision_id": "ADR-B-1",
                "title": "Adopt HNSW",
                "status": "accepted",
                "body": "Vectorizer chose HNSW for ANN search.",
                "tags": []
            }),
        ),
    ];
    let report = indexer.index_batch(&events).await.expect("index_batch");
    // 2 events × 2 lanes (per-repo + global) = 4 upserts.
    assert_eq!(report.documents_upserted, 4);
    assert_eq!(
        report.by_index.get("cortex-cortex-decisions").copied(),
        Some(1),
        "per-repo Cortex lane",
    );
    assert_eq!(
        report.by_index.get("cortex-vectorizer-decisions").copied(),
        Some(1),
        "per-repo Vectorizer lane",
    );
    assert_eq!(
        report.by_index.get("cortex_decisions").copied(),
        Some(2),
        "global lane carries both ADRs",
    );

    // Pin the wire shape: every upsert call records its target index
    // + the document ids actually written. Both ADRs MUST appear in
    // the global lane's document list.
    let calls = client.calls_snapshot();
    let mut global_doc_ids: Vec<String> = Vec::new();
    for c in calls {
        if let MemoryCall::UpsertDocuments { name, docs } = c {
            if name == "cortex_decisions" {
                for d in docs {
                    global_doc_ids.push(d.id.clone());
                }
            }
        }
    }
    global_doc_ids.sort();
    assert_eq!(
        global_doc_ids,
        vec!["dec-A-1".to_string(), "dec-B-1".to_string()],
        "global cortex_decisions must hold ADRs from BOTH repos",
    );
}

#[tokio::test]
async fn law_violations_from_two_repos_both_land_in_global_cortex_laws() {
    let (client, indexer) = make_indexer();
    let events = vec![
        evt(
            "lv-A-1",
            Kind::LawViolation,
            "Cortex",
            json!({
                "violation_id": "VIO-A-1",
                "law_id": "LAW-CORTEX-001",
                "severity": "critical",
                "tier": 1,
                "message": "task sequence cherry pick",
                "evidence": null
            }),
        ),
        evt(
            "lv-B-1",
            Kind::LawViolation,
            "Vectorizer",
            json!({
                "violation_id": "VIO-B-1",
                "law_id": "LAW-007",
                "severity": "notable",
                "tier": 2,
                "message": "destructive op without authorisation",
                "evidence": null
            }),
        ),
    ];
    let report = indexer.index_batch(&events).await.expect("index_batch");
    assert_eq!(report.documents_upserted, 4);
    assert_eq!(
        report.by_index.get("cortex-cortex-governance").copied(),
        Some(1),
    );
    assert_eq!(
        report.by_index.get("cortex-vectorizer-governance").copied(),
        Some(1),
    );
    assert_eq!(report.by_index.get("cortex_laws").copied(), Some(2));

    let calls = client.calls_snapshot();
    let mut global_law_ids: Vec<String> = Vec::new();
    for c in calls {
        if let MemoryCall::UpsertDocuments { name, docs } = c {
            if name == "cortex_laws" {
                for d in docs {
                    global_law_ids.push(d.law_id.clone().unwrap_or_default());
                }
            }
        }
    }
    global_law_ids.sort();
    assert_eq!(
        global_law_ids,
        vec!["LAW-007".to_string(), "LAW-CORTEX-001".to_string()],
        "global cortex_laws must hold violations from BOTH repos",
    );
}

#[tokio::test]
async fn non_governance_kinds_do_not_dual_write_to_globals() {
    let (client, indexer) = make_indexer();
    let events = vec![evt(
        "tu-1",
        Kind::Turn,
        "Cortex",
        json!({
            "user_message": "hello",
            "assistant_message": "world"
        }),
    )];
    let report = indexer.index_batch(&events).await.expect("index_batch");
    assert_eq!(report.documents_upserted, 1);
    assert_eq!(report.by_index.get("cortex-cortex-turns").copied(), Some(1),);
    // No global lane fan-out for `Kind::Turn`.
    let written: Vec<String> = client
        .calls_snapshot()
        .into_iter()
        .filter_map(|c| match c {
            MemoryCall::UpsertDocuments { name, .. } => Some(name),
            _ => None,
        })
        .collect();
    assert!(
        !written.iter().any(|n| n == "cortex_turns"),
        "non-governance kinds must not dual-write; saw {written:?}",
    );
}
