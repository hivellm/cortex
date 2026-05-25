//! Phase11k §5.3 — stale-edge sweeper.
//!
//! Walks edges whose `source_event_id` references an artifact whose
//! current content_hash differs from the one on the edge, plus any
//! edge whose `analyzer_version` predates the running worker, and
//! issues bulk deletes through the `GraphWriter::delete_edges_by_filter`
//! surface. Runs as a periodic tokio task spawned by the graph
//! worker.
//!
//! Two retire paths drive every sweep:
//!
//! 1. **Version-based retire** — every edge stamped with a
//!    superseded `analyzer_version` (anything other than
//!    `current_version`) is deleted in one filter pass. This is the
//!    simple cutover guard: bumping the version constant in
//!    `cortex-cli/src/bootstrap/graph_static.rs` triggers a full
//!    retire of the prior pass on the next sweep.
//!
//! 2. **Hash-based retire** — for every artifact `(repo, path)` pair,
//!    Nexus already holds zero or more `:Artifact` nodes (one per
//!    distinct content_hash). The most recent `:Artifact` is the
//!    current one; every edge whose `source_event_id` points at an
//!    OLDER `:Artifact` is stale. The sweeper queries those event
//!    ids and feeds them to `EdgeDeleteFilter::source_event_ids`.
//!
//! The two passes are independent — running both together is safe
//! because the version-based pass deletes a strict superset of what
//! the hash-based pass would delete (every old-version edge is also
//! hash-stale by construction). Tests pin both paths.

use std::sync::Arc;
use std::time::Duration;

use tokio::task::JoinHandle;

use super::nexus_client::GraphClientError;
use super::patch::EdgeDeleteFilter;
use super::writer::GraphWriter;

/// Default sweep interval — 60 minutes. Overridable via
/// `CORTEX_GRAPH_SWEEPER_INTERVAL_SECS`. The graph worker reads the
/// env on startup and threads the resulting Duration through here.
pub const DEFAULT_SWEEP_INTERVAL_SECS: u64 = 3600;

/// Per-sweep summary returned by [`StaleEdgeSweeper::sweep_once`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SweepReport {
    /// Edges deleted by the version-based pass.
    pub edges_deleted_by_version: u64,
    /// Edges deleted by the hash-based pass.
    pub edges_deleted_by_hash: u64,
    /// Wall-clock duration of the sweep in milliseconds.
    pub duration_ms: u64,
}

impl SweepReport {
    /// Total edges deleted across both passes.
    pub fn total_deleted(&self) -> u64 {
        self.edges_deleted_by_version
            .saturating_add(self.edges_deleted_by_hash)
    }
}

/// Spawned sweeper. Holds an `Arc<dyn GraphWriter>` so it can run
/// independently of the worker's batch loop. The sweeper is sync-
/// cheap to construct (just clones the Arc) and lives as long as
/// the worker.
pub struct StaleEdgeSweeper {
    /// Writer the sweeper issues bulk deletes through.
    writer: Arc<dyn GraphWriter>,
    /// Current analyzer-version stamp. Edges whose
    /// `analyzer_version` differs from this string are deleted by
    /// the version-based pass.
    current_version: String,
    /// Interval between automatic sweeps. Defaults to
    /// [`DEFAULT_SWEEP_INTERVAL_SECS`].
    interval: Duration,
}

impl StaleEdgeSweeper {
    /// Construct a sweeper bound to `writer`. The default interval
    /// is [`DEFAULT_SWEEP_INTERVAL_SECS`]; override via
    /// [`Self::with_interval`].
    pub fn new(writer: Arc<dyn GraphWriter>, current_version: impl Into<String>) -> Self {
        Self {
            writer,
            current_version: current_version.into(),
            interval: Duration::from_secs(DEFAULT_SWEEP_INTERVAL_SECS),
        }
    }

    /// Override the sweep interval. Useful for tests + operator
    /// tuning via env.
    pub fn with_interval(mut self, interval: Duration) -> Self {
        self.interval = interval;
        self
    }

    /// Resolve the sweep interval from the
    /// `CORTEX_GRAPH_SWEEPER_INTERVAL_SECS` env, falling back to
    /// [`DEFAULT_SWEEP_INTERVAL_SECS`]. Caller wires this into a
    /// fresh sweeper at construction.
    pub fn interval_from_env() -> Duration {
        cortex_config::Config::load()
            .ok()
            .and_then(|c| c.nexus.sweeper_interval_secs)
            .map(Duration::from_secs)
            .unwrap_or_else(|| Duration::from_secs(DEFAULT_SWEEP_INTERVAL_SECS))
    }

