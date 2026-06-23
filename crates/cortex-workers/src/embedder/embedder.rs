//! High-level [`Embedder`] trait and the [`VectorizerEmbedder`] implementation.
//!
//! Orchestration flow per batch:
//!
//! 1. Pick a chunker per event — [`CodeChunker`] for recognised source
//!    extensions, [`DocChunker`] for markdown, [`FallbackChunker`] otherwise.
//! 2. Apply summary substitution when a raw chunk's text exceeds 4 KB: emit
//!    a `Summary` record in place of the raw text, and a separate
//!    `RawOversize` record that preserves the full text as retrieval context.
//!    Events whose oversize chunks lack a classifier summary surface an
//!    `OversizeWithoutSummary` per-event error and are skipped.
//! 3. Group chunks by destination collection, ensure the collection exists
//!    (spec 06 schema check), pre-filter chunks whose `dedup_key` already
//!    lives in the target collection via
//!    [`VectorizerClient::exists_by_dedup_key`], upsert the remainder in
//!    batches of [`EmbedderConfig::upsert_batch`]. The server-assigned
//!    UUID per newly-written chunk flows back through `UpsertReport.new_entries`.
//! 4. Emit the full metric set from `docs/specs/06-embedder.md` §Observability.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Instant;

use crate::classifier::ClassifierOutput;
use async_trait::async_trait;
use cortex_core::events::Kind;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::chunker::{Chunk, ChunkMetadata, ChunkSource, Chunker};
use super::chunker_code::{detect_language_from_path, CodeChunker};
use super::chunker_doc::DocChunker;
use super::chunker_fallback::{event_text, sha256_hex, FallbackChunker};
use super::config::EmbedderConfig;
use super::identity::dedup_key;
use super::metrics::Metrics;
use super::routing::collection_for;
use super::vectorizer_client::{
    CollectionSchema, UpsertedChunk, VectorizerClient, VectorizerClientError, VectorizerErrorKind,
};

/// Raw-payload size threshold above which a chunk is summary-substituted.
pub const OVERSIZE_CHUNK_BYTES: usize = 4 * 1024;

/// Minimum batch size that triggers the `exists` pre-check optimisation. The
/// call is cheap enough that we always run it, but the spec calls it out as
/// an explicit knob so it's named here.
pub const EXISTS_PRECHECK_MIN_BATCH: usize = 0;

/// Payload handed to the embedder — an event that has already been redacted
/// and classified.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnrichedEvent {
    /// Event envelope id.
    pub event_id: String,
    /// Coarse event kind.
    pub kind: Kind,
    /// Pre-redaction content hash (dedup key).
    pub content_hash: String,
    /// Post-redaction payload.
    pub redacted_payload: Value,
    /// Classifier output.
    pub classifier: ClassifierOutput,
    /// Repo hint from the envelope context.
    pub context_repo: Option<String>,
    /// Path hint from the envelope context.
    pub context_path: Option<String>,
    /// Envelope occurrence time in epoch milliseconds. Stamped from
    /// `Envelope.occurred_at` (RFC3339) by the classifier worker.
    /// Defaults to 0 when an upstream did not parse / forward it —
    /// keeps backwards compat with tests that build EnrichedEvent
    /// without the field. The fulltext worker projects this to
    /// `Document.ts` so the per-repo Meili sortable axis works
    /// (phase20 §5.2 — was previously hard-coded to 0).
    #[serde(default)]
    pub occurred_at_ms: i64,
    /// Parent envelope event id (e.g. the `Turn` that emitted a child
    /// `Decision` / `LawViolation` / `Analysis`). Carried straight through
    /// from `Envelope.parent_event_id`. Consumed by the graph writer to
    /// anchor `LINKED_TO`, `DEBATED_IN`, etc. — `None` for top-level
    /// events that have no parent (like `turn.start` itself).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_event_id: Option<String>,
    /// Owning session id from the canonical envelope. Carried as a
    /// dedicated field (rather than buried inside `redacted_payload`)
    /// because every downstream worker — graph mapper, embedder
    /// metadata, dashboard session aggregator — anchors on it. `None`
    /// when the upstream emitter omitted the field; the graph writer
    /// then falls back to a synthetic per-event session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// phase21 — sensitivity level ordinal (public=0, internal=1, confidential=2,
    /// restricted=3). Stamped by the classification worker after the classifier runs;
    /// None until then. All four enforcement points read this field from the event
    /// metadata when projecting backend filter clauses.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class_level: Option<u8>,
    /// phase21 — orthogonal need-to-know compartment labels (e.g. `["financial","hr"]`).
    /// None is equivalent to an empty set — omitted on the wire when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class_compartments: Option<Vec<String>>,
}

