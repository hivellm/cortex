//! phase11w — Tool-call digest summariser.
//!
//! `turn_digest` (phase9e) bucketises old turns into one weekly
//! `Memory{memory_type=turn_digest}` per `(repo, year_week, top_topic)`.
//! Tool calls were left out of that pipeline — and they are the
//! single largest event class in the lane (28 141 of 38 866 in the
//! 2026-05-05 snapshot, 72 % of `events_total`). Most are short
//! `Bash` / `Read` / `Edit` / `Grep` invocations whose individual
//! payloads carry no long-term value but whose AGGREGATE shape (which
//! tools were busy in which weeks under which repo) is the actual
//! retrieval signal.
//!
//! This module summarises tool calls older than `digest_after_days`
//! into one `Memory{memory_type=tool_call_digest}` per
//! `(repo, year_week, tool)` bucket and — unlike `turn_digest` —
//! HARD-DELETES the originals from Meili + Vectorizer + Parquet
//! after the digest lands. The delete is gated by an explicit
//! `purge_originals` flag on the plan so the operator can preview
//! which buckets would shrink before committing.
//!
//! Library shape (the binary path lives in `cortex-cli`'s
//! `cortex-ops` bin so it shares the existing operator surface):
//!
//! - [`ToolCall`] — one source row the bucketiser consumes.
//! - [`Bucket`] — one `(repo, year_week, tool)` group.
//! - [`bucketize`] — pure function honouring `digest_after_days` +
//!   `min_bucket_size`.
//! - [`ToolCallDigestBackend`] trait — `lookup_existing`,
//!   `summarize`, `persist_digest`, `tag_source_tool_calls`,
//!   `delete_source_tool_calls`.
//! - [`run_tool_call_digest`] — orchestrator that maps buckets onto
//!   backend calls under a budget ceiling and returns a
//!   [`ToolCallDigestReport`] suitable for the `retention_sweeps`
//!   row.

use std::collections::BTreeMap;

use async_trait::async_trait;
use chrono::{DateTime, Datelike, Duration, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// One source tool-call row the bucketiser consumes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCall {
    /// Envelope id (`event_id`).
    pub event_id: String,
    /// Repo slug from `context.repo`. Required — tool calls without
    /// a repo are filtered out.
    pub repo: String,
    /// `occurred_at`.
    pub occurred_at: DateTime<Utc>,
    /// Tool name (`Bash`, `Read`, `Edit`, `Grep`, …). Required —
    /// untagged tool calls land in the `(repo, year_week, "other")`
    /// bucket so they are still summarised + purged.
    pub tool: String,
    /// `payload.summarized_by` if already set — flips the bucket out
    /// of the candidate set so re-runs are no-ops.
    pub summarized_by: Option<String>,
}

/// One bucket the orchestrator walks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bucket {
    /// Repo slug.
    pub repo: String,
    /// ISO year-week label (`YYYY-Www`).
    pub year_week: String,
    /// Tool name.
    pub tool: String,
    /// Source event ids in deterministic insertion order.
    pub event_ids: Vec<String>,
}

impl Bucket {
    /// Stable `(repo, year_week, tool)` key used in the bookkeeping
    /// row.
    pub fn key(&self) -> String {
        format!("{}|{}|{}", self.repo, self.year_week, self.tool)
    }
}

/// Plan inputs.
#[derive(Debug, Clone)]
pub struct DigestPlan {
    /// Reference time (defaults to `Utc::now()`).
    pub now: DateTime<Utc>,
    /// Minimum age (days) before a tool call enters the digest
    /// pipeline. Default 30 — matches `turn_digest`.
    pub digest_after_days: i64,
    /// Minimum bucket size — single-call buckets are never worth a
    /// classifier round-trip. Default 5.
    pub min_bucket_size: usize,
    /// Per-run budget ceiling in US cents.
    pub max_usd_cents_per_run: u64,
    /// Cost-per-call estimate the orchestrator uses to decide
    /// whether the next bucket fits inside the remaining budget.
    pub estimated_usd_cents_per_call: u64,
    /// `true` skips every backend mutation but still surfaces
    /// `buckets_pending` so the operator can preview.
    pub dry_run: bool,
    /// `true` rebuilds existing digests in place — the orchestrator
    /// calls `persist_digest` even when `lookup_existing` returns
    /// `Some(_)`.
    pub rebuild: bool,
    /// **phase11w core knob.** When `true`, after `persist_digest`
    /// and `tag_source_tool_calls` succeed, the orchestrator calls
    /// `delete_source_tool_calls` to hard-purge the originals from
    /// Meili + Vectorizer + Parquet. Default `false` so the first
    /// production run is observable before deletes happen.
    pub purge_originals: bool,
}

