//! Live-Vectorizer integration tests. Gated on `CORTEX_EMBEDDER_IT=1`.
//!
//! Each test creates a uniquely-named collection (`cortex-it-<ulid>-<suffix>`)
//! and deletes it at the end. Dev volume is disposable so cleanup failures
//! are logged but do not fail the test.

use std::sync::Arc;
use std::time::Duration;

use cortex_classifier::{ClassifierOutput, ClassifierSource, PiiRisk, Severity};
use cortex_core::events::Kind;
use cortex_embedder::{
    Embedder, EmbedderConfig, EnrichedEvent, VectorizerClient, VectorizerClientError,
    VectorizerEmbedder,
};
use serde_json::json;
use vectorizer_sdk::models::SimilarityMetric;

mod common;
use common::*;

#[tokio::test]
async fn ensure_collection_is_idempotent() {
    if skip_if_not_it() {
        return;
    }
    let client = live_client().await;
    let name = unique_collection("ensure-idem");

    let schema = it_schema();
    client
        .ensure_collection(&name, &schema)
        .await
        .expect("first ensure_collection");

    // Second call must succeed without error — the schema check should pass
    // because the stored dim/metric match what we just wrote.
    client
        .ensure_collection(&name, &schema)
        .await
        .expect("second ensure_collection");

    drop_collection(&client, &name).await;
}

#[tokio::test]
async fn upsert_batch_of_64_succeeds() {
    if skip_if_not_it() {
        return;
    }
    let client = live_client().await;
    let name = unique_collection("upsert-64");
    client
        .ensure_collection(&name, &it_schema())
        .await
        .expect("ensure_collection");

    // Build 64 chunks with distinct ids + distinct content so the server
    // actually embeds each one.
    let chunks: Vec<_> = (0..64)
        .map(|i| {
            make_chunk(
                &format!("it64-{i:02}"),
                &name,
                &format!(
                    "alpha beta gamma payload number {i} — embed me please.",
                ),
            )
        })
        .collect();

    let report = client
        .upsert_chunks(&name, &chunks)
        .await
        .expect("upsert_chunks");
    assert_eq!(
        report.written, 64,
        "expected 64 successful inserts, got {report:?}"
    );
    assert_eq!(
        report.new_entries.len(),
        64,
        "expected 64 new-entry mappings, got {}",
        report.new_entries.len()
    );
    // Each mapping must carry a non-empty server-assigned id, and round
    // every client dedup key back to itself.
    let mut seen_keys: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
    for entry in &report.new_entries {
        assert!(
            !entry.server_id.is_empty(),
            "empty server_id in {entry:?}"
        );
        assert!(
            entry.dedup_key.starts_with("it64-"),
            "unexpected dedup_key round-trip: {entry:?}"
        );
        seen_keys.insert(entry.dedup_key.as_str());
    }
    assert_eq!(seen_keys.len(), 64, "duplicate dedup_key in new_entries");

    // List view sanity-check.
    let ready = wait_until(
        || async { server_vector_count(&client, &name).await >= 64 },
        Duration::from_secs(15),
    )
    .await;
    assert!(
        ready,
        "server never reported 64 vectors; got {}",
        server_vector_count(&client, &name).await
    );

    drop_collection(&client, &name).await;
}