/// Per-event failure surfaced in an [`EmbedReport`].
///
/// The enum keeps machine-readable discrimination (`cause`) while flattening
/// the Vectorizer client error to a string so the whole report stays
/// serialisable.
#[derive(Debug, Clone, Serialize, Deserialize, thiserror::Error, PartialEq, Eq)]
#[serde(tag = "cause", rename_all = "snake_case")]
pub enum EmbedError {
    /// Raw chunk text exceeded 4 KB but the classifier did not supply a summary.
    #[error("oversize_without_summary for {event_id}")]
    OversizeWithoutSummary {
        /// Event id that failed.
        event_id: String,
    },
    /// Upstream Vectorizer failure. Retries are handled inside the client.
    #[error("vectorizer failure for {event_id}: {detail}")]
    Vectorizer {
        /// Event id that failed.
        event_id: String,
        /// Error detail lifted from [`VectorizerClientError`].
        detail: String,
    },
    /// Chunker returned an error (parse failure or unsupported input).
    #[error("chunker failure for {event_id}: {detail}")]
    Chunker {
        /// Event id that failed.
        event_id: String,
        /// Error detail.
        detail: String,
    },
}

impl EmbedError {
    /// Return the event id this error is bound to.
    pub fn event_id(&self) -> &str {
        match self {
            EmbedError::OversizeWithoutSummary { event_id }
            | EmbedError::Vectorizer { event_id, .. }
            | EmbedError::Chunker { event_id, .. } => event_id,
        }
    }
}

impl From<(&str, VectorizerClientError)> for EmbedError {
    fn from((event_id, err): (&str, VectorizerClientError)) -> Self {
        EmbedError::Vectorizer {
            event_id: event_id.to_string(),
            detail: err.to_string(),
        }
    }
}

/// Aggregate report for an `embed_batch` call.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EmbedReport {
    /// Total chunks written to Vectorizer.
    pub chunks_written: u32,
    /// Chunks skipped because Vectorizer already had them.
    pub chunks_deduped: u32,
    /// Events skipped because their redacted payload was empty.
    pub chunks_skipped_empty: u32,
    /// Chunk counts per destination collection.
    pub by_collection: BTreeMap<String, u32>,
    /// Wall-clock latency for the whole batch.
    pub latency_ms: u32,
    /// Per-event errors (partial success is allowed).
    pub errors: Vec<EmbedError>,
    /// Per-chunk `dedup_key` → server-assigned UUID mapping for every
    /// newly-written chunk in this batch. Consumers that need to join
    /// back to the stored vectors (graph writer, query API) key off
    /// `server_id`; pre-existing chunks deduplicated by the `exists`
    /// pre-check are **not** represented here.
    #[serde(default)]
    pub new_records: Vec<UpsertedChunk>,
}

/// Top-level embedder trait — see `docs/specs/06-embedder.md`.
#[async_trait]
pub trait Embedder: Send + Sync {
    /// Chunk, embed (via Vectorizer), and upsert a batch of enriched events.
    async fn embed_batch(&self, events: &[EnrichedEvent]) -> anyhow::Result<EmbedReport>;
}

/// Default [`Embedder`] implementation backed by a [`VectorizerClient`].
pub struct VectorizerEmbedder {
    /// Embedder configuration.
    pub config: EmbedderConfig,
    /// Vectorizer transport.
    pub client: Arc<dyn VectorizerClient>,
    /// Metrics registry shared with the worker.
    pub metrics: Arc<Metrics>,
    /// Collection schema used when ensuring the destination exists. Defaults
    /// to [`CollectionSchema::default`], which reflects the production
    /// Vectorizer model (dim=768, cosine, hybrid BM25+dense); override via
    /// [`VectorizerEmbedder::with_schema`] when targetting a deployment
    /// whose server model has a different dim.
    pub collection_schema: CollectionSchema,
    code: CodeChunker,
    doc: DocChunker,
    fallback: FallbackChunker,
}

