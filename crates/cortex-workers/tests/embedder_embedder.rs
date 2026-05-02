//! Integration tests for `cortex_workers::embedder::embedder`.

use cortex_classifier::{ClassifierOutput, ClassifierSource, PiiRisk, Severity};
use cortex_core::events::Kind;
use cortex_workers::embedder::embedder::OVERSIZE_CHUNK_BYTES;
use cortex_workers::embedder::{
    Chunk, ChunkSource, EmbedError, Embedder, EmbedderConfig, EnrichedEvent, MemoryCall,
    MemoryVectorizerClient, VectorizerEmbedder,
};
use serde_json::json;
use std::collections::BTreeSet;
use std::sync::Arc;

fn make_event(
    id: &str,
    kind: Kind,
    path: Option<&str>,
    content: &str,
    summary: Option<&str>,
) -> EnrichedEvent {
    EnrichedEvent {
        event_id: id.to_string(),
        kind,
        content_hash: format!("parent-{id}"),
        redacted_payload: json!({ "content": content }),
        classifier: ClassifierOutput {
            event_id: id.to_string(),
            kind_refinement: None,
            topics: vec!["topic".into()],
            severity: Severity::Info,
            pii_risk: PiiRisk::Low,
            redaction_suggestions: vec![],
            summary: summary.map(|s| s.to_string()),
            entities: Vec::new(),
            relations: Vec::new(),
            source: ClassifierSource::StaticFallback,
            prompt_version: "v1".into(),
            model: "static-v1".into(),
            latency_ms: 0,
            tokens_in: 0,
            tokens_out: 0,
        },
        context_repo: None,
        context_path: path.map(|s| s.to_string()),
        parent_event_id: None,
        session_id: None,
    }
}

fn default_config() -> EmbedderConfig {
    EmbedderConfig {
        upsert_batch: 64,
        ..EmbedderConfig::default()
    }
}

#[tokio::test]
async fn mixed_kind_events_route_to_correct_collections() {
    let client = Arc::new(MemoryVectorizerClient::default());
    let embedder = VectorizerEmbedder::new(default_config(), client.clone());

    let code = make_event(
        "evt_code",
        Kind::ToolCall,
        Some("sample.rs"),
        "fn hello() {}\nfn world() {}\n",
        None,
    );
    let doc = make_event(
        "evt_doc",
        Kind::Artifact,
        Some("README.md"),
        &format!("# Title\n{}", "x".repeat(400)),
        None,
    );
    let turn = make_event("evt_turn", Kind::Turn, None, "user said hi", None);

    let report = embedder
        .embed_batch(&[code, doc, turn])
        .await
        .expect("embed_batch");
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert!(report.by_collection.contains_key("cortex-unknown-code"));
    assert!(report.by_collection.contains_key("cortex-unknown-docs"));
    assert!(report.by_collection.contains_key("cortex-unknown-turns"));

    let collections_touched: BTreeSet<String> = client
        .calls()
        .into_iter()
        .filter_map(|c| match c {
            MemoryCall::Upsert(name, _) => Some(name),
            _ => None,
        })
        .collect();
    assert!(collections_touched.contains("cortex-unknown-code"));
    assert!(collections_touched.contains("cortex-unknown-docs"));
    assert!(collections_touched.contains("cortex-unknown-turns"));
}

#[tokio::test]
async fn oversize_with_summary_produces_summary_plus_raw_oversize() {
    let client = Arc::new(MemoryVectorizerClient::default());
    let embedder = VectorizerEmbedder::new(default_config(), client.clone());

    let big_body = "lorem ipsum dolor sit amet consectetur adipiscing. ".repeat(200);
    assert!(big_body.len() > OVERSIZE_CHUNK_BYTES);
    let md = format!("# Big section\n{body}\n", body = big_body);

    let event = make_event(
        "evt_over",
        Kind::Artifact,
        Some("doc.md"),
        &md,
        Some("this is the summary text"),
    );

    let report = embedder.embed_batch(&[event]).await.expect("embed_batch");
    assert!(report.errors.is_empty(), "{:?}", report.errors);

    let uploaded: Vec<Chunk> = client
        .calls()
        .into_iter()
        .filter_map(|c| match c {
            MemoryCall::Upsert(_, chunks) => Some(chunks),
            _ => None,
        })
        .flatten()
        .collect();

    let summaries: Vec<&Chunk> = uploaded
        .iter()
        .filter(|c| c.metadata.source == ChunkSource::Summary)
        .collect();
    let raw_oversize: Vec<&Chunk> = uploaded
        .iter()
        .filter(|c| c.metadata.source == ChunkSource::RawOversize)
        .collect();

    assert_eq!(summaries.len(), 1, "exactly one Summary record expected");
    assert_eq!(
        raw_oversize.len(),
        1,
        "exactly one RawOversize record expected"
    );
    assert_eq!(summaries[0].text, "this is the summary text");
    assert_eq!(summaries[0].metadata.prompt_version.as_deref(), Some("v1"));
    assert!(raw_oversize[0].text.contains("lorem ipsum"));
}