impl DigestPlan {
    /// Defaults per spec.
    pub fn default_for(now: DateTime<Utc>) -> Self {
        Self {
            now,
            digest_after_days: 30,
            min_bucket_size: 5,
            max_usd_cents_per_run: 500,
            estimated_usd_cents_per_call: 5,
            dry_run: false,
            rebuild: false,
            purge_originals: false,
        }
    }
}

/// Bucketise a vec of tool calls into one bucket per `(repo,
/// year_week, tool)` whose age + size pass the plan's thresholds.
///
/// Tool calls where `summarized_by` is already set drop out so a
/// re-run after a successful digest is a no-op.
pub fn bucketize(plan: &DigestPlan, tool_calls: Vec<ToolCall>) -> Vec<Bucket> {
    let cutoff = plan.now - Duration::days(plan.digest_after_days);
    let mut groups: BTreeMap<(String, String, String), Vec<String>> = BTreeMap::new();
    for tc in tool_calls {
        if tc.summarized_by.is_some() {
            continue;
        }
        if tc.occurred_at >= cutoff {
            continue;
        }
        let key = (tc.repo, iso_year_week(tc.occurred_at), tc.tool);
        groups.entry(key).or_default().push(tc.event_id);
    }
    let mut out = Vec::with_capacity(groups.len());
    for ((repo, year_week, tool), event_ids) in groups {
        if event_ids.len() < plan.min_bucket_size {
            continue;
        }
        out.push(Bucket {
            repo,
            year_week,
            tool,
            event_ids,
        });
    }
    out
}

/// Compute the ISO 8601 week label `YYYY-Www`.
pub fn iso_year_week(ts: DateTime<Utc>) -> String {
    let iso = ts.iso_week();
    format!("{:04}-W{:02}", iso.year(), iso.week())
}

/// One classifier call's outcome.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DigestResult {
    /// Generated digest body.
    pub body: String,
    /// Tokens-in.
    pub tokens_in: u64,
    /// Tokens-out.
    pub tokens_out: u64,
    /// Estimated USD cents.
    pub usd_cents: u64,
}

/// Per-bucket outcome surfaced in the report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BucketOutcome {
    /// `(repo, year_week, tool)` key.
    pub key: String,
    /// `true` when the digest was newly produced in this run.
    pub digested: bool,
    /// `true` when the bucket already had a digest and `--rebuild`
    /// was off.
    pub already_digested: bool,
    /// Number of source tool-calls hard-purged when
    /// `purge_originals = true`. Zero on dry runs / preview runs.
    pub purged: u64,
    /// `Some(reason)` on a per-bucket failure.
    pub error: Option<String>,
}

/// Counters returned by [`run_tool_call_digest`].
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolCallDigestReport {
    /// Buckets considered.
    pub examined: u64,
    /// Buckets digested in this run.
    pub buckets_done: u64,
    /// Buckets that already had a digest.
    pub already_digested: u64,
    /// Buckets the budget cut off mid-run.
    pub buckets_pending: u64,
    /// Cumulative spend in US cents.
    pub usd_cents: u64,
    /// Total tool-call rows hard-purged across every bucket.
    /// Headline counter for the `Bytes reclaimed last 30 d`
    /// dashboard panel.
    pub records_purged: u64,
    /// Per-bucket outcomes — bookkeeping row writes the JSON
    /// serialisation under `tier_transitions_json.tool_call_digest`.
    pub outcomes: Vec<BucketOutcome>,
}

