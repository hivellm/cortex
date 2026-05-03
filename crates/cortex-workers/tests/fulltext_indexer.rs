//! Integration tests for the Meili-backed `FulltextIndexer`.

use std::sync::Arc;

use cortex_classifier::{ClassifierOutput, ClassifierSource, PiiRisk, Severity};
use cortex_core::events::Kind;
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
        topics: vec!["topic".into()],
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

fn event(event_id: &str, kind: Kind, payload: serde_json::Value) -> EnrichedEvent {
    EnrichedEvent {
        event_id: event_id.to_string(),
        kind,
        content_hash: format!("h-{event_id}"),
        redacted_payload: payload,
        classifier: classifier(event_id),
        context_repo: Some("Vectorizer".into()),
        context_path: None,
        parent_event_id: None,
        session_id: None,
    }
}

fn build_indexer() -> (
    Arc<MemoryMeiliClient>,
    Arc<MeiliFulltextIndexer>,
    Arc<Metrics>,
) {
    let client = Arc::new(MemoryMeiliClient::new());
    let metrics = Arc::new(Metrics::new());
    let cfg = FulltextConfig {
        upsert_batch: 4,
        ..FulltextConfig::default()
    };
    let indexer = Arc::new(MeiliFulltextIndexer::new(
        cfg,
        client.clone(),
        metrics.clone(),
    ));
    (client, indexer, metrics)
}

#[tokio::test]
async fn index_batch_groups_events_per_index() {
    let (client, indexer, _metrics) = build_indexer();
    let events = vec![
        event(
            "tc-1",
            Kind::ToolCall,
            json!({
                "tool_name": "Edit",
                "input": { "command": "x" },
                "outcome": "success",
                "touched": []
            }),
        ),
        event(
            "tc-2",
            Kind::ToolCall,
            json!({
                "tool_name": "Read",
                "input": { "command": "y" },
                "outcome": "success",
                "touched": []
            }),
        ),
        event(
            "dec-1",
            Kind::Decision,
            json!({
                "decision_id": "DEC-1",
                "title": "x",
                "status": "accepted",
                "body": "details"
            }),
        ),
    ];
    let report = indexer.index_batch(&events).await.expect("index_batch");
    // Phase11k §2.2 — Decision events dual-write to the global
    // `cortex_decisions` index in addition to the per-repo target.
    // 2 ToolCall (per-repo only) + 1 Decision (per-repo + global) = 4.
    assert_eq!(report.documents_upserted, 4);
    assert_eq!(
        report.by_index.get("cortex-vectorizer-code").copied(),
        Some(2)
    );
    assert_eq!(
        report.by_index.get("cortex-vectorizer-decisions").copied(),
        Some(1)
    );
    assert_eq!(report.by_index.get("cortex_decisions").copied(), Some(1));

    let calls = client.calls_snapshot();
    let mut indexes_seen: Vec<String> = calls
        .into_iter()
        .filter_map(|c| match c {
            MemoryCall::UpsertDocuments { name, .. } => Some(name),
            _ => None,
        })
        .collect();
    indexes_seen.sort();
    assert_eq!(
        indexes_seen,
        vec![
            "cortex-vectorizer-code".to_string(),
            "cortex-vectorizer-decisions".to_string(),
            "cortex_decisions".to_string(),
        ]
    );
}

#[tokio::test]
async fn empty_payload_event_is_counted_as_skipped() {
    let (_client, indexer, metrics) = build_indexer();
    let events = vec![event(
        "evt-empty",
        Kind::Artifact,
        json!({ "artifact_type": "file" }),
    )];
    let report = indexer.index_batch(&events).await.expect("index_batch");
    assert_eq!(report.documents_upserted, 0);
    assert_eq!(report.documents_skipped, 1);
    assert_eq!(
        metrics
            .skipped_empty
            .load(std::sync::atomic::Ordering::Relaxed),
        1
    );
}

#[tokio::test]
async fn oversize_event_increments_truncated_counter_and_flag() {
    let (_client, indexer, metrics) = build_indexer();
    let raw = "z".repeat(20 * 1024);
    let events = vec![event("evt-big", Kind::Turn, json!({ "user_message": raw }))];
    // Small max_body_bytes to force truncation regardless of payload.
    let small_cfg = FulltextConfig {
        upsert_batch: 4,
        max_body_bytes: 1024,
        ..FulltextConfig::default()
    };
    let mc = Arc::new(MemoryMeiliClient::new());
    let metrics2 = Arc::new(Metrics::new());
    let indexer2 = MeiliFulltextIndexer::new(small_cfg, mc.clone(), metrics2.clone());
    let report = indexer2.index_batch(&events).await.expect("index_batch");
    assert_eq!(report.documents_upserted, 1);
    assert_eq!(report.documents_truncated, 1);
    assert_eq!(
        metrics2
            .truncated
            .load(std::sync::atomic::Ordering::Relaxed),
        1
    );
    let _ = (indexer, metrics);
}

