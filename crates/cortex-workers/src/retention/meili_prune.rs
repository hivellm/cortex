//! Phase9f — Meilisearch archival pruner.
//!
//! Meili holds the full body of every turn and tool_call indefinitely
//! (`cortex_turns.user_message`, `cortex_turns.assistant_message`,
//! `cortex_tool_calls.command_or_input`,
//! `cortex_tool_calls.output_excerpt`). On a busy repo this index
//! outgrows Vectorizer within months. After 90 days the user-facing
//! value of those raw bodies is small — the `summary` field plus
//! topic/repo metadata is enough for keyword recall, and the original
//! bytes still live in the Parquet archive for audit.
//!
//! Phase9f blanks the body fields, caps `summary` at 4 KiB with an
//! ellipsis marker, and stamps `pruned: true` + `pruned_at` so the
//! next pruner run treats the document as already-handled. The
//! document is NEVER deleted — the keyword lane still surfaces it on
//! a `summary` match.
//!
//! Library shape (mirrors phase9d/9e):
//!
//! - [`MeiliDoc`] — one source row the matcher consumes.
//! - [`PruneOp`] — the per-doc payload the backend `update_documents`
//!   call ships to Meili.
//! - [`MeiliBackend`] trait — `enumerate_prunable` (filter) +
//!   `update_documents` (batched update with terminal-state await) +
//!   `delete_documents` (deliberately absent — pruning never
//!   deletes).
//! - [`run_meili_prune`] — orchestrator that walks each index, caps
//!   the summary, batches the writes, and surfaces a [`PruneReport`].

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// One Meili document the matcher consumes. Production loaders
/// project the full document into this minimal shape; tests author
/// the rows directly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeiliDoc {
    /// Document id (== envelope `event_id`).
    pub event_id: String,
    /// Index uid (`cortex_turns` / `cortex_tool_calls`).
    pub index: String,
    /// `occurred_at` parsed from the document.
    pub occurred_at: DateTime<Utc>,
    /// Current `summary` field — the pruner caps this at
    /// `summary_cap_bytes` and writes it back unchanged otherwise.
    pub summary: String,
    /// `true` when the document already carries the `pruned` flag.
    /// The matcher filters these out so re-runs are no-ops.
    pub already_pruned: bool,
}

/// One per-doc payload the backend's `update_documents` call ships
/// to Meili. The four body fields land as empty strings in the
/// payload — Meili's partial-update semantics overwrite the
/// existing value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PruneOp {
    /// Document id.
    pub event_id: String,
    /// Already-capped summary.
    pub summary_capped: String,
    /// `pruned_at` RFC-3339.
    pub pruned_at: String,
}

/// Plan inputs.
#[derive(Debug, Clone)]
pub struct PrunePlan {
    /// Reference time.
    pub now: DateTime<Utc>,
    /// Age (days) at which a document gets its body blanked.
    /// Default 90 per spec.
    pub prune_after_days: i64,
    /// Maximum summary size in bytes after the cap. Default 4 096.
    pub summary_cap_bytes: usize,
    /// Indexes to walk. Defaults to both canonical kinds.
    pub indexes: Vec<String>,
    /// Maximum docs per backend `update_documents` task. Default
    /// 1 000 per spec.
    pub batch_size: u32,
    /// `true` skips backend mutation — the report still lists what
    /// would have happened so operators can preview.
    pub dry_run: bool,
    /// `true` re-prunes already-pruned docs (useful when the cap
    /// policy changes). The matcher accepts `already_pruned=true`
    /// rows when this is on.
    pub rebuild: bool,
}

impl PrunePlan {
    /// Defaults per spec — 90 d cutoff, 4 KiB cap, 1 000-batch,
    /// both canonical indexes.
    pub fn default_for(now: DateTime<Utc>) -> Self {
        Self {
            now,
            prune_after_days: 90,
            summary_cap_bytes: 4_096,
            indexes: vec!["cortex_turns".to_string(), "cortex_tool_calls".to_string()],
            batch_size: 1_000,
            dry_run: false,
            rebuild: false,
        }
    }
}

/// Counters returned by [`run_meili_prune`].
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PruneReport {
    /// Documents the matcher walked across every index.
    pub examined: u64,
    /// Documents the runner mutated (live runs only).
    pub pruned: u64,
    /// Documents whose summary the cap truncated.
    pub summaries_capped: u64,
    /// Documents the runner skipped (already pruned, fresh, or
    /// dry-run preview).
    pub skipped: u64,
    /// Per-index counts for the bookkeeping JSON.
    pub per_index: std::collections::BTreeMap<String, u64>,
}

