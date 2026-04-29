//! Integration tests for `cortex_workers::graph::writer::NexusGraphWriter`.
//!
//! Drives the writer end-to-end against a [`MemoryNexusClient`] so the
//! `write_batch` / `write_patches` flow is exercised without booting
//! Nexus. Covers happy-path metrics + the "edges silently dropped"
//! reconciliation branch.

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use cortex_classifier::{ClassifierOutput, ClassifierSource, PiiRisk, Severity};
use cortex_core::events::Kind;
use cortex_workers::graph::cypher::CypherTemplates;
use cortex_workers::graph::nexus_client::{
    GraphClient, GraphClientError, MemoryCall, MemoryNexusClient, WriteStats,
};
use cortex_workers::graph::patch::GraphPatch;
use cortex_workers::graph::writer::{GraphWriter, NexusGraphWriter};
use cortex_workers::graph::{EnrichedEvent, GraphConfig, Metrics};
use serde_json::{json, Value};

fn classifier(event_id: &str) -> ClassifierOutput {
    ClassifierOutput {
        event_id: event_id.into(),
        kind_refinement: None,
        topics: vec![],
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

fn enriched(event_id: &str, kind: Kind, payload: Value) -> EnrichedEvent {
    EnrichedEvent {
        event_id: event_id.into(),
        kind,
        content_hash: format!("h-{event_id}"),
        redacted_payload: payload,
        classifier: classifier(event_id),
        context_repo: Some("hivellm/cortex".into()),
        context_path: Some("src/lib.rs".into()),
        parent_event_id: None,
        session_id: None,
    }
}

fn writer(client: Arc<dyn GraphClient>) -> NexusGraphWriter {
    let templates = Arc::new(CypherTemplates::default());
    let metrics = Arc::new(Metrics::default());
    NexusGraphWriter::new(GraphConfig::default(), client, templates, metrics)
}

#[tokio::test]
async fn write_batch_maps_events_and_records_call() {
    let client = Arc::new(MemoryNexusClient::new());
    let w = writer(client.clone());

    let evt = enriched(
        "01TEST000000000000000000001",
        Kind::Artifact,
        json!({
            "language": "rust",
            "size_bytes": 64,
            "text": "fn main() {}",
        }),
    );
    let report = w.write_batch(&[evt]).await.expect("write_batch");

    // Memory client returned nodes_upserted = patch.nodes.len()
    // (artifact + repo for Artifact). One edge (in_repo).
    assert!(report.nodes_upserted >= 1);
    assert_eq!(report.nodes_deduped, 0);
    // by_label populated.
    assert!(!report.by_label.is_empty());
    // One write call recorded — schema not pre-bootstrapped here.
    let calls = client.calls_snapshot();
    assert_eq!(calls.len(), 1);
    assert!(matches!(calls[0], MemoryCall::WriteTx(_)));
}

#[tokio::test]
async fn write_patches_with_empty_input_yields_zero_report() {
    let client = Arc::new(MemoryNexusClient::new());
    let w = writer(client);

    let report = w.write_patches(Vec::new()).await.expect("empty write");
    assert_eq!(report.nodes_upserted, 0);
    assert_eq!(report.edges_upserted, 0);
    assert_eq!(report.nodes_deduped, 0);
    assert!(report.by_label.is_empty());
}

#[tokio::test]
async fn write_batch_increments_metrics_on_persisted_edges() {
    let client = Arc::new(MemoryNexusClient::new());
    let w = writer(client);

    let evt = enriched(
        "01TEST000000000000000000002",
        Kind::Artifact,
        json!({
            "title": "README",
            "size_bytes": 32,
            "text": "# hi",
            "language": "markdown",
        }),
    );
    let _ = w.write_batch(&[evt]).await.expect("write");

    // Metrics expose per-label maps directly (no aggregate snapshot).
    let nodes_total: u64 = w
        .metrics()
        .nodes_upserted
        .lock()
        .unwrap()
        .values()
        .copied()
        .sum();
    assert!(nodes_total >= 1, "nodes_upserted total = {nodes_total}");
}

#[tokio::test]
async fn config_and_metrics_accessors_round_trip() {
    let client = Arc::new(MemoryNexusClient::new());
    let cfg = GraphConfig::default();
    let templates = Arc::new(CypherTemplates::default());
    let metrics = Arc::new(Metrics::default());
    let w = NexusGraphWriter::new(cfg.clone(), client, templates, metrics.clone());

    // Accessors return the same handles passed in.
    assert_eq!(w.config().nexus_url, cfg.nexus_url);
    assert!(Arc::ptr_eq(w.metrics(), &metrics));
}

// ---------- dropping-edges branch ----------

/// Client that pretends Nexus silently dropped every edge of a given
/// type — exercises the reconcile-attempted-vs-persisted branch in
/// `write_patches`.
struct DroppingNexusClient {
    drop_type: String,
}

#[async_trait]
impl GraphClient for DroppingNexusClient {
    async fn ensure_schema(&self, _statements: &[String]) -> Result<(), GraphClientError> {
        Ok(())
    }

    async fn run_write_tx(
        &self,
        patch: &GraphPatch,
        _templates: &CypherTemplates,
    ) -> Result<WriteStats, GraphClientError> {
        let attempted_in_type = patch
            .edges
            .iter()
            .filter(|e| e.edge_type == self.drop_type)
            .count() as u32;
        let total_edges = patch.edges.len() as u32;
        let persisted = total_edges.saturating_sub(attempted_in_type);
        let mut edges_dropped = BTreeMap::new();
        if attempted_in_type > 0 {
            edges_dropped.insert(self.drop_type.clone(), attempted_in_type);
        }
        Ok(WriteStats {
            nodes_upserted: patch.nodes.len() as u32,
            edges_upserted: persisted,
            edges_dropped,
        })
    }
}

#[tokio::test]
async fn write_batch_records_dropped_edges_into_metrics() {
    // Force every IN_REPO edge to be reported as dropped (matches
    // the upper-case label emitted by the mapper for Artifact events).
    let client = Arc::new(DroppingNexusClient {
        drop_type: "IN_REPO".into(),
    });
    let w = writer(client);

    let evt = enriched(
        "01TEST000000000000000000003",
        Kind::Artifact,
        json!({
            "language": "rust",
            "size_bytes": 12,
            "text": "fn x(){}",
        }),
    );
    let _ = w.write_batch(&[evt]).await.expect("write");

    let dropped_total: u64 = w
        .metrics()
        .edges_dropped_snapshot()
        .values()
        .copied()
        .sum();
    assert!(
        dropped_total >= 1,
        "edges_dropped total = {dropped_total}"
    );
}
