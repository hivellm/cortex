//! Phase13b §3.1 — `BootstrapProducer` wraps the existing
//! per-repo runner ([`run_repo_with_dedup`]) behind the
//! [`EnvelopeProducer`] trait.
//!
//! The wrapper is the minimal correct migration ADR-010 calls for
//! at this phase: the trait contract is satisfied at the per-run
//! granularity (one `producer_checkpoints` row per `produce`
//! invocation, carrying the final `(repo_id, last_file)` cursor as
//! `(scope, last_event_id)`). The runner's internal per-file
//! cursor (`checkpoint.repo_mut(&repo_id).last_file`) keeps working
//! in parallel — Phase 14 retires the legacy JSON state file and
//! promotes per-emit checkpointing into `producer_checkpoints`.
//!
//! What this gives us today:
//!
//! - One operator-queryable surface: `SELECT * FROM
//!   producer_checkpoints WHERE producer_name = 'bootstrap'`
//!   answers "what repos has bootstrap walked, and when?".
//! - A path to true kill-resume: any future bootstrap caller can
//!   read `latest_producer_checkpoint("bootstrap", repo_id)` on
//!   entry and skip to the recorded cursor instead of restarting
//!   from the JSON state singleton.
//! - Adapter onboarding: phase16a–phase16d implement the same
//!   trait and get the same dashboard / audit surface for free.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use chrono::Utc;
use cortex_workers::producer::{EnvelopeProducer, ProducerCheckpoint, ProducerCtx, ProducerReport};

use super::checkpoint::Checkpoint;
use super::config::CortexSection;
use super::metrics::Metrics;
use super::publisher::Publisher;
use super::runner::{run_repo_with_dedup, DedupStore, RepoRunReport, RunnerConfig};

/// Canonical producer name. Matches the
/// `producer_checkpoints.producer_name` column.
pub const BOOTSTRAP_PRODUCER_NAME: &str = "bootstrap";

/// Wraps one configured bootstrap run behind the
/// [`EnvelopeProducer`] trait. Carries the closure inputs the
/// existing runner needs; `produce` delegates to
/// [`run_repo_with_dedup`] and stamps the trait-level checkpoint
/// at the end.
///
/// One `BootstrapProducer` instance corresponds to one
/// `(repo_root, runner_cfg)` pair — i.e. one repo's walk. Callers
/// that bootstrap multiple repos build one producer per repo and
/// run them in declaration order, just like the legacy
/// orchestrator.
pub struct BootstrapProducer {
    repo_root: PathBuf,
    runner_cfg: RunnerConfig,
    repo_cfg: CortexSection,
    publisher: Arc<dyn Publisher>,
    metrics: Arc<Metrics>,
    dedup: Option<DedupStore>,
}

impl BootstrapProducer {
    /// Build a producer over the supplied inputs. The legacy
    /// `last_file` / `last_git_ref` resume parameters are read
    /// from the `producer_checkpoints` table inside `produce`
    /// instead.
    pub fn new(
        repo_root: PathBuf,
        runner_cfg: RunnerConfig,
        repo_cfg: CortexSection,
        publisher: Arc<dyn Publisher>,
        metrics: Arc<Metrics>,
        dedup: Option<DedupStore>,
    ) -> Self {
        Self {
            repo_root,
            runner_cfg,
            repo_cfg,
            publisher,
            metrics,
            dedup,
        }
    }

    /// Stable scope string for the trait's checkpoint table:
    /// `runner_cfg.repo_id` (canonicalised lowercase) matches what
    /// the runner stamps into envelopes.
    pub fn scope(&self) -> String {
        self.runner_cfg.repo_id.to_lowercase()
    }
}