#[tokio::test]
async fn exists_returns_correct_subset() {
    if skip_if_not_it() {
        return;
    }
    // Server-behaviour note (open server bug — ADR 0001 / knowledge
    // anti-pattern `vectorizer-sdk-3-0-drifts-from-hivehub-vectorizer-3-0-0-dev-image`):
    // the `hivehub/vectorizer:3.0.x` server reassigns every stored vector
    // a fresh UUID on `POST /insert_texts`, discarding the caller's `id`
    // field. Round 8 stopped fighting that and adopted the server UUID as
    // canonical — the deterministic client identifier lives in metadata
    // as `dedup_key` and `LiveVectorizerClient::exists_by_dedup_key`
    // walks the list view to answer idempotency questions. This test
    // exercises that contract: the list endpoint must round-trip the
    // dedup keys through the metadata payload.
    let client = live_client().await;
    let name = unique_collection("exists");
    client
        .ensure_collection(&name, &it_schema())
        .await
        .expect("ensure_collection");

    let chunks: Vec<_> = (0..10)
        .map(|i| {
            make_chunk(
                &format!("it-{i:02}"),
                &name,
                &format!("subset-test payload body number {i}"),
            )
        })
        .collect();
    client
        .upsert_chunks(&name, &chunks)
        .await
        .expect("upsert");

    // Wait for the server to surface the ten vectors via its list view.
    let total_ready = wait_until(
        || async { server_vector_count(&client, &name).await >= 10 },
        Duration::from_secs(10),
    )
    .await;
    assert!(
        total_ready,
        "expected 10 vectors to be visible via /collections/{name}/vectors"
    );

    // Probe `exists_by_dedup_key` for a mix of stored and missing keys.
    // Strict contract: `{it-00, it-05}` — the two stored keys, NOT the
    // fabricated `nonexistent-id`. The list-view-based implementation in
    // `LiveVectorizerClient::exists_by_dedup_key` makes this reliable
    // against the 3.0.x server despite its id-reassignment bug.
    let probe = vec!["it-00".to_string(), "it-05".into(), "nonexistent-id".into()];
    let got = client
        .exists_by_dedup_key(&name, &probe)
        .await
        .expect("exists_by_dedup_key");

    assert!(
        !got.contains("nonexistent-id"),
        "regression: server reported nonexistent-id as present: {got:?}"
    );
    assert_same_ids(&got, &["it-00", "it-05"]);

    drop_collection(&client, &name).await;
}

/// Paginated count of server-side vectors in a collection.
async fn server_vector_count(
    client: &cortex_embedder::LiveVectorizerClient,
    collection: &str,
) -> usize {
    let url = format!(
        "{}/collections/{}/vectors?limit=200",
        client.config().vectorizer_url.trim_end_matches('/'),
        collection,
    );
    let http = reqwest::Client::new();
    let mut req = http.get(&url);
    if let Some(token) = client.config().vectorizer_password.as_deref() {
        req = req.header("Authorization", format!("Bearer {token}"));
    }
    match req.send().await {
        Ok(resp) => match resp.json::<serde_json::Value>().await {
            Ok(body) => body
                .get("total")
                .and_then(|v| v.as_u64())
                .map(|n| n as usize)
                .unwrap_or(0),
            Err(_) => 0,
        },
        Err(_) => 0,
    }
}