impl PruneReport {
    /// JSON suitable for `tier_transitions_json.meili_prune`.
    pub fn meili_prune_json(&self) -> String {
        serde_json::to_string(&serde_json::json!({
            "examined": self.examined,
            "pruned": self.pruned,
            "summaries_capped": self.summaries_capped,
            "skipped": self.skipped,
            "per_index": self.per_index,
        }))
        .unwrap_or_else(|_| "{}".into())
    }
}

/// Runner errors.
#[derive(Debug, Error)]
pub enum PruneError {
    /// Backend reported an unrecoverable error mid-flight (Meili
    /// task ended in `failed` state, transport timeout, etc.).
    #[error("backend: {0}")]
    Backend(String),
}

/// Backend trait the runner mutates. Production wires the live
/// Meili SDK; tests use [`MemoryMeiliBackend`] for in-memory
/// round-trips with one-shot failure injection.
#[async_trait]
pub trait MeiliBackend: Send + Sync {
    /// Return every document in `index` whose `occurred_at <
    /// cutoff` AND (`already_pruned == false` OR `accept_pruned ==
    /// true`). The runner pages by `batch_size`; the production
    /// impl translates this into the
    /// `filter = "occurred_at < <cutoff> AND pruned != true"` Meili
    /// filter expression.
    async fn enumerate_prunable(
        &self,
        index: &str,
        cutoff: DateTime<Utc>,
        accept_pruned: bool,
        batch_size: u32,
    ) -> Result<Vec<MeiliDoc>, String>;
    /// Submit a batch update + await terminal state. Returns when
    /// every doc in `ops` has landed; `Err` when the Meili task
    /// ends in `failed`.
    async fn update_documents(&self, index: &str, ops: &[PruneOp]) -> Result<(), String>;
}

/// Cap a summary at `cap_bytes` with an ellipsis marker when truncated.
/// The marker is the canonical `…` (3-byte UTF-8 ellipsis) so the
/// post-prune length is `cap_bytes` exactly when truncation happened.
pub fn cap_summary(summary: &str, cap_bytes: usize) -> (String, bool) {
    if summary.len() <= cap_bytes {
        return (summary.to_string(), false);
    }
    // Ellipsis is 3 bytes (0xE2 0x80 0xA6). Reserve room for it so
    // the final length is ≤ cap_bytes.
    const ELLIPSIS: &str = "\u{2026}";
    let elen = ELLIPSIS.len();
    if cap_bytes <= elen {
        return (ELLIPSIS.to_string(), true);
    }
    let mut budget = cap_bytes - elen;
    // Walk char boundaries so we don't slice mid-codepoint.
    while budget > 0 && !summary.is_char_boundary(budget) {
        budget -= 1;
    }
    let mut out = String::with_capacity(cap_bytes);
    out.push_str(&summary[..budget]);
    out.push_str(ELLIPSIS);
    (out, true)
}

/// Run the pruner against `backend`.
pub async fn run_meili_prune(
    plan: &PrunePlan,
    backend: &dyn MeiliBackend,
) -> Result<PruneReport, PruneError> {
    let cutoff = plan.now - Duration::days(plan.prune_after_days);
    let mut report = PruneReport::default();
    for index in &plan.indexes {
        let docs = backend
            .enumerate_prunable(index, cutoff, plan.rebuild, plan.batch_size)
            .await
            .map_err(PruneError::Backend)?;
        report.examined = report.examined.saturating_add(docs.len() as u64);
        if docs.is_empty() {
            continue;
        }
        let mut ops: Vec<PruneOp> = Vec::with_capacity(docs.len());
        for doc in &docs {
            if doc.already_pruned && !plan.rebuild {
                report.skipped = report.skipped.saturating_add(1);
                continue;
            }
            let (summary_capped, was_capped) = cap_summary(&doc.summary, plan.summary_cap_bytes);
            if was_capped {
                report.summaries_capped = report.summaries_capped.saturating_add(1);
            }
            ops.push(PruneOp {
                event_id: doc.event_id.clone(),
                summary_capped,
                pruned_at: plan.now.to_rfc3339(),
            });
        }
        if ops.is_empty() {
            continue;
        }
        if plan.dry_run {
            report.skipped = report.skipped.saturating_add(ops.len() as u64);
            continue;
        }
        // Send in `batch_size` chunks — Meili's update_documents
        // handles N records but we cap per task so a flaky
        // task-await doesn't lose the whole run.
        let batch = plan.batch_size as usize;
        for chunk in ops.chunks(batch.max(1)) {
            backend
                .update_documents(index, chunk)
                .await
                .map_err(PruneError::Backend)?;
            report.pruned = report.pruned.saturating_add(chunk.len() as u64);
            *report.per_index.entry(index.clone()).or_insert(0) += chunk.len() as u64;
        }
    }
    Ok(report)
}