impl VectorizerEmbedder {
    /// Build a new embedder from its dependencies. Metrics are initialised to
    /// a fresh registry — see [`VectorizerEmbedder::with_metrics`] when the
    /// caller wants to share a registry with the worker.
    pub fn new(config: EmbedderConfig, client: Arc<dyn VectorizerClient>) -> Self {
        Self::with_metrics(config, client, Arc::new(Metrics::new()))
    }

    /// Build a new embedder, injecting an external metrics registry.
    pub fn with_metrics(
        config: EmbedderConfig,
        client: Arc<dyn VectorizerClient>,
        metrics: Arc<Metrics>,
    ) -> Self {
        let collection_schema = CollectionSchema {
            dim: config.vector_dim,
            ..CollectionSchema::default()
        };
        Self {
            config,
            client,
            metrics,
            collection_schema,
            code: CodeChunker::new(),
            doc: DocChunker::new(),
            fallback: FallbackChunker::default(),
        }
    }

    /// Override the collection schema used by `ensure_collection` calls.
    pub fn with_schema(mut self, schema: CollectionSchema) -> Self {
        self.collection_schema = schema;
        self
    }

    /// Pick the right chunker for an event.
    ///
    /// * `CodeChunker` when the path extension maps to a known grammar.
    /// * `DocChunker` for `.md` / `.markdown` and when the payload looks like
    ///   markdown (starts with a heading).
    /// * `FallbackChunker` otherwise.
    pub fn chunker_for(&self, event: &EnrichedEvent) -> &dyn Chunker {
        if let Some(path) = event.context_path.as_deref() {
            let lower = path.to_ascii_lowercase();
            let ext = lower.rsplit('.').next().unwrap_or("");
            if matches!(ext, "md" | "markdown") {
                return &self.doc;
            }
            if detect_language_from_path(path).is_some() {
                return &self.code;
            }
        }
        // No path hint — sniff for a markdown heading at the start of the
        // raw content.
        let raw = event_text(event);
        if raw.trim_start().starts_with('#') {
            return &self.doc;
        }
        &self.fallback
    }
}