#[tokio::test]
async fn idempotent_replay_reports_zero_new() {
    if skip_if_not_it() {
        return;
    }
    let prefix = format!("cortex-it-{}", ulid::Ulid::new().to_string().to_lowercase());
    let base = it_config_authed(Some(&prefix)).await;
    let config = EmbedderConfig {
        collection_prefix: prefix.clone(),
        ..base
    };
    let live_arc: Arc<dyn VectorizerClient> = Arc::new(
        cortex_embedder::LiveVectorizerClient::new(config.clone())
            .expect("live client for replay test"),
    );
    let embedder =
        VectorizerEmbedder::new(config.clone(), live_arc.clone()).with_schema(it_schema());

    // Use a rust event so CodeChunker emits deterministic chunk ids.
    let source = (0..3)
        .map(|i| format!("fn replay_{i}() {{\n    let _x = {i};\n}}\n"))
        .collect::<String>();
    let event = EnrichedEvent {
        event_id: "evt_replay".into(),
        kind: Kind::ToolCall,
        content_hash: "parent-replay".into(),
        redacted_payload: json!({ "content": source }),
        classifier: ClassifierOutput {
            event_id: "evt_replay".into(),
            kind_refinement: None,
            topics: vec![],
            severity: Severity::Info,
            pii_risk: PiiRisk::Low,
            redaction_suggestions: vec![],
            summary: None,
            source: ClassifierSource::StaticFallback,
            prompt_version: "v1".into(),
            model: "static-v1".into(),
            latency_ms: 0,
            tokens_in: 0,
            tokens_out: 0,
        },
        context_repo: None,
        context_path: Some("replay.rs".into()),
        parent_event_id: None,
        session_id: None,
    };

    // Contract: even with the server's UUID-reassignment bug (ADR 0001),
    // the orchestrator's `exists` pre-check — now backed by the list
    // view that round-trips client ids via metadata — produces strict
    // dedup on the second run. The second call writes zero and reports
    // `chunks_deduped == first.chunks_written`.
    let first = embedder
        .embed_batch(std::slice::from_ref(&event))
        .await
        .expect("first embed_batch");
    let collection =
        cortex_embedder::collection_for(&Kind::ToolCall, &config.collection_prefix);

    assert!(first.chunks_written > 0, "first run must write: {first:?}");
    assert_eq!(first.chunks_deduped, 0, "first run has nothing to dedup");
    assert_eq!(
        first.new_records.len(),
        first.chunks_written as usize,
        "each written chunk must produce a new_records entry"
    );
    for rec in &first.new_records {
        assert!(!rec.server_id.is_empty(), "empty server_id in {rec:?}");
    }

    // Wait for the server to surface every chunk via the list view so the
    // second run's `exists_by_dedup_key` pre-check can see them.
    let first_count = wait_for_count(&collection, first.chunks_written as usize).await;
    assert!(
        first_count >= first.chunks_written as usize,
        "server list never surfaced all {} chunks (got {first_count})",
        first.chunks_written
    );

    let second = embedder
        .embed_batch(std::slice::from_ref(&event))
        .await
        .expect("second embed_batch");

    // Strict dedup contract — enabled now that
    // `LiveVectorizerClient::exists_by_dedup_key` round-trips client
    // dedup keys through the list-view metadata payload.
    assert_eq!(
        second.chunks_written, 0,
        "replay must write zero: {second:?}"
    );
    assert_eq!(
        second.chunks_deduped, first.chunks_written,
        "replay dedup must match first-run writes"
    );
    assert!(
        second.new_records.is_empty(),
        "replay must produce no new_records, got {:?}",
        second.new_records
    );

    drop_collection_by_name(&collection).await;
}

async fn wait_for_count(collection: &str, at_least: usize) -> usize {
    let client = live_client().await;
    wait_until(
        || async { server_vector_count(&client, collection).await >= at_least.max(1) },
        Duration::from_secs(10),
    )
    .await;
    server_vector_count(&client, collection).await
}

async fn drop_collection_by_name(name: &str) {
    let client = live_client().await;
    drop_collection(&client, name).await;
}

#[tokio::test]
async fn schema_drift_fails_fast() {
    if skip_if_not_it() {
        return;
    }
    let client = live_client().await;
    let name = unique_collection("drift");

    // Create the collection directly against the SDK with dim=512 so the
    // embedder's default (768) disagrees.
    client
        .sdk()
        .create_collection(&name, 512, Some(SimilarityMetric::Cosine))
        .await
        .expect("pre-create collection at dim=512");

    let mut schema = it_schema();
    schema.dim = 768;
    let res = client.ensure_collection(&name, &schema).await;
    match res {
        Err(VectorizerClientError::SchemaMismatch {
            collection, detail,
        }) => {
            assert_eq!(collection, name);
            assert!(
                detail.contains("dimension") || detail.contains("dim"),
                "expected dim mismatch in detail, got: {detail}"
            );
        }
        other => panic!("expected SchemaMismatch, got {other:?}"),
    }

    drop_collection(&client, &name).await;
}

// Suppress unused-import warning in non-IT cargo runs — the common module's
// helpers are fully used only when the suite actually runs against the live
// stack.
#[allow(dead_code)]
fn _classifier_type_is_referenced(_c: ClassifierOutput) {}
