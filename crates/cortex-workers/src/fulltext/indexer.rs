//! `FulltextIndexer` trait and the Meili-backed default implementation.
//!
//! Mirrors the spec-08 §Inputs/Outputs contract:
//!
//! ```ignore
//! #[async_trait]
//! pub trait FulltextIndexer {
//!     async fn index_batch(&self, events: &[EnrichedEvent]) -> Result<IndexReport>;
//! }
//! ```
//!
//! The Meili-backed impl groups events by their target index, calls
//! the per-kind builder for each one, and posts each group to Meili
//! through [`MeiliClient::upsert_documents`]. The `await_task` flag
//! controls whether the indexer waits on the Meili task to complete
//! (bootstrap fail-fast) or fires-and-forgets (live traffic).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::builders::{build_doc, BuildOutcome};
use super::config::FulltextConfig;
use super::document::Document;
use super::meili_client::{MeiliClient, MeiliError};
use super::metrics::Metrics;
use super::routing::{index_for_event, index_for_event_global, is_low_signal_tool_call_event};
use super::settings::settings_v1_json;
use crate::embedder::EnrichedEvent;

/// Per-batch report returned by [`FulltextIndexer::index_batch`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IndexReport {
    /// Total documents handed to Meilisearch this batch.
    pub documents_upserted: u32,
    /// Documents the builder declined (empty body and no summary).
    pub documents_skipped: u32,
    /// Documents whose body was truncated to honour the size cap.
    pub documents_truncated: u32,
    /// Per-index counts.
    pub by_index: BTreeMap<String, u32>,
    /// Wall-clock latency of the batch in milliseconds.
    pub latency_ms: u32,
}

/// Indexer trait — exactly the signature in spec 08 §Inputs/Outputs.
#[async_trait]
pub trait FulltextIndexer: Send + Sync {
    /// Translate `events` into Meilisearch documents and upsert them.
    async fn index_batch(&self, events: &[EnrichedEvent]) -> Result<IndexReport, MeiliError>;
}

/// Meili-backed [`FulltextIndexer`].
#[derive(Clone)]
pub struct MeiliFulltextIndexer {
    config: FulltextConfig,
    client: Arc<dyn MeiliClient>,
    metrics: Arc<Metrics>,
    /// Set of index uids whose spec-08 settings have already been
    /// applied during this process's lifetime. The startup pass in
    /// `cortex-fulltext-worker/main.rs` seeds the legacy
    /// `cortex-{family}` set; per-project indexes
    /// (`cortex-{slug}-{family}`) are materialised lazily on first
    /// upsert, so we ensure their settings here once and skip the
    /// PATCH on every subsequent batch. Wrapped in a `Mutex` so the
    /// worker pool can clone the indexer without sharing state via
    /// interior `RefCell`-flavoured maps.
    ensured: Arc<Mutex<BTreeSet<String>>>,
}

impl MeiliFulltextIndexer {
    /// Construct a new indexer.
    pub fn new(
        config: FulltextConfig,
        client: Arc<dyn MeiliClient>,
        metrics: Arc<Metrics>,
    ) -> Self {
        Self {
            config,
            client,
            metrics,
            ensured: Arc::new(Mutex::new(BTreeSet::new())),
        }
    }

    /// Construct an indexer that already considers `seeded` indexes
    /// settings-applied — used by the worker bootstrap which calls
    /// `ensure_index` for every legacy `cortex-{family}` uid before
    /// the consumer pool starts.
    pub fn with_ensured(
        config: FulltextConfig,
        client: Arc<dyn MeiliClient>,
        metrics: Arc<Metrics>,
        seeded: impl IntoIterator<Item = String>,
    ) -> Self {
        Self {
            config,
            client,
            metrics,
            ensured: Arc::new(Mutex::new(seeded.into_iter().collect())),
        }
    }

    /// Borrow the runtime configuration.
    pub fn config(&self) -> &FulltextConfig {
        &self.config
    }

    /// Borrow the metrics registry.
    pub fn metrics(&self) -> &Arc<Metrics> {
        &self.metrics
    }

    /// Apply spec-08 settings to `index` if this indexer hasn't done
    /// so yet. Subsequent batches skip the PATCH so concurrent
    /// workers don't fight Meili's settings task queue. A failure
    /// here is treated like any other Meili error — the caller's
    /// retry policy decides whether to retry the whole batch.
    async fn ensure_settings(&self, index: &str) -> Result<(), MeiliError> {
        {
            let guard = self.ensured.lock().expect("ensured mutex");
            if guard.contains(index) {
                return Ok(());
            }
        }
        // Settings are baked into the binary via include_str! — a
        // serialisation failure here is a programming error, not a
        // runtime failure, so map to MeiliError::Http for symmetry
        // with the rest of the indexer.
        let settings: Value =
            settings_v1_json().map_err(|e| MeiliError::Http(format!("settings_v1_json: {e}")))?;
        self.client.ensure_index(index, &settings).await?;
        self.metrics.incr_settings_bump();
        if let Ok(mut guard) = self.ensured.lock() {
            guard.insert(index.to_string());
        }
        Ok(())
    }
}

