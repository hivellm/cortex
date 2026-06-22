//! Fixed-window fallback chunker.
//!
//! Used when the language is unknown or a Tree-sitter grammar is missing.
//! Produces sliding windows of 512 tokens (≈ 2048 chars) with a 128-token
//! (≈ 512-char) stride. Approximating one token at four characters is good
//! enough for deterministic chunking — Vectorizer re-tokenises anyway — and
//! keeps this path free of external tokenizer dependencies.

use anyhow::Result;
use cortex_core::events::Kind;
use sha2::{Digest, Sha256};

use super::chunker::{Chunk, ChunkMetadata, ChunkSource, Chunker};
use super::embedder::EnrichedEvent;
use super::identity::dedup_key;
use super::routing::collection_for;

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
        let collection = collection_for(
            &event.kind,
            collection_prefix,
            event.context_repo.as_deref(),
        );
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
        let collection = collection_for(&event.kind, "cortex", event.context_repo.as_deref());
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
    let mut metadata = ChunkMetadata {
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
        project_id: None,
        branch_id: None,
        lifecycle: None,
        valid_from_unix: None,
        valid_to_unix: None,
        superseded_at_unix: None,
    };
    metadata.stamp_bitemporal(event);
    Chunk {
        dedup_key: key,
        parent_event_id: event.event_id.clone(),
        parent_content_hash: event.content_hash.clone(),
        chunk_content_hash: chunk_hash,
        collection: collection.to_string(),
        text: text.to_string(),
        metadata,
    }
}

/// Extract the chunkable text from a redacted payload.
///
/// Priority:
/// 1. Classifier summary when produced by a real model (non-empty).
/// 2. Per-kind deterministic NL projection — human-readable fields
///    instead of raw JSON, so NL queries embed meaningfully even when
///    the classifier runs in static-fallback mode.
/// 3. Legacy common string keys: `content` → `text` → `body`.
/// 4. Deterministic pretty-JSON so two runs of the same event always
///    produce identical windows.
pub fn event_text(event: &EnrichedEvent) -> String {
    if let Some(s) = event.classifier.summary.as_deref() {
        if !s.is_empty() {
            return s.to_string();
        }
    }
    let proj = nl_projection(event);
    if !proj.is_empty() {
        return proj;
    }
    for key in ["content", "text", "body"] {
        if let Some(s) = event.redacted_payload.get(key).and_then(|v| v.as_str()) {
            return s.to_string();
        }
    }
    serde_json::to_string_pretty(&event.redacted_payload).unwrap_or_default()
}

/// Build a deterministic NL projection for an event by extracting
/// human-readable payload fields instead of embedding raw JSON.
/// Returns an empty string when no meaningful projection is possible.
fn nl_projection(event: &EnrichedEvent) -> String {
    let p = &event.redacted_payload;
    match &event.kind {
        Kind::Turn => join_fields(&[
            p.get("user_message").and_then(|v| v.as_str()),
            p.get("assistant_message").and_then(|v| v.as_str()),
        ]),
        Kind::ToolCall => {
            let tool = p.get("tool_name").and_then(|v| v.as_str()).unwrap_or("");
            let input = p
                .get("input")
                .map(|v| serde_json::to_string(v).unwrap_or_default())
                .unwrap_or_default();
            let output = p
                .get("output")
                .and_then(|o| o.get("text").and_then(|v| v.as_str()))
                .unwrap_or("");
            let mut parts = vec![format!("{tool}({input})")];
            if !output.is_empty() {
                parts.push(output.to_string());
            }
            parts.join("\n")
        }
        Kind::AgentCall => join_fields(&[
            p.get("description").and_then(|v| v.as_str()),
            p.get("prompt").and_then(|v| v.as_str()),
        ]),
        Kind::Memory => join_fields(&[
            p.get("name").and_then(|v| v.as_str()),
            p.get("body").and_then(|v| v.as_str()),
            p.get("description").and_then(|v| v.as_str()),
        ]),
        Kind::Decision => join_fields(&[
            p.get("title").and_then(|v| v.as_str()),
            p.get("status").and_then(|v| v.as_str()),
            p.get("body").and_then(|v| v.as_str()),
        ]),
        Kind::Analysis => join_fields(&[p.get("question").and_then(|v| v.as_str())]),
        Kind::Law => join_fields(&[
            p.get("title").and_then(|v| v.as_str()),
            p.get("body").and_then(|v| v.as_str()),
        ]),
        Kind::LawViolation => join_fields(&[p.get("message").and_then(|v| v.as_str())]),
        Kind::Artifact => join_fields(&[
            event.context_path.as_deref(),
            p.get("body").and_then(|v| v.as_str()),
        ]),
        Kind::Knowledge => join_fields(&[
            p.get("title").and_then(|v| v.as_str()),
            p.get("body").and_then(|v| v.as_str()),
        ]),
        Kind::Learning => join_fields(&[
            p.get("title").and_then(|v| v.as_str()),
            p.get("body").and_then(|v| v.as_str()),
        ]),
        Kind::Consolidation => {
            let summary = p
                .get("summary_markdown")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let takeaways: Vec<&str> = p
                .get("takeaways")
                .and_then(|v| v.as_array())
                .map(|arr| arr.iter().filter_map(|t| t.as_str()).collect())
                .unwrap_or_default();
            let mut parts: Vec<&str> = Vec::new();
            if !summary.is_empty() {
                parts.push(summary);
            }
            let joined_takeaways;
            if !takeaways.is_empty() {
                joined_takeaways = takeaways.join("\n");
                parts.push(&joined_takeaways);
            }
            parts.join("\n\n")
        }
        Kind::TopicCard => p
            .get("synthesis_markdown")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
    }
}

