//! Integration tests for `cortex_embedder::chunker_doc`.

use cortex_classifier::{ClassifierOutput, ClassifierSource, PiiRisk, Severity};
use cortex_core::events::Kind;
use cortex_embedder::chunker_doc::LONG_FENCE_LINES;
use cortex_embedder::{ChunkSource, Chunker, DocChunker, EnrichedEvent};
use serde_json::json;

fn make_event(md: &str) -> EnrichedEvent {
    EnrichedEvent {
        event_id: "evt_doc".into(),
        kind: Kind::Artifact,
        content_hash: "hash_parent".into(),
        redacted_payload: json!({ "content": md }),
        classifier: ClassifierOutput {
            event_id: "evt_doc".into(),
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
        context_path: Some("README.md".into()),
    }
}

#[test]
fn splits_three_sections_with_correct_symbol_paths() {
    let body = "x".repeat(400);
    let md = format!(
        "# Intro\n{body}\n## Why\n{body}\n### Detail\n{body}\n",
        body = body
    );
    let event = make_event(&md);
    let chunks = DocChunker::new().chunk(&event).unwrap();
    assert_eq!(chunks.len(), 3);
    assert_eq!(chunks[0].metadata.source, ChunkSource::Doc);
    assert_eq!(chunks[0].metadata.symbol.as_deref(), Some("# Intro"));
    assert_eq!(
        chunks[1].metadata.symbol.as_deref(),
        Some("# Intro > ## Why")
    );
    assert_eq!(
        chunks[2].metadata.symbol.as_deref(),
        Some("# Intro > ## Why > ### Detail")
    );
}

#[test]
fn tiny_section_is_merged_into_next() {
    let big = "z".repeat(400);
    let md = format!("# Tiny\nshort\n## Big\n{big}\n", big = big);
    let event = make_event(&md);
    let chunks = DocChunker::new().chunk(&event).unwrap();
    assert_eq!(chunks.len(), 1);
    let only = &chunks[0];
    assert!(only.text.contains("# Tiny"));
    assert!(only.text.contains("## Big"));
    assert!(only.text.contains(&big));
}

#[test]
fn long_fence_is_extracted_as_separate_code_chunk() {
    let body = "x".repeat(400);
    let code: String = "println!(\"hi\");\n".repeat(LONG_FENCE_LINES + 5);
    let md = format!(
        "# Code carrier\n{body}\n```rust\n{code}```\n{body}\n",
        body = body,
        code = code,
    );
    let event = make_event(&md);
    let chunks = DocChunker::new().chunk(&event).unwrap();
    assert_eq!(chunks.len(), 2, "one section + one extracted fence");

    let doc_chunk = &chunks[0];
    assert_eq!(doc_chunk.metadata.source, ChunkSource::Doc);
    assert!(
        doc_chunk.text.contains("[[code-block:0]]"),
        "marker must survive in the doc chunk body"
    );
    assert!(!doc_chunk.text.contains("println!"));

    let code_chunk = &chunks[1];
    assert_eq!(code_chunk.metadata.source, ChunkSource::Code);
    assert_eq!(code_chunk.metadata.language.as_deref(), Some("rust"));
    assert_eq!(
        code_chunk.metadata.symbol.as_deref(),
        Some("# Code carrier")
    );
    assert!(code_chunk.text.contains("println!(\"hi\");"));
}

#[test]
fn short_fence_is_not_extracted() {
    let body = "y".repeat(400);
    let code: String = "a = 1\n".repeat(5);
    let md = format!(
        "# Short code\n{body}\n```python\n{code}```\n",
        body = body,
        code = code,
    );
    let event = make_event(&md);
    let chunks = DocChunker::new().chunk(&event).unwrap();
    assert_eq!(chunks.len(), 1);
    assert!(chunks[0].text.contains("```python"));
    assert!(chunks[0].text.contains("a = 1"));
}

#[test]
fn chunking_is_idempotent() {
    let body = "z".repeat(400);
    let md = format!("# A\n{body}\n## B\n{body}\n", body = body);
    let event = make_event(&md);
    let a = DocChunker::new().chunk(&event).unwrap();
    let b = DocChunker::new().chunk(&event).unwrap();
    assert_eq!(a.len(), b.len());
    for (ca, cb) in a.iter().zip(b.iter()) {
        assert_eq!(ca.dedup_key, cb.dedup_key);
        assert_eq!(ca.chunk_content_hash, cb.chunk_content_hash);
        assert_eq!(ca.metadata.byte_range, cb.metadata.byte_range);
    }
}

#[test]
fn heading_inside_fence_is_not_a_boundary() {
    let body_a = "q".repeat(400);
    let body_b = "r".repeat(400);
    let md = format!(
        "# Real\n{body_a}\n```\n# not a heading\n```\n{body_b}\n## After\n{body_b}\n",
        body_a = body_a,
        body_b = body_b,
    );
    let event = make_event(&md);
    let chunks = DocChunker::new().chunk(&event).unwrap();
    assert_eq!(chunks.len(), 2);
    assert_eq!(chunks[0].metadata.symbol.as_deref(), Some("# Real"));
    assert_eq!(
        chunks[1].metadata.symbol.as_deref(),
        Some("# Real > ## After")
    );
}