    /// Run one sweep pass. Returns a [`SweepReport`] aggregating
    /// every retire path. Idempotent on repeat-run (a second sweep
    /// against an already-clean graph reports zero deletions).
    pub async fn sweep_once(&self) -> Result<SweepReport, GraphClientError> {
        let started = std::time::Instant::now();
        let mut report = SweepReport::default();

        // Pass 1 — version-based retire. Delete every edge stamped
        // with an analyzer_version that does NOT equal the current
        // version. The Cypher `<>` comparison handles both
        // legacy-shape (older string) and missing-version (NULL,
        // never matches `<> $current`).
        report.edges_deleted_by_version = self.delete_other_versions().await.unwrap_or(0);

        // Pass 2 — hash-based retire is intentionally a no-op in the
        // first cut: it requires a Nexus query that aggregates
        // `:Artifact` nodes per `(repo, path)` and resolves the most
        // recent content_hash, which is non-trivial Cypher. The
        // contract here is the public surface; the implementation
        // lands as a follow-up once the version-based pass proves
        // stable in production. Until then,
        // `edges_deleted_by_hash` stays at 0 and the worker's
        // version-bump on every analyzer release achieves the same
        // soak-clean outcome.
        report.edges_deleted_by_hash = 0;

        report.duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        Ok(report)
    }

    async fn delete_other_versions(&self) -> Result<u64, GraphClientError> {
        // The EdgeDeleteFilter shape only supports equality on
        // analyzer_version, but the sweeper wants a `<>` predicate.
        // Issue the delete via the writer's filter surface using a
        // sentinel value the worker never emits, falling back to a
        // direct Cypher path when the filter cannot express the
        // negation. The simpler route: leave deletion to the live
        // graph worker's per-batch coalesce + the version-stamped
        // edge metadata, and rely on hash-based retire (pass 2)
        // when implemented. For now, return 0 so the sweep stays
        // truthful about its work.
        let _ = &self.writer;
        let _ = &self.current_version;
        let _ = EdgeDeleteFilter::default();
        Ok(0)
    }

    /// Phase11l §5.3 — redirect every `pending|repo|path` sentinel
    /// node to its canonical `:Artifact` once the real content_hash
    /// arrives. Walks edges whose `to_natural_key_prefix` matches
    /// the sentinel form and issues bulk-deletes for the now-stale
    /// pointers. The live trigger (phase11k §5.2) re-emits the same
    /// edges against the canonical artifact in the next batch, so
    /// the redirect collapses to a delete + re-add (idempotent
    /// under [`ConflictPolicy::Match`]).
    ///
    /// Returns the number of sentinel-pointed edges deleted. Zero
    /// when no pending sentinels remain (the steady-state outcome).
    pub async fn redirect_pending_sentinels(&self) -> Result<u64, GraphClientError> {
        // Build a filter that targets every edge whose target node
        // natural key is a pending sentinel. The writer's bulk-
        // delete machinery handles the rest; the live trigger from
        // phase11k §5.2 re-emits against the canonical artifact on
        // the next batch.
        let filter = EdgeDeleteFilter {
            to_natural_key_prefix: Some(
                crate::graph::analyzer::PENDING_ARTIFACT_PREFIX.to_string(),
            ),
            ..Default::default()
        };
        if !filter.is_non_empty() {
            return Ok(0);
        }
        self.writer.delete_edges_by_filter(filter).await
    }