/// Join non-empty `Option<&str>` fields with newlines.
fn join_fields(parts: &[Option<&str>]) -> String {
    parts
        .iter()
        .filter_map(|s| *s)
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
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
    use serde_json::json;

    use crate::classifier::{ClassifierOutput, ClassifierSource, PiiRisk, Severity};
    use super::super::embedder::EnrichedEvent;
    use cortex_core::events::Kind;

    use super::*;

    fn static_classifier() -> ClassifierOutput {
        ClassifierOutput {
            event_id: "01EVT".into(),
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

    fn event(kind: Kind, payload: serde_json::Value) -> EnrichedEvent {
        EnrichedEvent {
            event_id: "01EVT".into(),
            kind,
            content_hash: "h".into(),
            redacted_payload: payload,
            classifier: static_classifier(),
            context_repo: Some("cortex".into()),
            context_path: None,
            parent_event_id: None,
            session_id: None,
            occurred_at_ms: 0,
        }
    }

    #[test]
    fn nl_projection_turn_uses_user_and_assistant_message() {
        let e = event(
            Kind::Turn,
            json!({"user_message": "How does X work?", "assistant_message": "X works by Y."}),
        );
        let text = event_text(&e);
        assert!(text.contains("How does X work?"), "missing user_message");
        assert!(text.contains("X works by Y."), "missing assistant_message");
    }

    #[test]
    fn nl_projection_tool_call_uses_tool_name_and_input() {
        let e = event(
            Kind::ToolCall,
            json!({"tool_name": "Read", "input": {"file_path": "/foo.rs"}, "output": {"text": "fn main(){}"}}),
        );
        let text = event_text(&e);
        assert!(text.contains("Read"), "missing tool_name");
        assert!(text.contains("fn main(){}"), "missing output.text");
    }

    #[test]
    fn nl_projection_decision_includes_title_status_body() {
        let e = event(
            Kind::Decision,
            json!({"title": "ADR-016 Config migration", "status": "accepted", "body": "Move all env reads to cortex_config."}),
        );
        let text = event_text(&e);
        assert!(text.contains("ADR-016 Config migration"), "missing title");
        assert!(text.contains("accepted"), "missing status");
        assert!(text.contains("Move all env reads"), "missing body");
    }

    #[test]
    fn nl_projection_consolidation_uses_summary_markdown_and_takeaways() {
        let e = event(
            Kind::Consolidation,
            json!({
                "summary_markdown": "## Summary\nWe fixed the wiring.",
                "takeaways": ["TEI service must use --auto-truncate", "Reranker MRR +36%"]
            }),
        );
        let text = event_text(&e);
        assert!(text.contains("We fixed the wiring."), "missing summary_markdown");
        assert!(text.contains("TEI service"), "missing takeaways");
    }

    #[test]
    fn classifier_summary_wins_over_nl_projection() {
        let mut e = event(
            Kind::Turn,
            json!({"user_message": "ignored if summary present"}),
        );
        e.classifier.summary = Some("Classifier produced this summary.".into());
        let text = event_text(&e);
        assert_eq!(
            text, "Classifier produced this summary.",
            "classifier summary must take priority"
        );
    }

    #[test]
    fn nl_projection_topic_card_uses_synthesis_markdown() {
        let e = event(
            Kind::TopicCard,
            json!({"synthesis_markdown": "# Topic\nThis is the synthesis."}),
        );
        let text = event_text(&e);
        assert!(text.contains("This is the synthesis."), "missing synthesis_markdown");
    }

    #[test]
    fn nl_projection_knowledge_uses_title_and_body() {
        let e = event(
            Kind::Knowledge,
            json!({"title": "Anti-pattern: raw-JSON embedding", "body": "Always project NL text."}),
        );
        let text = event_text(&e);
        assert!(text.contains("Anti-pattern: raw-JSON embedding"), "missing title");
        assert!(text.contains("Always project NL text."), "missing body");
    }

    #[test]
    fn fallback_chunk_str_produces_windows() {
        let chunker = FallbackChunker::new(4, 2); // 4-token window = 16 chars
        let text = "abcdefghijklmnopqrstuvwxyz";
        let e = event(Kind::Turn, json!({"user_message": text}));
        let chunks = chunker.chunk(&e).unwrap();
        assert!(!chunks.is_empty(), "must produce at least one chunk");
        for c in &chunks {
            assert!(!c.text.is_empty());
        }
    }

    #[test]
    fn sha256_hex_is_deterministic() {
        assert_eq!(sha256_hex("hello"), sha256_hex("hello"));
        assert_ne!(sha256_hex("hello"), sha256_hex("world"));
        assert_eq!(sha256_hex("hello").len(), 64);
    }
}
