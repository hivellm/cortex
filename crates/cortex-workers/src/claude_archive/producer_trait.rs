//! Phase13b §3.2 — `ClaudeArchiveProducer` wraps the claude-archive
//! walker behind the [`EnvelopeProducer`] trait.
//!
//! The wrapper carries a `WalkConfig`; `produce(ctx)` runs the
//! walker, groups the resulting entries by `project_dir`, and
//! writes one `producer_checkpoints` row per project carrying the
//! final session path as the cursor token. The existing
//! `claude_archive::checkpoint` file-store keeps tracking per-session
//! byte offsets in parallel; the trait surface adds the
//! cross-project audit the legacy file-store cannot answer
//! (`SELECT * FROM producer_checkpoints WHERE producer_name =
//! 'claude_archive'`).
//!
//! Scope policy: one scope per project directory
//! (`WalkEntry::project_dir`). A walk that visits ten projects
//! writes ten rows.

use std::collections::BTreeMap;

use async_trait::async_trait;
use chrono::Utc;

use crate::producer::{EnvelopeProducer, ProducerCheckpoint, ProducerCtx, ProducerReport};

use super::walker::{walk, WalkConfig, WalkEntry};

/// Canonical producer name.
pub const CLAUDE_ARCHIVE_PRODUCER_NAME: &str = "claude_archive";

/// Wraps the claude-archive walker behind the trait. Holds the
/// `WalkConfig`; `produce` is a pure function of that config and
/// the filesystem state at `ctx.now`.
pub struct ClaudeArchiveProducer {
    walk_config: WalkConfig,
}

impl ClaudeArchiveProducer {
    /// Build the producer over the supplied walker configuration.
    pub fn new(walk_config: WalkConfig) -> Self {
        Self { walk_config }
    }
}

