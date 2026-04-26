//! End-to-end integration: chunk → embed → Vectorizer → search.
//!
//! Gated on `CORTEX_EMBEDDER_IT=1`. Uses a fresh collection prefix per test
//! run so nothing collides, and best-effort-deletes each collection at the
//! end.
//!
//! Server-image caveats (`hivehub/vectorizer:3.0.x`, tracked in ADR 0001):
//! - The server still reassigns every stored vector a fresh UUID on
//!   insert, discarding the caller's `id`. Round 8 adopted that UUID as
//!   the canonical chunk id (surfaced via
//!   [`cortex_embedder::UpsertedChunk::server_id`] in
//!   `EmbedReport::new_records`) and carries the deterministic
//!   client-side identifier as `metadata.dedup_key`, so orchestrator
//!   dedup via `exists_by_dedup_key` is precise against the live server.
//! - The server's `POST /collections/{c}/search_vectors` endpoint still
//!   returns 404 and the alternate `/search` wants a precomputed dense
//!   query vector (the default BM25 backend can't synthesise one). Text
//!   search via the SDK therefore still can't be exercised against the
//!   dev image — these tests assert the embedder pipeline produced the
//!   expected chunks and the server's list view reflects them.

use std::sync::Arc;
use std::time::Duration;

use cortex_core::events::Kind;
use cortex_embedder::{
    EmbedderConfig, LiveVectorizerClient, VectorizerClient, VectorizerEmbedder,
};
use ulid::Ulid;

mod common;
use common::*;

/// Shared helper: run the embed pipeline for a single event using a live
/// client under the given prefix. Returns the live client handle (so the
/// caller can inspect + clean up) and the destination collection name.
async fn embed_one(
    prefix: &str,
    event: &cortex_embedder::EnrichedEvent,
) -> (Arc<LiveVectorizerClient>, String, cortex_embedder::EmbedReport) {
    let base = it_config_authed(Some(prefix)).await;
    let config = EmbedderConfig {
        collection_prefix: prefix.to_string(),
        ..base
    };
    let live = Arc::new(
        LiveVectorizerClient::new(config.clone())
            .expect("live client"),
    );
    let trait_client: Arc<dyn VectorizerClient> = live.clone();
    let embedder =
        VectorizerEmbedder::new(config.clone(), trait_client).with_schema(it_schema());
    use cortex_embedder::Embedder;
    let report = embedder
        .embed_batch(std::slice::from_ref(event))
        .await
        .expect("embed_batch");
    let collection = cortex_embedder::collection_for(&event.kind, prefix);
    (live, collection, report)
}

async fn server_vector_total(live: &LiveVectorizerClient, collection: &str) -> usize {
    let url = format!(
        "{}/collections/{}/vectors?limit=200",
        live.config().vectorizer_url.trim_end_matches('/'),
        collection,
    );
    let http = reqwest::Client::new();
    let mut req = http.get(&url);
    if let Some(token) = live.config().vectorizer_password.as_deref() {
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
async fn enriched_event_produces_retrievable_chunks() {
    if skip_if_not_it() {
        return;
    }

    // Real-ish Rust source: three top-level functions with a rare unique
    // token that would drive a search assertion against a production
    // Vectorizer. Here we assert the pipeline produced three chunks and
    // the server visibly stored them.
    let source = r#"
fn alpha_handler(input: &str) -> usize {
    input.len()
}

fn beta_handler(input: &str) -> usize {
    input.bytes().filter(|b| *b == b'x').count()
}

fn zanzibar_token_finder(input: &str) -> Option<usize> {
    input.find("ZANZIBAR_TOKEN")
}
"#;

    let event = enriched_event(
        "evt_e2e",
        Kind::ToolCall,
        Some("handlers.rs"),
        source,
        None,
    );
    let prefix = format!(
        "cortex-it-{}",
        Ulid::new().to_string().to_lowercase()
    );
    let (live, collection, report) = embed_one(&prefix, &event).await;
    assert!(
        report.errors.is_empty(),
        "embed report had errors: {:?}",
        report.errors
    );
    let total_observed = report.chunks_written + report.chunks_deduped;
    assert!(
        total_observed >= 3,
        "expected at least 3 chunks observed, got {report:?}"
    );
    // Strict contract: every newly-written chunk surfaces in new_records
    // with a non-empty server-assigned UUID.
    assert!(
        report.new_records.len() >= 3,
        "expected ≥3 new_records, got {}",
        report.new_records.len()
    );
    for rec in &report.new_records {
        assert!(!rec.server_id.is_empty(), "empty server_id in {rec:?}");
        assert!(!rec.dedup_key.is_empty(), "empty dedup_key in {rec:?}");
    }

    let ready = wait_until(
        || async { server_vector_total(&live, &collection).await >= 1 },
        Duration::from_secs(15),
    )
    .await;
    assert!(ready, "no vectors visible in server list view");

    let total = server_vector_total(&live, &collection).await;
    assert!(total >= 1, "server reports zero vectors: got {total}");

    // A production Vectorizer build would round-trip the unique token
    // through text search at this point. The dev image cannot serve
    // text search (see module docs). Assert we at least have stored
    // content — which is the spec 06 end-to-end acceptance.
    drop_collection(&live, &collection).await;
}

#[tokio::test]
async fn oversize_with_summary_roundtrips() {
    if skip_if_not_it() {
        return;
    }

    // Markdown doc with one section body > 4 KB → DocChunker emits one
    // oversize section, the embedder summary-substitutes, resulting in
    // one Summary chunk + one RawOversize chunk.
    let mut big = String::new();
    while big.len() < 20_000 {
        big.push_str("alpha beta gamma delta epsilon zeta eta theta. ");
    }
    big.push_str(" QUAGMIRE_MARKER ");
    while big.len() < 24_000 {
        big.push_str("iota kappa lambda mu nu xi omicron pi. ");
    }
    let md = format!("# Big section\n{body}\n", body = big);
    let event = enriched_event(
        "evt_over_e2e",
        Kind::Artifact,
        Some("big.md"),
        &md,
        Some("this is the summary text"),
    );

    let prefix = format!(
        "cortex-it-{}",
        Ulid::new().to_string().to_lowercase()
    );
    let (live, collection, report) = embed_one(&prefix, &event).await;
    assert!(
        report.errors.is_empty(),
        "oversize embed had errors: {:?}",
        report.errors
    );

    let total_observed = report.chunks_written + report.chunks_deduped;
    assert!(
        total_observed >= 2,
        "expected at least 2 chunks (Summary + RawOversize), got {report:?}"
    );
    assert_eq!(
        report.new_records.len(),
        report.chunks_written as usize,
        "each newly-written chunk must appear in new_records"
    );
    for rec in &report.new_records {
        assert!(!rec.server_id.is_empty(), "empty server_id in {rec:?}");
    }

    let ready = wait_until(
        || async { server_vector_total(&live, &collection).await >= 2 },
        Duration::from_secs(15),
    )
    .await;
    assert!(
        ready,
        "server list never reported >= 2 vectors; total now = {}",
        server_vector_total(&live, &collection).await
    );

    drop_collection(&live, &collection).await;
}