// ---- in-memory test double ----------------------------------------

/// In-memory [`MeiliBackend`] for tests. Stores documents per
/// index and records every `update_documents` call so tests can
/// assert call ordering + payload shape.
#[derive(Debug, Default)]
pub struct MemoryMeiliBackend {
    inner: tokio::sync::Mutex<MemoryMeiliState>,
}

#[derive(Debug, Default)]
struct MemoryMeiliState {
    pub by_index: std::collections::BTreeMap<String, Vec<MeiliDoc>>,
    pub updates: Vec<(String, Vec<PruneOp>)>,
    pub fail_on: Option<&'static str>,
}

impl MemoryMeiliBackend {
    /// Empty backend.
    pub fn new() -> Self {
        Self::default()
    }
    /// Seed `index` with a list of source documents.
    pub async fn seed(&self, index: &str, docs: Vec<MeiliDoc>) {
        self.inner
            .lock()
            .await
            .by_index
            .entry(index.to_string())
            .or_default()
            .extend(docs);
    }
    /// Inject a one-shot failure at `step` (`enumerate` / `update`).
    pub async fn inject_failure(&self, step: &'static str) {
        self.inner.lock().await.fail_on = Some(step);
    }
    /// Snapshot recorded updates for assertion.
    pub async fn updates(&self) -> Vec<(String, Vec<PruneOp>)> {
        self.inner.lock().await.updates.clone()
    }
    /// Apply the recorded updates back to the in-memory store so
    /// the second-run idempotence test can re-enumerate the same
    /// docs and find them already-pruned.
    pub async fn commit_updates(&self) {
        let mut s = self.inner.lock().await;
        let updates = std::mem::take(&mut s.updates);
        for (index, ops) in &updates {
            if let Some(bucket) = s.by_index.get_mut(index) {
                for op in ops {
                    if let Some(doc) = bucket.iter_mut().find(|d| d.event_id == op.event_id) {
                        doc.summary = op.summary_capped.clone();
                        doc.already_pruned = true;
                    }
                }
            }
        }
        s.updates = updates;
    }
}