#[async_trait]
impl EnvelopeProducer for BootstrapProducer {
    fn name(&self) -> &'static str {
        BOOTSTRAP_PRODUCER_NAME
    }

    async fn produce(&self, ctx: &ProducerCtx) -> Result<ProducerReport> {
        // Resume cursor (best-effort). The legacy runner accepts
        // `last_file` directly; the producer-trait checkpoint stores
        // the same path string under `last_event_id` per the
        // wrapper's encoding contract.
        let scope = self.scope();
        let last_file: Option<String> = {
            let store = ctx.metadata.lock().await;
            store
                .latest_producer_checkpoint(BOOTSTRAP_PRODUCER_NAME, &scope)?
                .map(|row| row.last_event_id)
                .filter(|s| !s.is_empty())
        };

        // Drive the legacy runner. It owns the file walk + git
        // walk + per-file emission, all of which already advance
        // the legacy `Checkpoint::last_file` cursor file-by-file.
        let mut legacy_checkpoint = Checkpoint::new(ctx.now.to_rfc3339());
        let report: RepoRunReport = run_repo_with_dedup(
            &self.repo_root,
            &self.runner_cfg,
            &self.repo_cfg,
            self.publisher.clone(),
            self.metrics.clone(),
            &mut legacy_checkpoint,
            last_file.clone(),
            None,
            self.dedup.clone(),
        )
        .await?;

        // Persist the trait-level checkpoint. We use the final
        // `last_file` from the legacy checkpoint as the cursor
        // token; resume reads it back through `latest_…` above.
        let cursor = legacy_checkpoint
            .repo_mut(&self.runner_cfg.repo_id)
            .last_file
            .clone()
            .unwrap_or_default();
        let accumulated_at = Utc::now();
        {
            let store = ctx.metadata.lock().await;
            store.record_producer_checkpoint(
                BOOTSTRAP_PRODUCER_NAME,
                &scope,
                &cursor,
                ctx.now,
                accumulated_at,
            )?;
        }

        Ok(ProducerReport {
            producer_name: BOOTSTRAP_PRODUCER_NAME.to_string(),
            envelopes_emitted: report.events_published,
            batches_emitted: 1,
            last_event_id: cursor,
            last_occurred_at: Some(ctx.now),
        })
    }

    async fn resume_from(
        &self,
        ctx: &ProducerCtx,
        scope: &str,
    ) -> Result<Option<ProducerCheckpoint>> {
        let store = ctx.metadata.lock().await;
        let row = store.latest_producer_checkpoint(BOOTSTRAP_PRODUCER_NAME, scope)?;
        Ok(row.map(ProducerCheckpoint::from_row))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bootstrap::publisher::Publisher;
    use async_trait::async_trait;
    use cortex_storage::MetadataStore;
    use std::sync::Mutex as StdMutex;
    use tempfile::TempDir;
    use tokio::sync::Mutex;

    /// Recording publisher — captures every published payload so
    /// the unit test can assert per-envelope counts. Mirrors the
    /// `Publisher` shape already used in the bootstrap runner's
    /// own tests.
    #[derive(Default)]
    struct RecordingPublisher {
        events: StdMutex<Vec<(String, String)>>,
    }

    #[async_trait]
    impl Publisher for RecordingPublisher {
        async fn publish_one(
            &self,
            room: &str,
            event: &crate::bootstrap::emitter::BootstrapEvent,
        ) -> Result<()> {
            self.events
                .lock()
                .expect("events lock")
                .push((room.to_string(), event.kind.clone()));
            Ok(())
        }
    }

    fn make_ctx() -> (ProducerCtx, Arc<Mutex<MetadataStore>>) {
        let store = MetadataStore::open_in_memory().expect("metadata store");
        let handle = Arc::new(Mutex::new(store));
        let ctx = ProducerCtx::new(handle.clone(), "cortex.producer.bootstrap");
        (ctx, handle)
    }

    fn synthetic_repo() -> (TempDir, PathBuf) {
        let dir = TempDir::new().expect("temp repo");
        let root = dir.path().to_path_buf();
        std::fs::write(root.join("README.md"), "# tiny repo\n").unwrap();
        std::fs::write(
            root.join("notes.md"),
            "phase13b smoke test for BootstrapProducer.\n",
        )
        .unwrap();
        (dir, root)
    }

    #[tokio::test]
    async fn bootstrap_producer_writes_one_checkpoint_per_run() {
        let (_dir, root) = synthetic_repo();
        let runner_cfg = RunnerConfig {
            repo_id: "test_repo".into(),
            stream: super::super::emitter::BOOTSTRAP_STREAM.into(),
            since: None,
            dry_run: true, // No publish path needed.
            kind_filter: Vec::new(),
        };
        let repo_cfg = CortexSection::default();
        let publisher: Arc<dyn Publisher> = Arc::new(RecordingPublisher::default());
        let metrics = Arc::new(Metrics::default());
        let producer = BootstrapProducer::new(root, runner_cfg, repo_cfg, publisher, metrics, None);
        let (ctx, handle) = make_ctx();

        let report = producer.produce(&ctx).await.unwrap();
        assert_eq!(report.producer_name, BOOTSTRAP_PRODUCER_NAME);
        assert_eq!(report.batches_emitted, 1);

        let rows = handle
            .lock()
            .await
            .list_producer_checkpoints_for(BOOTSTRAP_PRODUCER_NAME, 50)
            .unwrap();
        assert_eq!(rows.len(), 1, "exactly one row per run");
        assert_eq!(rows[0].scope, "test_repo");
    }

    #[tokio::test]
    async fn bootstrap_producer_resume_from_returns_latest_row() {
        let (_dir, root) = synthetic_repo();
        let runner_cfg = RunnerConfig {
            repo_id: "test_repo".into(),
            stream: super::super::emitter::BOOTSTRAP_STREAM.into(),
            since: None,
            dry_run: true,
            kind_filter: Vec::new(),
        };
        let repo_cfg = CortexSection::default();
        let publisher: Arc<dyn Publisher> = Arc::new(RecordingPublisher::default());
        let metrics = Arc::new(Metrics::default());
        let producer = BootstrapProducer::new(root, runner_cfg, repo_cfg, publisher, metrics, None);
        let (ctx, _handle) = make_ctx();

        // First run writes a checkpoint.
        let _ = producer.produce(&ctx).await.unwrap();

        // Resume reads it back.
        let resume = producer.resume_from(&ctx, "test_repo").await.unwrap();
        assert!(resume.is_some(), "resume_from returns Some after a run");
    }
}