impl ToolCallDigestReport {
    /// JSON-encoded summary suitable for `tier_transitions_json`.
    pub fn tool_call_digest_json(&self) -> String {
        serde_json::to_string(&serde_json::json!({
            "buckets_done": self.buckets_done,
            "buckets_pending": self.buckets_pending,
            "already_digested": self.already_digested,
            "records_purged": self.records_purged,
            "usd_cents": self.usd_cents,
        }))
        .unwrap_or_else(|_| "{}".into())
    }
}

/// Orchestrator errors.
#[derive(Debug, Error)]
pub enum DigestError {
    /// Backend reported an unrecoverable error mid-flight.
    #[error("backend: {0}")]
    Backend(String),
}

/// Mutator surface the orchestrator depends on. Production wires
/// the live Sonnet classifier + embedder + Nexus + Parquet
/// rewriter + Meili delete-batch + Vectorizer `delete_vectors`.
/// Tests use [`MemoryToolCallDigestBackend`].
#[async_trait]
pub trait ToolCallDigestBackend: Send + Sync {
    /// `Some(digest_event_id)` when a digest already exists for
    /// `(repo, year_week, tool)`.
    async fn lookup_existing(
        &self,
        repo: &str,
        year_week: &str,
        tool: &str,
    ) -> Result<Option<String>, String>;

    /// Summarise the bucket via the classifier.
    async fn summarize(&self, bucket: &Bucket) -> Result<DigestResult, String>;

    /// Persist the digest end-to-end:
    /// - emit `cortex.events.enriched` with `kind=memory`,
    ///   `memory_type=tool_call_digest`,
    /// - embed via the embedder + upsert into `cortex.memory.fp32`,
    /// - insert the `:Memory{memory_type:tool_call_digest}` Nexus node,
    /// - link `(:Memory)-[:SUMMARIZES]->(:ToolCall)` for every
    ///   source event id.
    /// Returns the digest event id.
    async fn persist_digest(
        &self,
        bucket: &Bucket,
        digest: &DigestResult,
    ) -> Result<String, String>;

    /// Tag every source tool-call row's Parquet with
    /// `payload.summarized_by = <digest_event_id>` so future
    /// `bucketize` runs short-circuit the same buckets even when
    /// the originals are kept (`purge_originals = false`).
    async fn tag_source_tool_calls(
        &self,
        digest_event_id: &str,
        event_ids: &[String],
    ) -> Result<(), String>;

    /// Hard-purge the source tool-call rows from Meili
    /// (`cortex_tool_calls`), Vectorizer
    /// (`cortex.tool_call.fp32` + `.pq` + `.cold.binary`), and
    /// every Parquet partition that carries them. Called only
    /// when `plan.purge_originals = true`. Returns the count of
    /// rows actually deleted (so the report can surface the
    /// per-bucket headline).
    async fn delete_source_tool_calls(&self, event_ids: &[String]) -> Result<u64, String>;
}

/// Run the orchestrator against `backend`. The bucket order is
/// stable (sorted by `(repo, year_week, tool)`) so a budget cut-off
/// on day N resumes from the same point on day N+1.
pub async fn run_tool_call_digest(
    plan: &DigestPlan,
    backend: &dyn ToolCallDigestBackend,
    tool_calls: Vec<ToolCall>,
) -> Result<ToolCallDigestReport, DigestError> {
    let buckets = bucketize(plan, tool_calls);
    let mut report = ToolCallDigestReport {
        examined: buckets.len() as u64,
        ..Default::default()
    };
    for bucket in buckets {
        if report
            .usd_cents
            .saturating_add(plan.estimated_usd_cents_per_call)
            > plan.max_usd_cents_per_run
        {
            report.buckets_pending += 1;
            continue;
        }
        let existing = backend
            .lookup_existing(&bucket.repo, &bucket.year_week, &bucket.tool)
            .await
            .map_err(DigestError::Backend)?;
        if existing.is_some() && !plan.rebuild {
            report.already_digested += 1;
            report.outcomes.push(BucketOutcome {
                key: bucket.key(),
                digested: false,
                already_digested: true,
                purged: 0,
                error: None,
            });
            continue;
        }
        if plan.dry_run {
            report.outcomes.push(BucketOutcome {
                key: bucket.key(),
                digested: false,
                already_digested: false,
                purged: 0,
                error: None,
            });
            report.buckets_pending += 1;
            continue;
        }
        match digest_one(&bucket, plan, backend).await {
            Ok((usd, purged)) => {
                report.buckets_done += 1;
                report.usd_cents = report.usd_cents.saturating_add(usd);
                report.records_purged = report.records_purged.saturating_add(purged);
                report.outcomes.push(BucketOutcome {
                    key: bucket.key(),
                    digested: true,
                    already_digested: false,
                    purged,
                    error: None,
                });
            }
            Err(reason) => {
                report.outcomes.push(BucketOutcome {
                    key: bucket.key(),
                    digested: false,
                    already_digested: false,
                    purged: 0,
                    error: Some(reason),
                });
            }
        }
    }
    Ok(report)
}