#[async_trait]
impl EnvelopeProducer for ClaudeArchiveProducer {
    fn name(&self) -> &'static str {
        CLAUDE_ARCHIVE_PRODUCER_NAME
    }

    async fn produce(&self, ctx: &ProducerCtx) -> anyhow::Result<ProducerReport> {
        let walk_config = self.walk_config.clone();
        let entries: Vec<WalkEntry> =
            tokio::task::spawn_blocking(move || walk(&walk_config)).await?;

        // Group by project_dir; the last entry per project becomes
        // the cursor token. Empty project_dir (global sidecars) is
        // collapsed under a `__sidecars__` scope so it still gets a
        // checkpoint row.
        let mut by_project: BTreeMap<String, &WalkEntry> = BTreeMap::new();
        let mut total: u64 = 0;
        for entry in &entries {
            let scope_key = if entry.project_dir.is_empty() {
                "__sidecars__".to_string()
            } else {
                entry.project_dir.clone()
            };
            by_project.insert(scope_key, entry);
            total += 1;
        }

        // Write one checkpoint per project, offset by ms so the
        // composite PK stays unique under a pinned reference clock
        // (matches the producer mod's per-batch offset contract).
        let mut last_event_id = String::new();
        let mut batches = 0u64;
        {
            let store = ctx.metadata.lock().await;
            for (idx, (scope, entry)) in by_project.iter().enumerate() {
                let cursor = entry.path.to_string_lossy().into_owned();
                let accumulated_at = Utc::now() + chrono::Duration::microseconds(idx as i64);
                store.record_producer_checkpoint(
                    CLAUDE_ARCHIVE_PRODUCER_NAME,
                    scope,
                    &cursor,
                    ctx.now,
                    accumulated_at,
                )?;
                last_event_id = cursor;
                batches += 1;
            }
        }

        Ok(ProducerReport {
            producer_name: CLAUDE_ARCHIVE_PRODUCER_NAME.to_string(),
            envelopes_emitted: total,
            batches_emitted: batches,
            last_event_id,
            last_occurred_at: Some(ctx.now),
        })
    }

    async fn resume_from(
        &self,
        ctx: &ProducerCtx,
        scope: &str,
    ) -> anyhow::Result<Option<ProducerCheckpoint>> {
        let store = ctx.metadata.lock().await;
        let row = store.latest_producer_checkpoint(CLAUDE_ARCHIVE_PRODUCER_NAME, scope)?;
        Ok(row.map(ProducerCheckpoint::from_row))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cortex_storage::MetadataStore;
    use std::sync::Arc;
    use tempfile::TempDir;
    use tokio::sync::Mutex;

    fn make_ctx() -> (ProducerCtx, Arc<Mutex<MetadataStore>>) {
        let store = MetadataStore::open_in_memory().unwrap();
        let handle = Arc::new(Mutex::new(store));
        let ctx = ProducerCtx::new(handle.clone(), "cortex.producer.claude_archive");
        (ctx, handle)
    }

    fn synthetic_archive() -> (TempDir, WalkConfig) {
        let dir = TempDir::new().unwrap();
        let projects = dir.path().join("projects");
        std::fs::create_dir_all(&projects).unwrap();
        // Project A — one session file.
        let proj_a = projects.join("e--demo-a");
        std::fs::create_dir_all(&proj_a).unwrap();
        std::fs::write(proj_a.join("session-a.jsonl"), "{}\n").unwrap();
        // Project B — two session files.
        let proj_b = projects.join("e--demo-b");
        std::fs::create_dir_all(&proj_b).unwrap();
        std::fs::write(proj_b.join("session-b1.jsonl"), "{}\n").unwrap();
        std::fs::write(proj_b.join("session-b2.jsonl"), "{}\n").unwrap();
        let cfg = WalkConfig::projects_only(dir.path().to_path_buf());
        (dir, cfg)
    }

    #[tokio::test]
    async fn claude_archive_producer_writes_one_row_per_project() {
        let (_dir, walk_config) = synthetic_archive();
        let (ctx, handle) = make_ctx();
        let producer = ClaudeArchiveProducer::new(walk_config);
        let report = producer.produce(&ctx).await.unwrap();
        assert_eq!(report.producer_name, CLAUDE_ARCHIVE_PRODUCER_NAME);
        assert_eq!(report.batches_emitted, 2);
        let rows = handle
            .lock()
            .await
            .list_producer_checkpoints_for(CLAUDE_ARCHIVE_PRODUCER_NAME, 50)
            .unwrap();
        assert_eq!(rows.len(), 2);
        let scopes: Vec<String> = rows.iter().map(|r| r.scope.clone()).collect();
        assert!(scopes.iter().any(|s| s == "e--demo-a"));
        assert!(scopes.iter().any(|s| s == "e--demo-b"));
    }

    #[tokio::test]
    async fn claude_archive_resume_from_returns_latest_per_scope() {
        let (_dir, walk_config) = synthetic_archive();
        let (ctx, _handle) = make_ctx();
        let producer = ClaudeArchiveProducer::new(walk_config);
        let _ = producer.produce(&ctx).await.unwrap();
        let resume = producer.resume_from(&ctx, "e--demo-a").await.unwrap();
        assert!(resume.is_some());
    }

    #[tokio::test]
    async fn claude_archive_empty_walk_writes_no_rows() {
        let dir = TempDir::new().unwrap();
        let walk_config = WalkConfig::projects_only(dir.path().to_path_buf());
        let (ctx, handle) = make_ctx();
        let producer = ClaudeArchiveProducer::new(walk_config);
        let report = producer.produce(&ctx).await.unwrap();
        assert_eq!(report.envelopes_emitted, 0);
        assert_eq!(report.batches_emitted, 0);
        let rows = handle
            .lock()
            .await
            .list_producer_checkpoints_for(CLAUDE_ARCHIVE_PRODUCER_NAME, 50)
            .unwrap();
        assert!(rows.is_empty());
    }
}