#[async_trait]
impl Embedder for VectorizerEmbedder {
    async fn embed_batch(&self, events: &[EnrichedEvent]) -> anyhow::Result<EmbedReport> {
        let start = Instant::now();
        let mut report = EmbedReport::default();

        // 1. Chunk + summary-substitute. Events that have no content after
        //    redaction get skipped here.
        let mut all_chunks: Vec<Chunk> = Vec::new();
        for event in events {
            // Empty-payload detection: neither a textual payload nor any
            // structured JSON worth pretty-printing.
            let raw = event_text(event);
            if raw.trim().is_empty() {
                report.chunks_skipped_empty = report.chunks_skipped_empty.saturating_add(1);
                continue;
            }

            let chunker = self.chunker_for(event);
            let mut chunks = match chunker.chunk(event) {
                Ok(c) => c,
                Err(err) => {
                    report.errors.push(EmbedError::Chunker {
                        event_id: event.event_id.clone(),
                        detail: err.to_string(),
                    });
                    continue;
                }
            };

            if chunks.is_empty() {
                // Code/doc chunkers may decline on unknown extensions; always
                // fall through to the windowing path so we never drop content
                // silently.
                let prefix = &self.config.collection_prefix;
                chunks = self.fallback.chunk_text(event, &raw, None, 0, prefix);
            }

            // Relabel collections so they honour the configured prefix.
            // For Kind::Artifact we route per-chunk on `ChunkSource`
            // (Code → cortex-code, Doc → cortex-docs); other kinds use
            // the event-level table.
            for chunk in &mut chunks {
                chunk.collection = super::routing::collection_for_chunk(
                    &event.kind,
                    &chunk.metadata.source,
                    &self.config.collection_prefix,
                    event.context_repo.as_deref(),
                );
            }

            // Summary substitution pass.
            let mut summary_error = false;
            let mut substituted: Vec<Chunk> = Vec::with_capacity(chunks.len());
            for (ordinal, chunk) in chunks.into_iter().enumerate() {
                if chunk.text.len() <= OVERSIZE_CHUNK_BYTES {
                    substituted.push(chunk);
                    continue;
                }
                let Some(summary) = event.classifier.summary.as_ref() else {
                    // Surface a per-event error, drop all chunks for this
                    // event, and move on.
                    summary_error = true;
                    self.metrics.incr_oversize_without_summary();
                    break;
                };

                // Summary-substituted chunk: embedded, carries prompt_version.
                let summary_text = summary.clone();
                let summary_hash = sha256_hex(&summary_text);
                let summary_ord = (ordinal * 2) as u32;
                let summary_key = dedup_key(&event.event_id, summary_ord, &summary_hash);
                substituted.push(Chunk {
                    dedup_key: summary_key,
                    parent_event_id: chunk.parent_event_id.clone(),
                    parent_content_hash: chunk.parent_content_hash.clone(),
                    chunk_content_hash: summary_hash,
                    collection: chunk.collection.clone(),
                    text: summary_text,
                    metadata: ChunkMetadata {
                        source: ChunkSource::Summary,
                        prompt_version: Some(event.classifier.prompt_version.clone()),
                        ..chunk.metadata.clone()
                    },
                });

                // Raw-oversize record: shipped to Vectorizer as retrieval
                // context. The SDK surface does not expose a "skip
                // embedding" flag today, so v3 still pays one embed call per
                // raw-oversize record. This is called out in spec 06
                // §Decisions 6 ("Vectorizer is the system of record for
                // chunk text in v1"). When the server grows a per-record
                // embed-skip flag we can re-route this record without
                // changing callers.
                let raw_hash = sha256_hex(&chunk.text);
                let raw_ord = (ordinal * 2 + 1) as u32;
                let raw_key = dedup_key(&event.event_id, raw_ord, &raw_hash);
                substituted.push(Chunk {
                    dedup_key: raw_key,
                    parent_event_id: chunk.parent_event_id.clone(),
                    parent_content_hash: chunk.parent_content_hash.clone(),
                    chunk_content_hash: raw_hash,
                    collection: chunk.collection.clone(),
                    text: chunk.text.clone(),
                    metadata: ChunkMetadata {
                        source: ChunkSource::RawOversize,
                        ..chunk.metadata
                    },
                });
            }

            if summary_error {
                report.errors.push(EmbedError::OversizeWithoutSummary {
                    event_id: event.event_id.clone(),
                });
                continue;
            }

            // Metrics: record each chunk's source + byte size.
            for chunk in &substituted {
                self.metrics.incr_chunks(chunk.metadata.source, 1);
                self.metrics.observe_chunk_bytes(chunk.text.len() as u64);
            }
            all_chunks.extend(substituted);
        }

        if all_chunks.is_empty() {
            report.latency_ms = elapsed_ms(start);
            return Ok(report);
        }

        // 2. Group by collection while preserving input order within each.
        let mut by_collection: BTreeMap<String, Vec<Chunk>> = BTreeMap::new();
        for chunk in all_chunks {
            by_collection
                .entry(chunk.collection.clone())
                .or_default()
                .push(chunk);
        }

        // 3. Ensure every touched collection once.
        let schema = self.collection_schema.clone();
        let mut ensured: BTreeSet<String> = BTreeSet::new();
        for name in by_collection.keys() {
            if ensured.contains(name) {
                continue;
            }
            if let Err(err) = self.client.ensure_collection(name, &schema).await {
                // Schema drift / hard failure stops the whole batch per
                // spec 06 (fail-fast on drift). Surface it as a generic
                // Vectorizer error on the first event of that collection.
                let victim_event = events
                    .iter()
                    .find(|e| {
                        collection_for(
                            &e.kind,
                            &self.config.collection_prefix,
                            e.context_repo.as_deref(),
                        ) == *name
                    })
                    .map(|e| e.event_id.clone())
                    .unwrap_or_else(|| "unknown".into());
                report.errors.push(EmbedError::Vectorizer {
                    event_id: victim_event,
                    detail: err.to_string(),
                });
                self.metrics
                    .incr_vectorizer_error(VectorizerErrorKind::from(&err));
                report.latency_ms = elapsed_ms(start);
                return Ok(report);
            }
            ensured.insert(name.clone());
        }

        // 4. Pre-check + upsert per collection.
        let upsert_batch = self.config.upsert_batch.max(1);
        for (collection, chunks) in by_collection {
            // Pre-filter by `dedup_key` — the list-view scan knows which
            // client-side keys already have a stored vector, and the
            // orchestrator skips those so re-runs don't duplicate work.
            let keys: Vec<String> = chunks.iter().map(|c| c.dedup_key.clone()).collect();
            let existing = match self.client.exists_by_dedup_key(&collection, &keys).await {
                Ok(set) => set,
                Err(err) => {
                    // Pre-check failures are non-fatal — log, count, and
                    // continue; the upsert itself is idempotent at the
                    // `dedup_key` level once the pre-check runs again on a
                    // later replay.
                    tracing::debug!(
                        %collection, error = %err, "exists pre-check failed; continuing"
                    );
                    self.metrics
                        .incr_vectorizer_error(VectorizerErrorKind::from(&err));
                    BTreeSet::new()
                }
            };
            let filtered: Vec<Chunk> = chunks
                .into_iter()
                .filter(|c| !existing.contains(&c.dedup_key))
                .collect();
            let deduped_precheck = (keys.len() - filtered.len()) as u32;
            if deduped_precheck > 0 {
                self.metrics.incr_dedup_hits(deduped_precheck as u64);
            }
            report.chunks_deduped = report.chunks_deduped.saturating_add(deduped_precheck);

            // Upsert in batches of `upsert_batch`.
            for sub in filtered.chunks(upsert_batch) {
                let batch_len = sub.len() as u32;
                self.metrics.observe_upsert_batch(batch_len);
                let call_start = Instant::now();
                let res = self.client.upsert_chunks(&collection, sub).await;
                let elapsed = elapsed_ms(call_start);
                self.metrics.observe_upsert_latency(elapsed);
                match res {
                    Ok(rep) => {
                        report.chunks_written = report.chunks_written.saturating_add(rep.written);
                        report.chunks_deduped = report.chunks_deduped.saturating_add(rep.deduped);
                        *report.by_collection.entry(collection.clone()).or_insert(0) += rep.written;
                        report.new_records.extend(rep.new_entries);
                    }
                    Err(err) => {
                        self.metrics
                            .incr_vectorizer_error(VectorizerErrorKind::from(&err));
                        // Attribute the failure to the first event of this
                        // collection.
                        let victim_event = events
                            .iter()
                            .find(|e| {
                                collection_for(
                                    &e.kind,
                                    &self.config.collection_prefix,
                                    e.context_repo.as_deref(),
                                ) == collection
                            })
                            .map(|e| e.event_id.clone())
                            .unwrap_or_else(|| "unknown".into());
                        report.errors.push(EmbedError::Vectorizer {
                            event_id: victim_event,
                            detail: err.to_string(),
                        });
                    }
                }
            }
        }

        report.latency_ms = elapsed_ms(start);
        Ok(report)
    }
}

