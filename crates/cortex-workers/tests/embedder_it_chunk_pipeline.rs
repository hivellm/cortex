//! Pure-Rust integration tests covering the chunker pipeline end-to-end.
//!
//! These tests do not touch the network; they run under the default
//! `cargo test` invocation.

use cortex_workers::classifier::{ClassifierOutput, ClassifierSource, PiiRisk, Severity};
use cortex_core::events::Kind;
use cortex_workers::embedder::{
    Chunk, ChunkSource, Chunker, CodeChunker, DocChunker, EnrichedEvent, FallbackChunker,
};
use serde_json::json;

fn classifier() -> ClassifierOutput {
    ClassifierOutput {
        event_id: "evt".into(),
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

fn enriched(id: &str, kind: Kind, path: Option<&str>, content: &str) -> EnrichedEvent {
    let mut cls = classifier();
    cls.event_id = id.into();
    EnrichedEvent {
        event_id: id.into(),
        kind,
        content_hash: format!("parent-{id}"),
        redacted_payload: json!({ "content": content }),
        classifier: cls,
        context_repo: None,
        context_path: path.map(|s| s.to_string()),
        parent_event_id: None,
        session_id: None,
    }
}

#[test]
fn rust_source_1000_loc_round_trips() {
    // 10 top-level fns, each ~100 lines, names `fn_0` … `fn_9`.
    let mut source = String::new();
    for i in 0..10 {
        source.push_str(&format!("fn fn_{i}() {{\n"));
        for j in 0..98 {
            source.push_str(&format!("    let _line_{j} = {j};\n"));
        }
        source.push_str("}\n\n");
    }
    // Sanity — we did claim 1000-LOC territory.
    assert!(source.lines().count() >= 1_000, "source too short");

    let event = enriched("evt_rust_big", Kind::ToolCall, Some("big.rs"), &source);
    let chunks = CodeChunker::new().chunk(&event).unwrap();
    assert_eq!(chunks.len(), 10);

    for (i, chunk) in chunks.iter().enumerate() {
        assert_eq!(chunk.metadata.source, ChunkSource::Code);
        assert_eq!(chunk.metadata.language.as_deref(), Some("rust"));
        assert_eq!(
            chunk.metadata.symbol.as_deref(),
            Some(format!("fn_{i}").as_str()),
            "symbol name for chunk {i}"
        );
        let (start, end) = chunk.metadata.byte_range.expect("byte range present");
        assert_eq!(
            chunk.text,
            &source[start as usize..end as usize],
            "round-trip slice mismatch on chunk {i}"
        );
    }
}

#[test]
fn oversize_rust_fn_windows_losslessly() {
    // Single fn with a 12 KB body — exceeds the 8 KB oversize threshold,
    // so CodeChunker windows it through FallbackChunker.
    let mut body = String::new();
    while body.len() < 12 * 1024 {
        body.push_str("    let _n: usize = 123456789;\n");
    }
    let source = format!("fn huge() {{\n{body}}}\n");
    assert!(source.len() > 8 * 1024);

    let event = enriched("evt_huge", Kind::ToolCall, Some("huge.rs"), &source);
    let chunks = CodeChunker::new().chunk(&event).unwrap();
    assert!(
        chunks.len() > 1,
        "expected multiple fallback windows, got {}",
        chunks.len()
    );

    // Every chunk must be tagged as a fallback window carrying the original
    // language label, and byte-ranges must be monotonically non-decreasing.
    let mut last_start = 0u32;
    for chunk in &chunks {
        assert_eq!(chunk.metadata.source, ChunkSource::FallbackWindow);
        assert_eq!(chunk.metadata.language.as_deref(), Some("rust"));
        let (start, _end) = chunk.metadata.byte_range.expect("byte range present");
        assert!(
            start >= last_start,
            "byte_start not monotonic: {start} < {last_start}"
        );
        last_start = start;
    }

    // Lossless reconstruction: walk the chunks in order, reconstruct the
    // slice covering the entire windowed region, and verify it matches the
    // declaration in the source. Windows overlap (stride < window), so we
    // reconstruct by taking the stride-sized prefix of every window except
    // the last, which contributes in full.
    let (first_start, _) = chunks[0].metadata.byte_range.unwrap();
    let (_, last_end) = chunks.last().unwrap().metadata.byte_range.unwrap();
    let declared = &source[first_start as usize..last_end as usize];

    let fallback = FallbackChunker::default();
    let stride = fallback.stride_chars();
    let mut reconstructed = String::new();
    for (i, chunk) in chunks.iter().enumerate() {
        if i + 1 == chunks.len() {
            reconstructed.push_str(&chunk.text);
        } else {
            let take = stride.min(chunk.text.len());
            reconstructed.push_str(&chunk.text[..take]);
        }
    }
    assert_eq!(reconstructed, declared, "lossless reconstruction");
}

#[test]
fn markdown_readme_section_paths() {
    let body = "word ".repeat(200); // ~1000 chars each, well above merge threshold
    let md = format!(
        "# Title\n{body}\n## Section\n{body}\n### Sub\n{body}\n",
        body = body
    );
    let event = enriched("evt_md", Kind::Artifact, Some("README.md"), &md);
    let chunks = DocChunker::new().chunk(&event).unwrap();
    assert_eq!(chunks.len(), 3);
    assert_eq!(chunks[0].metadata.symbol.as_deref(), Some("# Title"));
    assert_eq!(
        chunks[1].metadata.symbol.as_deref(),
        Some("# Title > ## Section")
    );
    assert_eq!(
        chunks[2].metadata.symbol.as_deref(),
        Some("# Title > ## Section > ### Sub")
    );
    for c in &chunks {
        assert_eq!(c.metadata.source, ChunkSource::Doc);
    }
}

#[test]
fn unknown_language_falls_back() {
    // `.ex` (Elixir) is not in the Phase-1 grammar set.
    let src = "defmodule Foo do\n  def bar, do: :ok\nend\n";
    let event = enriched("evt_ex", Kind::ToolCall, Some("foo.ex"), src);

    // CodeChunker returns empty for unknown extensions.
    let code_chunks = CodeChunker::new().chunk(&event).unwrap();
    assert!(code_chunks.is_empty());

    // FallbackChunker covers it.
    let fb_chunks = FallbackChunker::default().chunk(&event).unwrap();
    assert_eq!(fb_chunks.len(), 1);
    assert_eq!(fb_chunks[0].metadata.source, ChunkSource::FallbackWindow);
    assert_eq!(fb_chunks[0].text, src);
}

#[test]
fn determinism_across_runs() {
    let mut source = String::new();
    for i in 0..25 {
        source.push_str(&format!("fn handler_{i}() {{\n"));
        for j in 0..18 {
            source.push_str(&format!("    let _x_{j} = {j};\n"));
        }
        source.push_str("}\n\n");
    }
    // ~500 lines total.
    assert!(source.lines().count() >= 500);

    let event = enriched("evt_det", Kind::ToolCall, Some("det.rs"), &source);
    let chunker = CodeChunker::new();
    let a = chunker.chunk(&event).unwrap();
    let b = chunker.chunk(&event).unwrap();
    let c = chunker.chunk(&event).unwrap();

    assert_eq!(a.len(), b.len());
    assert_eq!(a.len(), c.len());
    for i in 0..a.len() {
        assert_eq!(a[i].dedup_key, b[i].dedup_key, "dedup_key drift at {i}");
        assert_eq!(a[i].dedup_key, c[i].dedup_key, "dedup_key drift at {i}");
        assert_eq!(a[i].metadata.byte_range, b[i].metadata.byte_range);
        assert_eq!(a[i].metadata.symbol, b[i].metadata.symbol);
    }

    // Also check no surprise duplicates in the dedup-key set.
    let keys: Vec<&str> = a.iter().map(|c: &Chunk| c.dedup_key.as_str()).collect();
    let mut sorted: Vec<&str> = keys.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), keys.len(), "duplicate dedup_keys");
}