async fn digest_one(
    bucket: &Bucket,
    plan: &DigestPlan,
    backend: &dyn ToolCallDigestBackend,
) -> Result<(u64, u64), String> {
    let digest = backend.summarize(bucket).await?;
    let usd = digest.usd_cents;
    let event_id = backend.persist_digest(bucket, &digest).await?;
    // When `purge_originals` is on, the source rows disappear at
    // the next call so the idempotence tag they would carry is
    // wasted I/O; skip tagging in that branch. When the operator
    // wants to KEEP the originals (purge off), the tag path is
    // mandatory because the next bucketize pass uses
    // `payload.summarized_by` to short-circuit already-digested
    // buckets.
    let purged = if plan.purge_originals {
        backend.delete_source_tool_calls(&bucket.event_ids).await?
    } else {
        backend
            .tag_source_tool_calls(&event_id, &bucket.event_ids)
            .await?;
        0
    };
    Ok((usd, purged))
}

// ---- in-memory test double ------------------------------------------

/// In-memory test backend.
#[derive(Debug, Default)]
pub struct MemoryToolCallDigestBackend {
    inner: tokio::sync::Mutex<MemoryToolCallDigestState>,
}

#[derive(Debug, Default)]
struct MemoryToolCallDigestState {
    pub existing: BTreeMap<String, String>,
    pub summaries: Vec<(String, String, String, usize)>,
    pub persisted: Vec<(String, String, String, String)>,
    pub tag_calls: Vec<(String, Vec<String>)>,
    pub delete_calls: Vec<Vec<String>>,
    pub summary_override: Option<DigestResult>,
}

impl MemoryToolCallDigestBackend {
    /// Fresh empty backend.
    pub fn new() -> Self {
        Self::default()
    }

    /// Pre-seed an existing digest for `(repo, week, tool)`.
    pub async fn pre_existing(&self, repo: &str, year_week: &str, tool: &str, digest_id: &str) {
        let key = format!("{repo}|{year_week}|{tool}");
        self.inner
            .lock()
            .await
            .existing
            .insert(key, digest_id.to_string());
    }

    /// Override the body / cost the test backend returns from
    /// `summarize`.
    pub async fn set_summary(&self, result: DigestResult) {
        self.inner.lock().await.summary_override = Some(result);
    }

    /// Snapshot of recorded summarise calls.
    pub async fn summaries(&self) -> Vec<(String, String, String, usize)> {
        self.inner.lock().await.summaries.clone()
    }

    /// Snapshot of recorded persist calls.
    pub async fn persisted(&self) -> Vec<(String, String, String, String)> {
        self.inner.lock().await.persisted.clone()
    }

    /// Snapshot of recorded tag calls.
    pub async fn tag_calls(&self) -> Vec<(String, Vec<String>)> {
        self.inner.lock().await.tag_calls.clone()
    }

    /// Snapshot of recorded delete calls (one entry per bucket
    /// whose originals were purged).
    pub async fn delete_calls(&self) -> Vec<Vec<String>> {
        self.inner.lock().await.delete_calls.clone()
    }
}

#[async_trait]
impl ToolCallDigestBackend for MemoryToolCallDigestBackend {
    async fn lookup_existing(
        &self,
        repo: &str,
        year_week: &str,
        tool: &str,
    ) -> Result<Option<String>, String> {
        let key = format!("{repo}|{year_week}|{tool}");
        Ok(self.inner.lock().await.existing.get(&key).cloned())
    }