#[tokio::test]
async fn oversize_without_summary_raises_embed_error() {
    let client = Arc::new(MemoryVectorizerClient::default());
    let embedder = VectorizerEmbedder::new(default_config(), client.clone());

    let big_body = "lorem ipsum dolor sit amet consectetur adipiscing. ".repeat(200);
    let md = format!("# Big section\n{body}\n", body = big_body);
    let event = make_event("evt_bad", Kind::Artifact, Some("doc.md"), &md, None);

    let report = embedder.embed_batch(&[event]).await.expect("embed_batch");
    assert_eq!(report.errors.len(), 1);
    assert!(matches!(
        report.errors[0],
        EmbedError::OversizeWithoutSummary { .. }
    ));
    assert_eq!(report.chunks_written, 0);

    let upserts = client
        .calls()
        .into_iter()
        .filter(|c| matches!(c, MemoryCall::Upsert(_, _)))
        .count();
    assert_eq!(upserts, 0);
}

#[tokio::test]
async fn empty_payload_is_skipped() {
    let client = Arc::new(MemoryVectorizerClient::default());
    let embedder = VectorizerEmbedder::new(default_config(), client.clone());

    let mut event = make_event("evt_empty", Kind::Turn, None, "", None);
    event.redacted_payload = json!({ "content": "   " });

    let report = embedder.embed_batch(&[event]).await.expect("embed_batch");
    assert_eq!(report.chunks_skipped_empty, 1);
    assert_eq!(report.chunks_written, 0);

    let upserts = client
        .calls()
        .into_iter()
        .filter(|c| matches!(c, MemoryCall::Upsert(_, _)))
        .count();
    assert_eq!(upserts, 0);
}

#[tokio::test]
async fn idempotent_replay_reports_deduped_on_second_run() {
    let client = Arc::new(MemoryVectorizerClient::default());
    let embedder = VectorizerEmbedder::new(default_config(), client.clone());

    let event = make_event(
        "evt_replay",
        Kind::ToolCall,
        Some("sample.rs"),
        "fn alpha() {}\nfn beta() {}\n",
        None,
    );

    let first = embedder
        .embed_batch(std::slice::from_ref(&event))
        .await
        .expect("first batch");
    assert!(first.chunks_written > 0);
    assert_eq!(first.chunks_deduped, 0);
    assert_eq!(
        first.new_records.len(),
        first.chunks_written as usize,
        "each first-run write must appear in new_records"
    );
    for rec in &first.new_records {
        assert!(
            !rec.server_id.is_empty(),
            "memory client must mint a server_id: {rec:?}"
        );
    }

    let second = embedder.embed_batch(&[event]).await.expect("second batch");
    assert_eq!(second.chunks_written, 0);
    assert_eq!(second.chunks_deduped, first.chunks_written);
    assert!(
        second.new_records.is_empty(),
        "replay must produce no new_records, got {:?}",
        second.new_records
    );
}

#[tokio::test]
async fn large_batch_splits_upserts_correctly() {
    let client = Arc::new(MemoryVectorizerClient::default());
    let embedder = VectorizerEmbedder::new(default_config(), client.clone());

    let mut source = String::new();
    for i in 0..150 {
        source.push_str(&format!("fn f_{i:03}() {{}}\n"));
    }
    let event = make_event("evt_big", Kind::ToolCall, Some("mega.rs"), &source, None);

    let report = embedder.embed_batch(&[event]).await.expect("embed_batch");
    assert_eq!(report.chunks_written, 150);
    assert_eq!(
        report.new_records.len(),
        150,
        "all 150 writes must surface in new_records"
    );

    let upsert_call_sizes: Vec<usize> = client
        .calls()
        .into_iter()
        .filter_map(|c| match c {
            MemoryCall::Upsert(_, chunks) => Some(chunks.len()),
            _ => None,
        })
        .collect();
    assert_eq!(upsert_call_sizes, vec![64, 64, 22]);
}