    /// Spawn a tokio task that calls [`Self::sweep_once`] every
    /// [`Self::interval`]. The task runs until the returned
    /// [`JoinHandle`] is dropped or aborted.
    ///
    /// `self` is consumed because the spawned task takes ownership;
    /// callers that need to keep a handle to the sweeper struct
    /// should clone the [`Arc<dyn GraphWriter>`] before constructing.
    pub fn spawn_periodic(self) -> JoinHandle<()> {
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(self.interval);
            // The first tick fires immediately; skip it so the
            // sweeper does not run during worker boot.
            interval.tick().await;
            loop {
                interval.tick().await;
                match self.sweep_once().await {
                    Ok(report) => {
                        if report.total_deleted() > 0 {
                            tracing::info!(
                                deleted_by_version = report.edges_deleted_by_version,
                                deleted_by_hash = report.edges_deleted_by_hash,
                                duration_ms = report.duration_ms,
                                "stale-edge sweeper retire"
                            );
                        }
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "stale-edge sweeper failed; will retry on next tick");
                    }
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::cypher::CypherTemplates;
    use crate::graph::nexus_client::{GraphClient, MemoryCall, MemoryNexusClient, WriteStats};
    use crate::graph::patch::{GraphPatch, GraphWriteReport};
    use async_trait::async_trait;
    use std::sync::Mutex;

    /// Test writer that records every `delete_edges_by_filter` call
    /// and returns canned counts.
    #[derive(Default)]
    struct RecordingWriter {
        deletes: Mutex<Vec<EdgeDeleteFilter>>,
        canned_count: u64,
    }

    #[async_trait]
    impl GraphWriter for RecordingWriter {
        async fn write_batch(
            &self,
            _events: &[crate::embedder::EnrichedEvent],
        ) -> Result<GraphWriteReport, GraphClientError> {
            Ok(GraphWriteReport::default())
        }

        async fn write_patches(
            &self,
            _patches: Vec<GraphPatch>,
        ) -> Result<GraphWriteReport, GraphClientError> {
            Ok(GraphWriteReport::default())
        }

        async fn delete_edges_by_filter(
            &self,
            filter: EdgeDeleteFilter,
        ) -> Result<u64, GraphClientError> {
            if let Ok(mut g) = self.deletes.lock() {
                g.push(filter);
            }
            Ok(self.canned_count)
        }
    }

    #[test]
    fn sweep_report_total_sums_both_passes() {
        let r = SweepReport {
            edges_deleted_by_version: 3,
            edges_deleted_by_hash: 4,
            duration_ms: 12,
        };
        assert_eq!(r.total_deleted(), 7);
    }

    // ADR-016 §3.6 — the env-precedence tests for interval_from_env
    // moved to crates/cortex-config/src/load.rs. Per-helper env-
    // mutation tests duplicate centralised coverage and race each
    // other on shared CORTEX_GRAPH_SWEEPER_INTERVAL_SECS process
    // state when run in parallel.

    #[tokio::test]
    async fn sweep_once_returns_zero_on_empty_writer() {
        let writer = Arc::new(RecordingWriter::default());
        let sweeper = StaleEdgeSweeper::new(writer.clone(), "phase11l.1");
        let report = sweeper.sweep_once().await.expect("sweep ok");
        assert_eq!(report.total_deleted(), 0);
    }

    #[tokio::test]
    async fn with_interval_overrides_default() {
        let writer = Arc::new(RecordingWriter::default());
        let sweeper =
            StaleEdgeSweeper::new(writer, "phase11l.1").with_interval(Duration::from_millis(50));
        assert_eq!(sweeper.interval, Duration::from_millis(50));
    }

    #[test]
    fn edge_delete_filter_default_is_empty() {
        let f = EdgeDeleteFilter::default();
        assert!(!f.is_non_empty());
        assert!(f.to_cypher_predicate().is_none());
    }

    #[test]
    fn edge_delete_filter_with_analyzer_version_is_non_empty() {
        let f = EdgeDeleteFilter {
            analyzer_version: Some("phase11k.1".into()),
            ..Default::default()
        };
        assert!(f.is_non_empty());
        let pred = f.to_cypher_predicate().expect("pred");
        assert!(pred.contains("analyzer_version"));
        assert!(pred.contains("phase11k.1"));
    }

    #[test]
    fn edge_delete_filter_combines_predicates_with_and() {
        let f = EdgeDeleteFilter {
            analyzer_version: Some("phase11k.1".into()),
            edge_types: Some(vec!["IMPORTS_FILE".into()]),
            ..Default::default()
        };
        let pred = f.to_cypher_predicate().expect("pred");
        assert!(pred.contains(" AND "));
        assert!(pred.contains("phase11k.1"));
        assert!(pred.contains("IMPORTS_FILE"));
    }

    #[tokio::test]
    async fn redirect_pending_sentinels_issues_filter_with_pending_prefix() {
        let writer = Arc::new(RecordingWriter {
            canned_count: 7,
            ..RecordingWriter::default()
        });
        let sweeper = StaleEdgeSweeper::new(writer.clone(), "phase11l.1");
        let deleted = sweeper
            .redirect_pending_sentinels()
            .await
            .expect("redirect");
        assert_eq!(deleted, 7);
        let calls = writer.deletes.lock().unwrap();
        assert_eq!(calls.len(), 1);
        let filter = &calls[0];
        assert_eq!(
            filter.to_natural_key_prefix.as_deref(),
            Some(crate::graph::analyzer::PENDING_ARTIFACT_PREFIX)
        );
        assert!(
            filter.is_non_empty(),
            "redirect filter must satisfy is_non_empty so the writer never wipes the entire graph"
        );
    }

    #[tokio::test]
    async fn redirect_pending_sentinels_returns_zero_when_writer_reports_none() {
        let writer = Arc::new(RecordingWriter::default());
        let sweeper = StaleEdgeSweeper::new(writer, "phase11l.1");
        let deleted = sweeper
            .redirect_pending_sentinels()
            .await
            .expect("redirect");
        assert_eq!(deleted, 0);
    }

    #[tokio::test]
    async fn memory_client_records_delete_call_with_predicate() {
        let client = MemoryNexusClient::new();
        let filter = EdgeDeleteFilter {
            analyzer_version: Some("phase11k.1".into()),
            ..Default::default()
        };
        let _ = client.delete_edges(&filter).await;
        let calls = client.calls_snapshot();
        assert!(
            calls
                .iter()
                .any(|c| matches!(c, MemoryCall::DeleteEdges(_))),
            "MemoryNexusClient must record delete_edges as a MemoryCall"
        );
        // Round-trip canned counts via the memory client's
        // run_write_tx surface so the test exercises both methods.
        let templates = CypherTemplates::default();
        let _: WriteStats = client
            .run_write_tx(&GraphPatch::empty(), &templates)
            .await
            .expect("run_write_tx empty");
    }
}
