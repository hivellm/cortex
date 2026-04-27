//! Integration tests for `cortex_embedder::chunker_code`.

use cortex_classifier::{ClassifierOutput, ClassifierSource, PiiRisk, Severity};
use cortex_core::events::Kind;
use cortex_embedder::chunker_code::OVERSIZE_THRESHOLD_BYTES;
use cortex_embedder::{ChunkSource, Chunker, CodeChunker, EnrichedEvent};
use serde_json::json;

fn make_event(path: &str, content: &str) -> EnrichedEvent {
    EnrichedEvent {
        event_id: "evt_code".into(),
        kind: Kind::ToolCall,
        content_hash: "hash_parent".into(),
        redacted_payload: json!({ "content": content }),
        classifier: ClassifierOutput {
            event_id: "evt_code".into(),
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
        context_path: Some(path.into()),
        parent_event_id: None,
        session_id: None,
    }
}

#[test]
fn rust_file_with_ten_top_level_fns_produces_ten_chunks() {
    let mut source = String::new();
    let mut expected_symbols: Vec<String> = Vec::new();
    for i in 0..10 {
        let name = format!("handler_{i:02}");
        expected_symbols.push(name.clone());
        let pad = "    let _x = 1; let _y = 2; let _z = 3;\n".repeat(100);
        source.push_str(&format!("fn {name}() {{\n{pad}}}\n\n"));
    }

    let event = make_event("sample.rs", &source);
    let chunks = CodeChunker::new().chunk(&event).unwrap();
    assert_eq!(chunks.len(), 10, "one chunk per top-level fn");

    for (i, chunk) in chunks.iter().enumerate() {
        assert_eq!(chunk.metadata.source, ChunkSource::Code);
        assert_eq!(chunk.metadata.language.as_deref(), Some("rust"));
        assert_eq!(
            chunk.metadata.symbol.as_deref(),
            Some(expected_symbols[i].as_str())
        );
        let (start, end) = chunk.metadata.byte_range.expect("byte range present");
        assert!(end > start, "byte range non-empty");
        assert!((end as usize) <= source.len());
        assert_eq!(
            chunk.text,
            &source[start as usize..end as usize],
            "chunk text should equal source slice"
        );
    }
}

#[test]
fn oversize_rust_fn_is_windowed() {
    let big_body = "let _n: usize = 123456789;\n".repeat(400);
    let src = format!("fn huge() {{\n{body}}}\n", body = big_body);
    assert!(src.len() > OVERSIZE_THRESHOLD_BYTES);

    let event = make_event("huge.rs", &src);
    let chunks = CodeChunker::new().chunk(&event).unwrap();
    assert!(
        chunks.len() > 1,
        "oversize fn must produce multiple fallback windows, got {}",
        chunks.len()
    );
    for chunk in &chunks {
        assert_eq!(chunk.metadata.source, ChunkSource::FallbackWindow);
        assert_eq!(chunk.metadata.language.as_deref(), Some("rust"));
        let (start, end) = chunk.metadata.byte_range.expect("byte range present");
        assert!(end > start);
        assert!((end as usize) <= src.len());
    }
}

#[test]
fn unknown_extension_returns_empty() {
    let event = make_event("program.ex", "defmodule Foo do\n  def bar, do: :ok\nend\n");
    let chunks = CodeChunker::new().chunk(&event).unwrap();
    assert!(chunks.is_empty());
}

#[test]
fn chunks_are_deterministic_across_runs() {
    let src = "fn alpha() {}\nfn beta() {}\nstruct Gamma;\n";
    let event = make_event("d.rs", src);
    let a = CodeChunker::new().chunk(&event).unwrap();
    let b = CodeChunker::new().chunk(&event).unwrap();
    assert_eq!(a.len(), b.len());
    assert_eq!(a.len(), 3);
    for (ca, cb) in a.iter().zip(b.iter()) {
        assert_eq!(ca.dedup_key, cb.dedup_key);
        assert_eq!(ca.metadata.symbol, cb.metadata.symbol);
        assert_eq!(ca.metadata.byte_range, cb.metadata.byte_range);
    }
}