    async fn summarize(&self, bucket: &Bucket) -> Result<DigestResult, String> {
        let mut s = self.inner.lock().await;
        s.summaries.push((
            bucket.repo.clone(),
            bucket.year_week.clone(),
            bucket.tool.clone(),
            bucket.event_ids.len(),
        ));
        Ok(s.summary_override.clone().unwrap_or(DigestResult {
            body: format!(
                "Synthetic digest of {} {} calls in {}/{}",
                bucket.event_ids.len(),
                bucket.tool,
                bucket.repo,
                bucket.year_week
            ),
            tokens_in: 0,
            tokens_out: 0,
            usd_cents: 5,
        }))
    }

    async fn persist_digest(
        &self,
        bucket: &Bucket,
        _digest: &DigestResult,
    ) -> Result<String, String> {
        let event_id = format!("01TCD-{}-{}-{}", bucket.repo, bucket.year_week, bucket.tool);
        self.inner.lock().await.persisted.push((
            bucket.repo.clone(),
            bucket.year_week.clone(),
            bucket.tool.clone(),
            event_id.clone(),
        ));
        Ok(event_id)
    }

    async fn tag_source_tool_calls(
        &self,
        digest_event_id: &str,
        event_ids: &[String],
    ) -> Result<(), String> {
        self.inner
            .lock()
            .await
            .tag_calls
            .push((digest_event_id.to_string(), event_ids.to_vec()));
        Ok(())
    }

    async fn delete_source_tool_calls(&self, event_ids: &[String]) -> Result<u64, String> {
        self.inner
            .lock()
            .await
            .delete_calls
            .push(event_ids.to_vec());
        Ok(event_ids.len() as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 5, 5, 0, 0, 0).unwrap()
    }

    fn old(repo: &str, tool: &str, n: usize) -> Vec<ToolCall> {
        (0..n)
            .map(|i| ToolCall {
                event_id: format!("01OLD-{tool}-{i}"),
                repo: repo.to_string(),
                occurred_at: now() - Duration::days(60),
                tool: tool.to_string(),
                summarized_by: None,
            })
            .collect()
    }

    #[test]
    fn bucketize_groups_old_tool_calls_by_repo_week_tool() {
        let plan = DigestPlan::default_for(now());
        let mut tcs = old("cortex", "Bash", 6);
        tcs.extend(old("cortex", "Read", 5));
        // 4 Edit calls — below threshold, drop bucket
        tcs.extend(old("cortex", "Edit", 4));
        let buckets = bucketize(&plan, tcs);
        assert_eq!(buckets.len(), 2, "Bash + Read survive, Edit dropped");
        assert!(buckets
            .iter()
            .any(|b| b.tool == "Bash" && b.event_ids.len() == 6));
        assert!(buckets
            .iter()
            .any(|b| b.tool == "Read" && b.event_ids.len() == 5));
    }

    #[test]
    fn bucketize_skips_already_summarized_tool_calls() {
        let plan = DigestPlan::default_for(now());
        let mut tcs = old("cortex", "Bash", 6);
        for tc in &mut tcs {
            tc.summarized_by = Some("01PRIOR".into());
        }
        let buckets = bucketize(&plan, tcs);
        assert!(buckets.is_empty());
    }

    #[test]
    fn bucketize_skips_fresh_tool_calls() {
        let plan = DigestPlan::default_for(now());
        let fresh: Vec<_> = (0..10)
            .map(|i| ToolCall {
                event_id: format!("01FRESH-{i}"),
                repo: "cortex".into(),
                occurred_at: now() - Duration::days(5),
                tool: "Bash".into(),
                summarized_by: None,
            })
            .collect();
        let buckets = bucketize(&plan, fresh);
        assert!(
            buckets.is_empty(),
            "tool calls < 30 d old must not bucketise"
        );
    }