fn elapsed_ms(start: Instant) -> u32 {
    start.elapsed().as_millis().min(u32::MAX as u128) as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classifier::{ClassifierSource, PiiRisk, Severity};

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
            sensitivity: Default::default(),
            source: ClassifierSource::StaticFallback,
            prompt_version: "v1".into(),
            model: "static-v1".into(),
            latency_ms: 0,
            tokens_in: 0,
            tokens_out: 0,
        }
    }

    fn event_with_path(path: Option<&str>, payload: Value) -> EnrichedEvent {
        EnrichedEvent {
            event_id: "01EVT".into(),
            kind: Kind::Artifact,
            content_hash: "h".into(),
            redacted_payload: payload,
            classifier: classifier("01EVT"),
            context_repo: Some("Cortex".into()),
            context_path: path.map(String::from),
            parent_event_id: None,
            session_id: None,
            occurred_at_ms: 0,
            class_level: None,
            class_compartments: None,
        }
    }

    #[test]
    fn embed_error_event_id_for_each_variant() {
        let e1 = EmbedError::OversizeWithoutSummary {
            event_id: "x".into(),
        };
        let e2 = EmbedError::Vectorizer {
            event_id: "y".into(),
            detail: "err".into(),
        };
        let e3 = EmbedError::Chunker {
            event_id: "z".into(),
            detail: "err".into(),
        };
        assert_eq!(e1.event_id(), "x");
        assert_eq!(e2.event_id(), "y");
        assert_eq!(e3.event_id(), "z");
    }

    #[test]
    fn embed_error_from_tuple_attaches_event_id() {
        let v_err = VectorizerClientError::Transport("boom".into());
        let e: EmbedError = ("evt-1", v_err).into();
        match e {
            EmbedError::Vectorizer { event_id, detail } => {
                assert_eq!(event_id, "evt-1");
                assert!(detail.contains("boom"));
            }
            _ => panic!("expected Vectorizer variant"),
        }
    }

    #[test]
    fn embed_error_serde_round_trips() {
        let e = EmbedError::OversizeWithoutSummary {
            event_id: "42".into(),
        };
        let json = serde_json::to_string(&e).unwrap();
        // The internally-tagged "cause" + snake-case rename apply.
        assert!(json.contains("\"cause\":\"oversize_without_summary\""));
        let back: EmbedError = serde_json::from_str(&json).unwrap();
        assert_eq!(back, e);
    }

    #[test]
    fn embed_error_display_matches_thiserror_template() {
        let e = EmbedError::Chunker {
            event_id: "abc".into(),
            detail: "parse-fail".into(),
        };
        let s = e.to_string();
        assert!(s.contains("abc") && s.contains("parse-fail"));
    }

    // ---- chunker_for routing ----

    use crate::embedder::vectorizer_client::MemoryVectorizerClient;

    fn embedder() -> VectorizerEmbedder {
        let cfg = EmbedderConfig::default();
        let client = Arc::new(MemoryVectorizerClient::default());
        VectorizerEmbedder::new(cfg, client)
    }

    // Verify chunker_for branches by observing the chunk-output
    // shape each picker produces. Code chunker decorates with
    // `language` metadata; doc chunker stamps source=Doc; fallback
    // stamps source=Sliding. Pointer equality is unreliable here
    // because Rust may collapse identical zero-sized type refs to
    // the same address.

    fn output_source_for(e: &VectorizerEmbedder, evt: &EnrichedEvent) -> ChunkSource {
        let chunker = e.chunker_for(evt);
        let chunks = chunker.chunk(evt).expect("chunker did not error");
        chunks
            .first()
            .map(|c| c.metadata.source)
            .unwrap_or(ChunkSource::FallbackWindow)
    }

    #[test]
    fn chunker_for_picks_doc_for_markdown_extension() {
        let e = embedder();
        let evt = event_with_path(Some("README.md"), serde_json::json!({"text": "# hi"}));
        assert_eq!(output_source_for(&e, &evt), ChunkSource::Doc);
    }

    #[test]
    fn chunker_for_picks_code_for_known_extension() {
        let e = embedder();
        let evt = event_with_path(
            Some("src/main.rs"),
            serde_json::json!({"text": "fn main(){}"}),
        );
        assert_eq!(output_source_for(&e, &evt), ChunkSource::Code);
    }

    #[test]
    fn chunker_for_picks_doc_when_text_starts_with_heading() {
        let e = embedder();
        let evt = event_with_path(None, serde_json::json!({"text": "# Title\n\nbody"}));
        assert_eq!(output_source_for(&e, &evt), ChunkSource::Doc);
    }

    #[test]
    fn chunker_for_falls_back_for_unknown_path() {
        let e = embedder();
        let evt = event_with_path(
            Some("data.bin.unknown"),
            serde_json::json!({"text": "raw bytes here"}),
        );
        assert_eq!(output_source_for(&e, &evt), ChunkSource::FallbackWindow);
    }

    #[test]
    fn with_schema_replaces_default() {
        let cfg = EmbedderConfig::default();
        let client = Arc::new(MemoryVectorizerClient::default());
        let custom = CollectionSchema {
            dim: 1024,
            ..CollectionSchema::default()
        };
        let e = VectorizerEmbedder::new(cfg, client).with_schema(custom.clone());
        assert_eq!(e.collection_schema.dim, 1024);
    }

    #[test]
    fn with_metrics_uses_provided_registry() {
        let cfg = EmbedderConfig::default();
        let client = Arc::new(MemoryVectorizerClient::default());
        let m = Arc::new(Metrics::new());
        let e = VectorizerEmbedder::with_metrics(cfg, client, m.clone());
        assert!(Arc::ptr_eq(&e.metrics, &m));
    }
}
