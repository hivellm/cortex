//! Integration tests for `cortex_embedder::chunker_fallback`.

use cortex_classifier::{ClassifierOutput, ClassifierSource, PiiRisk, Severity};
use cortex_core::events::Kind;
use cortex_embedder::{ChunkSource, Chunker, EnrichedEvent, FallbackChunker};
use serde_json::json;

fn make_event(payload_text: &str) -> EnrichedEvent {
    EnrichedEvent {
        event_id: "evt_test".into(),
        kind: Kind::Turn,
        content_hash: "hash_parent".into(),
        redacted_payload: json!({ "content": payload_text }),
        classifier: ClassifierOutput {
            event_id: "evt_test".into(),
            kind_refinement: None,
            topics: vec!["t".into()],
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
        context_path: None,
        parent_event_id: None,
        session_id: None,
    }
}

#[test]
fn short_text_yields_single_chunk() {
    let event = make_event("hello world, short input");
    let chunker = FallbackChunker::default();
    let chunks = chunker.chunk(&event).unwrap();
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].text, "hello world, short input");
    assert_eq!(chunks[0].metadata.source, ChunkSource::FallbackWindow);
    assert_eq!(
        chunks[0].metadata.byte_range,
        Some((0, "hello world, short input".len() as u32))
    );
}

#[test]
fn long_text_produces_expected_window_count() {
    let payload: String = "a".repeat(5_000);
    let event = make_event(&payload);
    let chunker = FallbackChunker::default();
    let chunks = chunker.chunk(&event).unwrap();

    let window = 2048usize;
    let stride = 512usize;
    let len = 5000usize;
    assert_eq!(chunks.len(), 7);

    for (i, chunk) in chunks.iter().enumerate() {
        let expected_start = i * stride;
        let expected_end = (expected_start + window).min(len);
        assert_eq!(
            chunk.metadata.byte_range,
            Some((expected_start as u32, expected_end as u32)),
            "window {i} range mismatch"
        );
    }
}

#[test]
fn dedup_keys_are_deterministic_on_replay() {
    let event = make_event(&"x".repeat(3_000));
    let chunker = FallbackChunker::default();
    let a = chunker.chunk(&event).unwrap();
    let b = chunker.chunk(&event).unwrap();
    assert_eq!(a.len(), b.len());
    for (ca, cb) in a.iter().zip(b.iter()) {
        assert_eq!(ca.dedup_key, cb.dedup_key);
        assert_eq!(ca.chunk_content_hash, cb.chunk_content_hash);
    }
}

#[test]
fn falls_back_to_pretty_json_when_no_text_field() {
    let mut event = make_event("ignored");
    event.redacted_payload = json!({ "random": { "nested": 42 } });
    let chunker = FallbackChunker::default();
    let chunks = chunker.chunk(&event).unwrap();
    assert_eq!(chunks.len(), 1);
    assert!(chunks[0].text.contains("\"random\""));
    assert!(chunks[0].text.contains("\"nested\""));
}