    #[tokio::test]
    async fn run_tool_call_digest_calls_summarize_persist_tag_per_bucket() {
        // purge_originals=false → tag pass is mandatory (idempotence
        // marker on the source rows the operator chose to keep).
        let plan = DigestPlan::default_for(now());
        let backend = MemoryToolCallDigestBackend::new();
        let mut tcs = old("cortex", "Bash", 6);
        tcs.extend(old("cortex", "Read", 5));
        let report = run_tool_call_digest(&plan, &backend, tcs).await.unwrap();
        assert_eq!(report.examined, 2);
        assert_eq!(report.buckets_done, 2);
        assert_eq!(
            report.records_purged, 0,
            "purge_originals=false → no deletes"
        );
        assert_eq!(backend.summaries().await.len(), 2);
        assert_eq!(backend.persisted().await.len(), 2);
        assert_eq!(backend.tag_calls().await.len(), 2);
        assert!(backend.delete_calls().await.is_empty());
    }

    #[tokio::test]
    async fn run_tool_call_digest_purges_originals_when_flag_on() {
        // purge_originals=true → orchestrator skips the tag pass
        // because the source rows are about to disappear.
        let mut plan = DigestPlan::default_for(now());
        plan.purge_originals = true;
        let backend = MemoryToolCallDigestBackend::new();
        let tcs = old("cortex", "Bash", 6);
        let report = run_tool_call_digest(&plan, &backend, tcs).await.unwrap();
        assert_eq!(report.buckets_done, 1);
        assert_eq!(
            report.records_purged, 6,
            "every original tool-call hard-purged"
        );
        let calls = backend.delete_calls().await;
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].len(), 6);
        // tag pass MUST be skipped when originals are being purged.
        assert!(
            backend.tag_calls().await.is_empty(),
            "tag pass is wasted I/O when originals are about to be purged"
        );
    }

    #[tokio::test]
    async fn run_tool_call_digest_idempotent_skips_existing_buckets() {
        let plan = DigestPlan::default_for(now());
        let backend = MemoryToolCallDigestBackend::new();
        let tcs = old("cortex", "Bash", 6);
        let week = iso_year_week(now() - Duration::days(60));
        backend
            .pre_existing("cortex", &week, "Bash", "01PRIOR")
            .await;
        let report = run_tool_call_digest(&plan, &backend, tcs).await.unwrap();
        assert_eq!(report.examined, 1);
        assert_eq!(report.already_digested, 1);
        assert_eq!(report.buckets_done, 0);
        assert!(backend.summaries().await.is_empty());
    }

    #[tokio::test]
    async fn run_tool_call_digest_dry_run_does_not_call_summarize() {
        let mut plan = DigestPlan::default_for(now());
        plan.dry_run = true;
        plan.purge_originals = true; // even with purge ON, dry-run blocks deletes
        let backend = MemoryToolCallDigestBackend::new();
        let tcs = old("cortex", "Bash", 6);
        let report = run_tool_call_digest(&plan, &backend, tcs).await.unwrap();
        assert_eq!(report.examined, 1);
        assert_eq!(report.buckets_pending, 1);
        assert_eq!(report.records_purged, 0);
        assert!(backend.summaries().await.is_empty());
        assert!(backend.delete_calls().await.is_empty());
    }

    #[tokio::test]
    async fn run_tool_call_digest_budget_cuts_off_pending_buckets() {
        let mut plan = DigestPlan::default_for(now());
        plan.estimated_usd_cents_per_call = 5;
        plan.max_usd_cents_per_run = 5; // exactly one call's worth
        let backend = MemoryToolCallDigestBackend::new();
        let mut tcs = old("cortex", "Bash", 6);
        tcs.extend(old("cortex", "Read", 5));
        tcs.extend(old("cortex", "Grep", 5));
        let report = run_tool_call_digest(&plan, &backend, tcs).await.unwrap();
        assert_eq!(report.examined, 3);
        assert_eq!(report.buckets_done, 1);
        assert_eq!(report.buckets_pending, 2);
    }

    #[test]
    fn report_json_carries_the_purge_counter_for_bookkeeping() {
        let report = ToolCallDigestReport {
            buckets_done: 4,
            records_purged: 1289,
            usd_cents: 20,
            ..Default::default()
        };
        let json = report.tool_call_digest_json();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["records_purged"], 1289);
        assert_eq!(parsed["buckets_done"], 4);
        assert_eq!(parsed["usd_cents"], 20);
    }
}