/// Spec-08 §Routing matrix: a mixed batch should fan out across every
/// destination family (`code`, `docs`, `decisions`, `governance`,
/// `turns`, `misc`). The `routed_total` counter must mirror the
/// `IndexReport.by_index` numbers exactly.
#[tokio::test]
async fn routing_matrix_distributes_mixed_batch_across_families() {
    let (_client, indexer, metrics) = build_indexer();

    // ToolCall → cortex-vectorizer-code.
    let mut tool_call = event(
        "tc-1",
        Kind::ToolCall,
        json!({
            "tool_name": "Edit",
            "input": { "command": "edit foo" },
            "outcome": "success",
            "touched": []
        }),
    );
    tool_call.classifier.topics = vec!["code".into()];

    // AgentCall → cortex-vectorizer-turns (per spec-08).
    let mut agent_call = event(
        "ag-1",
        Kind::AgentCall,
        json!({
            "agent_type": "researcher",
            "description": "research",
            "outcome": "success",
            "duration_ms": 5
        }),
    );
    agent_call.classifier.topics = vec!["agent".into()];

    // Turn → cortex-vectorizer-turns.
    let turn = event(
        "tu-1",
        Kind::Turn,
        json!({
            "user_message": "hi",
            "assistant_message": "hello"
        }),
    );

    // Decision → cortex-vectorizer-decisions.
    let decision = event(
        "dec-1",
        Kind::Decision,
        json!({
            "decision_id": "DEC-1",
            "title": "Adopt X",
            "status": "accepted",
            "body": "rationale"
        }),
    );

    // LawViolation → cortex-vectorizer-governance.
    let law = event(
        "lv-1",
        Kind::LawViolation,
        json!({
            "violation_id": "VIO-1",
            "law_id": "LAW-1",
            "severity": "critical",
            "tier": 1,
            "message": "broke a rule",
            "evidence": null
        }),
    );

    // Artifact (.rs path) → cortex-vectorizer-code.
    let mut code_artifact = event(
        "art-rs",
        Kind::Artifact,
        json!({
            "artifact_type": "file",
            "path": "src/lib.rs",
            "body": "fn main() {}"
        }),
    );
    code_artifact.context_path = Some("src/lib.rs".into());

    // Artifact (.md path) → cortex-vectorizer-docs.
    let mut doc_artifact = event(
        "art-md",
        Kind::Artifact,
        json!({
            "artifact_type": "file",
            "path": "docs/spec-08.md",
            "body": "# Spec"
        }),
    );
    doc_artifact.context_path = Some("docs/spec-08.md".into());

    // Artifact with unknown ext + no topics → cortex-vectorizer-misc.
    let mut misc_artifact = event(
        "art-bin",
        Kind::Artifact,
        json!({
            "artifact_type": "file",
            "path": "tools/blob.bin",
            "body": "raw"
        }),
    );
    misc_artifact.context_path = Some("tools/blob.bin".into());
    misc_artifact.classifier.topics = vec![];

    let events = vec![
        tool_call,
        agent_call,
        turn,
        decision,
        law,
        code_artifact,
        doc_artifact,
        misc_artifact,
    ];

    let report = indexer.index_batch(&events).await.expect("index_batch");
    // Phase11k §2.2 — governance kinds dual-write to global indexes:
    // 6 non-governance + 1 Decision (per-repo + global) + 1 LawViolation
    // (per-repo + global) = 10 upserts.
    assert_eq!(report.documents_upserted, 10);

    // Every destination from the spec-08 matrix should be populated.
    let want = [
        ("cortex-vectorizer-code", 2),  // tool_call + .rs artifact
        ("cortex-vectorizer-turns", 2), // turn + agent_call
        ("cortex-vectorizer-decisions", 1),
        ("cortex-vectorizer-governance", 1),
        ("cortex-vectorizer-docs", 1), // .md artifact
        ("cortex-vectorizer-misc", 1), // unknown-ext artifact
        // Phase11k §2.2 — global dual-write targets.
        ("cortex_decisions", 1),
        ("cortex_laws", 1),
    ];
    for (idx, expected) in want {
        assert_eq!(
            report.by_index.get(idx).copied(),
            Some(expected),
            "index {idx} count drift: by_index={:?}",
            report.by_index
        );
    }

    // The routed_total counter must mirror by_index exactly so the
    // operator dashboard and the per-batch report cannot diverge.
    let routed = metrics.routed_snapshot();
    for (idx, expected) in want {
        assert_eq!(
            routed.get(idx).copied(),
            Some(expected as u64),
            "routed_total mismatch for {idx}: snapshot={:?}",
            routed
        );
    }
}