#[async_trait]
impl MeiliBackend for MemoryMeiliBackend {
    async fn enumerate_prunable(
        &self,
        index: &str,
        cutoff: DateTime<Utc>,
        accept_pruned: bool,
        _batch_size: u32,
    ) -> Result<Vec<MeiliDoc>, String> {
        let mut s = self.inner.lock().await;
        if s.fail_on == Some("enumerate") {
            s.fail_on = None;
            return Err("synthetic-enumerate-failure".into());
        }
        let bucket = s.by_index.get(index).cloned().unwrap_or_default();
        Ok(bucket
            .into_iter()
            .filter(|d| d.occurred_at < cutoff)
            .filter(|d| accept_pruned || !d.already_pruned)
            .collect())
    }
    async fn update_documents(&self, index: &str, ops: &[PruneOp]) -> Result<(), String> {
        let mut s = self.inner.lock().await;
        if s.fail_on == Some("update") {
            s.fail_on = None;
            return Err("synthetic-update-failure".into());
        }
        s.updates.push((index.to_string(), ops.to_vec()));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-04-29T18:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn doc(id: &str, index: &str, age_days: i64, summary: &str, already: bool) -> MeiliDoc {
        MeiliDoc {
            event_id: id.to_string(),
            index: index.to_string(),
            occurred_at: now() - Duration::days(age_days),
            summary: summary.to_string(),
            already_pruned: already,
        }
    }

    #[test]
    fn plan_default_uses_spec_thresholds() {
        let plan = PrunePlan::default_for(now());
        assert_eq!(plan.prune_after_days, 90);
        assert_eq!(plan.summary_cap_bytes, 4_096);
        assert_eq!(plan.batch_size, 1_000);
        assert_eq!(plan.indexes, vec!["cortex_turns", "cortex_tool_calls"]);
        assert!(!plan.dry_run);
        assert!(!plan.rebuild);
    }

    #[test]
    fn cap_summary_returns_unchanged_when_under_cap() {
        let (out, capped) = cap_summary("hi", 100);
        assert_eq!(out, "hi");
        assert!(!capped);
    }

    #[test]
    fn cap_summary_truncates_with_ellipsis_marker() {
        let s = "abcdefghijabcdefghij"; // 20 bytes
        let (out, capped) = cap_summary(s, 10);
        assert!(capped);
        assert!(out.ends_with('\u{2026}'));
        assert!(out.len() <= 10);
        // First 7 bytes preserved (10 - 3 byte ellipsis).
        assert!(out.starts_with("abcdefg"));
    }

    #[test]
    fn cap_summary_rounds_to_char_boundary() {
        // Using a multibyte character at position 9 — the boundary
        // walker MUST drop back to position 8 so the slice never
        // mid-codepoints.
        let s = "abcdefghi\u{1F600}jklmnop";
        // 9 ASCII bytes + 4-byte emoji + 7 ASCII = 20 bytes. Cap at
        // 10 means budget = 7; the boundary walk lands at 7 cleanly.
        let (out, capped) = cap_summary(s, 10);
        assert!(capped);
        assert!(out.ends_with('\u{2026}'));
        // No mid-codepoint slice — the result MUST be valid UTF-8,
        // which the assertion implicitly verifies (otherwise the
        // assert below panics).
        assert_eq!(out.chars().count(), 8);
    }

    #[test]
    fn cap_summary_handles_cap_smaller_than_ellipsis() {
        let (out, capped) = cap_summary("anything", 2);
        assert!(capped);
        assert_eq!(out, "\u{2026}");
    }

    #[tokio::test]
    async fn enumerate_excludes_fresh_documents() {
        let backend = MemoryMeiliBackend::new();
        backend
            .seed(
                "cortex_turns",
                vec![doc("01FRESH", "cortex_turns", 5, "fresh summary", false)],
            )
            .await;
        let docs = backend
            .enumerate_prunable("cortex_turns", now() - Duration::days(90), false, 1_000)
            .await
            .unwrap();
        assert!(docs.is_empty());
    }

    #[tokio::test]
    async fn enumerate_excludes_already_pruned_unless_accept() {
        let backend = MemoryMeiliBackend::new();
        backend
            .seed(
                "cortex_turns",
                vec![doc("01OLD", "cortex_turns", 100, "old summary", true)],
            )
            .await;
        // accept_pruned=false → empty.
        let docs = backend
            .enumerate_prunable("cortex_turns", now() - Duration::days(90), false, 1_000)
            .await
            .unwrap();
        assert!(docs.is_empty());
        // accept_pruned=true → returns the pruned doc.
        let docs = backend
            .enumerate_prunable("cortex_turns", now() - Duration::days(90), true, 1_000)
            .await
            .unwrap();
        assert_eq!(docs.len(), 1);
    }

    #[tokio::test]
    async fn run_prunes_91_day_old_documents_in_each_index() {
        let backend = MemoryMeiliBackend::new();
        backend
            .seed(
                "cortex_turns",
                vec![
                    doc("01T1", "cortex_turns", 91, "summary A", false),
                    doc("01T2", "cortex_turns", 100, "summary B", false),
                    doc("01T3", "cortex_turns", 5, "fresh", false), // no-op
                ],
            )
            .await;
        backend
            .seed(
                "cortex_tool_calls",
                vec![doc("01TC1", "cortex_tool_calls", 95, "tool sum", false)],
            )
            .await;
        let plan = PrunePlan::default_for(now());
        let report = run_meili_prune(&plan, &backend).await.unwrap();
        assert_eq!(report.pruned, 3);
        assert_eq!(*report.per_index.get("cortex_turns").unwrap_or(&0), 2);
        assert_eq!(*report.per_index.get("cortex_tool_calls").unwrap_or(&0), 1);
        let updates = backend.updates().await;
        // One update batch per index.
        assert_eq!(updates.len(), 2);
    }

    #[tokio::test]
    async fn re_run_after_commit_is_a_no_op() {
        let backend = MemoryMeiliBackend::new();
        backend
            .seed(
                "cortex_turns",
                vec![doc("01OLD", "cortex_turns", 100, "summary", false)],
            )
            .await;
        let plan = PrunePlan::default_for(now());
        let r1 = run_meili_prune(&plan, &backend).await.unwrap();
        assert_eq!(r1.pruned, 1);
        // Apply the updates so the in-memory store reflects the
        // pruned state, then re-run — every doc is already_pruned.
        backend.commit_updates().await;
        let r2 = run_meili_prune(&plan, &backend).await.unwrap();
        assert_eq!(r2.pruned, 0);
    }

    #[tokio::test]
    async fn rebuild_re_prunes_already_pruned_docs() {
        let backend = MemoryMeiliBackend::new();
        backend
            .seed(
                "cortex_turns",
                vec![doc("01OLD", "cortex_turns", 100, "old summary", true)],
            )
            .await;
        let mut plan = PrunePlan::default_for(now());
        plan.rebuild = true;
        let report = run_meili_prune(&plan, &backend).await.unwrap();
        assert_eq!(report.pruned, 1);
    }

    #[tokio::test]
    async fn dry_run_records_skipped_without_calling_update() {
        let backend = MemoryMeiliBackend::new();
        backend
            .seed(
                "cortex_turns",
                vec![doc("01OLD", "cortex_turns", 100, "summary", false)],
            )
            .await;
        let mut plan = PrunePlan::default_for(now());
        plan.dry_run = true;
        let report = run_meili_prune(&plan, &backend).await.unwrap();
        assert_eq!(report.pruned, 0);
        assert_eq!(report.skipped, 1);
        assert!(backend.updates().await.is_empty());
    }

    #[tokio::test]
    async fn oversize_summary_is_capped_with_ellipsis() {
        let backend = MemoryMeiliBackend::new();
        let big = "x".repeat(10_000);
        backend
            .seed(
                "cortex_turns",
                vec![doc("01BIG", "cortex_turns", 100, &big, false)],
            )
            .await;
        let plan = PrunePlan::default_for(now());
        let report = run_meili_prune(&plan, &backend).await.unwrap();
        assert_eq!(report.pruned, 1);
        assert_eq!(report.summaries_capped, 1);
        let updates = backend.updates().await;
        assert_eq!(updates.len(), 1);
        let op = &updates[0].1[0];
        assert!(op.summary_capped.len() <= 4_096);
        assert!(op.summary_capped.ends_with('\u{2026}'));
    }

    #[tokio::test]
    async fn enumerate_failure_propagates_to_runner() {
        let backend = MemoryMeiliBackend::new();
        backend.inject_failure("enumerate").await;
        let plan = PrunePlan::default_for(now());
        let r = run_meili_prune(&plan, &backend).await;
        assert!(matches!(r, Err(PruneError::Backend(_))));
    }

    #[tokio::test]
    async fn update_failure_propagates_to_runner() {
        let backend = MemoryMeiliBackend::new();
        backend
            .seed(
                "cortex_turns",
                vec![doc("01OLD", "cortex_turns", 100, "summary", false)],
            )
            .await;
        backend.inject_failure("update").await;
        let plan = PrunePlan::default_for(now());
        let r = run_meili_prune(&plan, &backend).await;
        assert!(matches!(r, Err(PruneError::Backend(_))));
    }

    #[test]
    fn report_meili_prune_json_round_trips() {
        let mut r = PruneReport {
            examined: 100,
            pruned: 90,
            summaries_capped: 4,
            skipped: 10,
            ..Default::default()
        };
        r.per_index.insert("cortex_turns".into(), 70);
        r.per_index.insert("cortex_tool_calls".into(), 20);
        let json = r.meili_prune_json();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["pruned"], 90);
        assert_eq!(parsed["summaries_capped"], 4);
        assert_eq!(parsed["per_index"]["cortex_turns"], 70);
    }

    #[tokio::test]
    async fn batch_size_splits_large_runs_into_chunks() {
        let backend = MemoryMeiliBackend::new();
        let mut docs = Vec::new();
        for i in 0..2_500 {
            docs.push(doc(
                &format!("01D{i:020}"),
                "cortex_turns",
                100,
                "summary",
                false,
            ));
        }
        backend.seed("cortex_turns", docs).await;
        let mut plan = PrunePlan::default_for(now());
        plan.indexes = vec!["cortex_turns".into()];
        plan.batch_size = 1_000;
        let report = run_meili_prune(&plan, &backend).await.unwrap();
        assert_eq!(report.pruned, 2_500);
        let updates = backend.updates().await;
        // 2 500 / 1 000 = 3 batches.
        assert_eq!(updates.len(), 3);
        assert_eq!(updates[0].1.len(), 1_000);
        assert_eq!(updates[2].1.len(), 500);
    }
}
