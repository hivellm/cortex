//! Fixed-window fallback chunker.
//!
//! Used when the language is unknown or a Tree-sitter grammar is missing.
//! Produces sliding windows of 512 tokens (≈ 2048 chars) with a 128-token
//! (≈ 512-char) stride. Approximating one token at four characters is good
//! enough for deterministic chunking — Vectorizer re-tokenises anyway — and
//! keeps this path free of external tokenizer dependencies.

use anyhow::Result;
use sha2::{Digest, Sha256};

use crate::chunker::{Chunk, ChunkMetadata, ChunkSource, Chunker};
use crate::embedder::EnrichedEvent;
use crate::identity::dedup_key;
use crate::routing::collection_for;

/// Approximate character width of a single token. Used to translate the
/// spec's 512-token window into a byte/char-level window.
pub const CHARS_PER_TOKEN: usize = 4;

/// Sliding-window fallback chunker.
#[derive(Debug, Clone, Copy)]
pub struct FallbackChunker {
    /// Window size in tokens.
    pub window_tokens: u32,
    /// Stride between windows in tokens.
    pub stride_tokens: u32,
}

impl Default for FallbackChunker {
    fn default() -> Self {
        Self::new(512, 128)
    }
}

impl FallbackChunker {
    /// Create a fallback chunker with explicit window/stride (in tokens).
    pub fn new(window_tokens: u32, stride_tokens: u32) -> Self {
        assert!(window_tokens > 0, "window must be > 0 tokens");
        assert!(stride_tokens > 0, "stride must be > 0 tokens");
        assert!(
            stride_tokens <= window_tokens,
            "stride ({stride_tokens}) must not exceed window ({window_tokens})"
        );
        Self {
            window_tokens,
            stride_tokens,
        }
    }

    /// Window size in characters (≈ 4 chars per token).
    pub fn window_chars(&self) -> usize {
        self.window_tokens as usize * CHARS_PER_TOKEN
    }

    /// Stride in characters (≈ 4 chars per token).
    pub fn stride_chars(&self) -> usize {
        self.stride_tokens as usize * CHARS_PER_TOKEN
    }

    /// Public helper: chunk an arbitrary text blob. Used by `CodeChunker`
    /// when a top-level declaration exceeds the 8 KB threshold and has to be
    /// windowed with a known source language.
    pub fn chunk_text(
        &self,
        event: &EnrichedEvent,
        text: &str,
        language: Option<String>,
        starting_ordinal: u32,
        collection_prefix: &str,
    ) -> Vec<Chunk> {
        let collection = collection_for(&event.kind, collection_prefix);
        chunk_str(
            text,
            self.window_chars(),
            self.stride_chars(),
            |ordinal, slice, byte_range| {
                build_chunk(
                    event,
                    &collection,
                    starting_ordinal + ordinal,
                    slice,
                    byte_range,
                    language.clone(),
                )
            },
        )
    }
}

impl Chunker for FallbackChunker {
    fn chunk(&self, event: &EnrichedEvent) -> Result<Vec<Chunk>> {
        let text = event_text(event);
        if text.is_empty() {
            return Ok(Vec::new());
        }
        // Default routing uses the `cortex` prefix; the orchestrator layer can
        // relabel chunks if it needs a non-default prefix, but most call sites
        // go through `VectorizerEmbedder` which owns its own prefix and will
        // invoke `chunk_text` directly. The `Chunker` trait method keeps a
        // sensible default for stand-alone use (e.g. tests).
        let collection = collection_for(&event.kind, "cortex");
        let chunks = chunk_str(
            &text,
            self.window_chars(),
            self.stride_chars(),
            |ordinal, slice, byte_range| {
                build_chunk(event, &collection, ordinal, slice, byte_range, None)
            },
        );
        Ok(chunks)
    }
}

/// Produce sliding windows over `text`, invoking `emit` for each.
///
/// Byte ranges are on UTF-8 boundaries — windows snap to the nearest character
/// boundary at or below the target offset so the emitted slices are always
/// valid `&str`.
fn chunk_str<F>(text: &str, window: usize, stride: usize, mut emit: F) -> Vec<Chunk>
where
    F: FnMut(u32, &str, (u32, u32)) -> Chunk,
{
    let len = text.len();
    if len == 0 {
        return Vec::new();
    }
    // Pre-collect char boundaries so we can snap window/stride offsets without
    // panicking on multi-byte sequences.
    let boundaries: Vec<usize> = text
        .char_indices()
        .map(|(idx, _)| idx)
        .chain(std::iter::once(len))
        .collect();

    let snap = |byte_offset: usize| -> usize {
        match boundaries.binary_search(&byte_offset) {
            Ok(exact) => boundaries[exact],
            Err(next) => boundaries[next.saturating_sub(1)],
        }
    };

    let mut out = Vec::new();
    let mut ordinal: u32 = 0;
    let mut start = 0usize;
    loop {
        let end = snap(start.saturating_add(window).min(len));
        let start_snapped = snap(start);
        if start_snapped >= end {
            break;
        }
        let slice = &text[start_snapped..end];
        let byte_range = (
            start_snapped.min(u32::MAX as usize) as u32,
            end.min(u32::MAX as usize) as u32,
        );
        out.push(emit(ordinal, slice, byte_range));
        ordinal = ordinal.saturating_add(1);
        if end >= len {
            break;
        }
        start = start_snapped.saturating_add(stride);
    }
    out
}

/// Build a single `Chunk` for the fallback path.
fn build_chunk(
    event: &EnrichedEvent,
    collection: &str,
    ordinal: u32,
    text: &str,
    byte_range: (u32, u32),
    language: Option<String>,
) -> Chunk {
    let chunk_hash = sha256_hex(text);
    let key = dedup_key(&event.event_id, ordinal, &chunk_hash);
    Chunk {
        dedup_key: key,
        parent_event_id: event.event_id.clone(),
        parent_content_hash: event.content_hash.clone(),
        chunk_content_hash: chunk_hash,
        collection: collection.to_string(),
        text: text.to_string(),
        metadata: ChunkMetadata {
            kind: event.kind,
            topics: event.classifier.topics.clone(),
            severity: event.classifier.severity,
            repo: event.context_repo.clone(),
            path: event.context_path.clone(),
            symbol: None,
            byte_range: Some(byte_range),
            language,
            source: ChunkSource::FallbackWindow,
            prompt_version: None,
        },
    }
}

/// Extract the chunkable text from a redacted payload.
///
/// Order of preference: `content` → `text` → `body` (when a `Value::String`);
/// otherwise fall back to deterministic pretty-JSON so two runs of the same
/// event always produce identical windows.
pub fn event_text(event: &EnrichedEvent) -> String {
    for key in ["content", "text", "body"] {
        if let Some(s) = event.redacted_payload.get(key).and_then(|v| v.as_str()) {
            return s.to_string();
        }
    }
    serde_json::to_string_pretty(&event.redacted_payload).unwrap_or_default()
}

/// Hex-encoded SHA-256 of the input.
pub fn sha256_hex(input: &str) -> String {
    let digest = Sha256::digest(input.as_bytes());
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(&mut out, "{byte:02x}");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use cortex_classifier::{ClassifierOutput, ClassifierSource, PiiRisk, Severity};
    use cortex_core::events::Kind;
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

        // window=2048, stride=512 over 5000 chars — windows start at
        // 0, 512, 1024, 1536, 2048, 2560, 3072; the last clamps its end to
        // the document length (5000) and the loop terminates. That is
        // 7 windows.
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
}