#[tokio::test]
async fn batches_chunk_by_upsert_batch_size() {
    let (client, indexer, _metrics) = build_indexer();
    // 9 ToolCall events with upsert_batch=4 ⇒ 3 chunks for the
    // `cortex-code` index (4 + 4 + 1).
    let mut events = Vec::with_capacity(9);
    for i in 0..9 {
        events.push(event(
            &format!("tc-{i}"),
            Kind::ToolCall,
            json!({
                "tool_name": "Edit",
                "input": { "command": format!("cmd-{i}") },
                "outcome": "success",
                "touched": []
            }),
        ));
    }
    indexer.index_batch(&events).await.expect("index_batch");
    let upserts: Vec<usize> = client
        .calls_snapshot()
        .into_iter()
        .filter_map(|c| match c {
            MemoryCall::UpsertDocuments { docs, .. } => Some(docs.len()),
            _ => None,
        })
        .collect();
    assert_eq!(upserts, vec![4, 4, 1]);
}

#[tokio::test]
async fn await_task_branch_runs_and_succeeds() {
    // With `await_task = true` the indexer reaches `wait_task` after
    // every upsert; the memory client returns `Succeeded` by default,
    // so the batch must complete without error and report all docs
    // upserted. (The MemoryMeiliClient does not log wait_task calls
    // into its `calls` log, so observation goes through the report.)
    let client = Arc::new(MemoryMeiliClient::new());
    let metrics = Arc::new(Metrics::new());
    let cfg = FulltextConfig {
        await_task: true,
        upsert_batch: 4,
        ..FulltextConfig::default()
    };
    let indexer = MeiliFulltextIndexer::new(cfg, client.clone(), metrics);
    let evt = event(
        "tc-await",
        Kind::ToolCall,
        json!({
            "tool_name": "Edit",
            "input": {"command": "x"},
            "outcome": "success",
            "touched": [],
        }),
    );
    let report = indexer.index_batch(&[evt]).await.expect("await branch");
    assert_eq!(report.documents_upserted, 1);
}

#[tokio::test]
async fn with_ensured_skips_settings_for_seeded_indexes() {
    let client = Arc::new(MemoryMeiliClient::new());
    let metrics = Arc::new(Metrics::new());
    let cfg = FulltextConfig {
        upsert_batch: 4,
        ..FulltextConfig::default()
    };
    // Pre-seed the cortex-code index as already-ensured. The
    // indexer must NOT call ensure_index for it on the first batch.
    // Index name follows `cortex-{slug}-{family}` — no context_repo
    // → slug is `unknown`; ToolCall → family `code`.
    let indexer = MeiliFulltextIndexer::with_ensured(
        cfg,
        client.clone(),
        metrics,
        vec!["cortex-vectorizer-code".to_string()],
    );
    let evt = event(
        "tc-ensured",
        Kind::ToolCall,
        json!({
            "tool_name": "Edit",
            "input": {"command": "x"},
            "outcome": "success",
            "touched": [],
        }),
    );
    indexer.index_batch(&[evt]).await.expect("ensured branch");
    let ensure_names: Vec<String> = client
        .calls_snapshot()
        .into_iter()
        .filter_map(|c| match c {
            MemoryCall::EnsureIndex { name, .. } => Some(name),
            _ => None,
        })
        .collect();
    assert_eq!(
        ensure_names,
        Vec::<String>::new(),
        "pre-seeded index must not trigger ensure_index — saw {:?}",
        ensure_names
    );
}

#[tokio::test]
async fn ensure_settings_runs_only_once_per_index() {
    let (client, indexer, _metrics) = build_indexer();
    let evt_a = event(
        "a-1",
        Kind::ToolCall,
        json!({
            "tool_name": "Edit",
            "input": {"command": "1"},
            "outcome": "success",
            "touched": [],
        }),
    );
    let evt_b = event(
        "a-2",
        Kind::ToolCall,
        json!({
            "tool_name": "Edit",
            "input": {"command": "2"},
            "outcome": "success",
            "touched": [],
        }),
    );
    // Two batches against the same index → ensure_index should only
    // fire once.
    indexer.index_batch(&[evt_a]).await.unwrap();
    indexer.index_batch(&[evt_b]).await.unwrap();
    let ensure_calls: usize = client
        .calls_snapshot()
        .into_iter()
        .filter(|c| matches!(c, MemoryCall::EnsureIndex { .. }))
        .count();
    assert_eq!(
        ensure_calls, 1,
        "ensure_index must only be called once per index across batches"
    );
}

#[test]
fn config_and_metrics_accessors_round_trip() {
    let client = Arc::new(MemoryMeiliClient::new());
    let metrics = Arc::new(Metrics::new());
    let cfg = FulltextConfig {
        upsert_batch: 32,
        ..FulltextConfig::default()
    };
    let indexer = MeiliFulltextIndexer::new(cfg.clone(), client, metrics.clone());
    assert_eq!(indexer.config().upsert_batch, 32);
    assert!(Arc::ptr_eq(indexer.metrics(), &metrics));
}