#[async_trait]
impl FulltextIndexer for MeiliFulltextIndexer {
    async fn index_batch(&self, events: &[EnrichedEvent]) -> Result<IndexReport, MeiliError> {
        let start = Instant::now();
        let mut by_index: BTreeMap<String, Vec<Document>> = BTreeMap::new();
        let mut skipped: u32 = 0;
        let mut truncated: u32 = 0;

        // 1. Build per-event documents and group by target index.
        for event in events {
            // 2026-05-20 — drop low-signal tool_call envelopes
            // (Bash / Read / Grep / Edit / Write / Glob / …) before
            // they reach the builder. A single Claude Code session
            // emits hundreds of these; without the filter the per-
            // repo `cortex-{slug}-code` index ends up dominated by
            // shell payloads that out-rank real source documents in
            // BM25 matches. Higher-signal tool calls (Task /
            // WebFetch / TodoWrite / non-Anthropic MCP) still index
            // because `is_low_signal_tool_call_event` only matches
            // names in the deny list. Operators can re-enable
            // indexing via `CORTEX_INDEX_LOW_SIGNAL_TOOL_CALLS=1`.
            if is_low_signal_tool_call_event(event) {
                skipped = skipped.saturating_add(1);
                self.metrics.incr_skipped_empty();
                continue;
            }
            let outcome = build_doc(
                event,
                /* bootstrap */ false,
                self.config.max_body_bytes,
            );
            match outcome {
                BuildOutcome::Skipped => {
                    skipped = skipped.saturating_add(1);
                    self.metrics.incr_skipped_empty();
                }
                BuildOutcome::Ready(doc) => {
                    if doc.truncated {
                        truncated = truncated.saturating_add(1);
                        self.metrics.incr_truncated();
                    }
                    let index_name = index_for_event(&self.config.index_prefix, event);
                    self.metrics.incr_routed(&index_name);
                    // Phase11k §2.2 — governance kinds dual-write to
                    // the global `cortex_decisions` / `cortex_laws`
                    // index so `decision_lookup` / `law_check` can
                    // answer without `scope.repo`. The per-repo index
                    // remains authoritative for scoped reads; the
                    // global one is the cross-repo overlay. The
                    // document instance is shared by `Arc`-cloning the
                    // boxed payload — both lanes write the same row,
                    // so dedupe by `doc.id` is automatic.
                    let global_target =
                        index_for_event_global(event.kind).map(|name| name.to_string());
                    let doc_unboxed = *doc;
                    if let Some(global) = global_target {
                        self.metrics.incr_routed(&global);
                        by_index
                            .entry(global)
                            .or_default()
                            .push(doc_unboxed.clone());
                    }
                    by_index.entry(index_name).or_default().push(doc_unboxed);
                }
            }
        }

        // 2. Per-index upsert with the configured batch size.
        let mut total_upserted: u32 = 0;
        let mut by_index_counts: BTreeMap<String, u32> = BTreeMap::new();
        let batch_size = self.config.upsert_batch.max(1);
        for (index, docs) in by_index {
            // Lazy spec-08 settings on first encounter. Per-project
            // uids (cortex-{slug}-{family}) are materialised on the
            // first upsert; without this they'd auto-create with an
            // empty settings doc and `sort: ["ts:desc"]` / `filter`
            // queries from the dashboard would fail with
            // "Attribute ts is not sortable". Settings are
            // idempotent — if another worker raced us, Meili swallows
            // the no-op PATCH.
            self.ensure_settings(&index).await?;
            let mut count_for_index: u32 = 0;
            for chunk in docs.chunks(batch_size) {
                let upsert_started = Instant::now();
                let report = self.client.upsert_documents(&index, chunk).await?;
                let latency_ms =
                    u32::try_from(upsert_started.elapsed().as_millis()).unwrap_or(u32::MAX);
                self.metrics.observe_upsert_latency(&index, latency_ms);
                self.metrics.observe_batch_size(report.documents_upserted);
                self.metrics
                    .incr_documents(&index, u64::from(report.documents_upserted));
                count_for_index = count_for_index.saturating_add(report.documents_upserted);
                total_upserted = total_upserted.saturating_add(report.documents_upserted);

                if self.config.await_task {
                    self.client
                        .wait_task(report.task_uid, Duration::from_secs(60))
                        .await?;
                }
            }
            by_index_counts.insert(index, count_for_index);
        }

        let latency_ms = u32::try_from(start.elapsed().as_millis()).unwrap_or(u32::MAX);
        Ok(IndexReport {
            documents_upserted: total_upserted,
            documents_skipped: skipped,
            documents_truncated: truncated,
            by_index: by_index_counts,
            latency_ms,
        })
    }
}
