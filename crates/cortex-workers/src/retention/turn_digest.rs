//! Phase9e — LLM turn digest summarizer.
//!
//! Phase9a–9d shrink storage record-by-record. They keep one row
//! per turn forever — a repo with 10 000 daily turns over a year
//! yields 3.6 M `:Turn` nodes plus 3.6 M Vectorizer vectors plus
//! 3.6 M Meili docs, most of which is noisy back-and-forth nobody
//! will ever query individually.
//!
//! Phase9e builds the dense weekly digest the user actually wants
//! to retrieve from old data: "in repo X, week N, here is what was
//! decided / what bugs were hunted / which subsystems were
//! touched". One narrative per `(repo, year_week, top_topic)` bucket
//! that satisfies the size threshold.
//!
//! Library shape (the binary path lives in `cortex-cli`'s
//! `cortex-ops` bin so it shares the existing operator surface):
//!
//! - [`Turn`] — one source row the bucketizer consumes.
//! - [`Bucket`] — one `(repo, year_week, top_topic)` group with the
//!   `event_ids` it covers.
//! - [`bucketize`] — pure function that walks a `Vec<Turn>` and
//!   yields `Vec<Bucket>` honoring `digest_after_days` and
//!   `min_bucket_size`.
//! - [`DigestBackend`] trait — minimal surface the orchestrator
//!   mutates: `lookup_existing`, `summarize`, `persist_digest`,
//!   `tag_source_turns`. Production wires the live classifier +
//!   embedder + Nexus + Parquet rewriter; tests use
//!   [`MemoryDigestBackend`].
//! - [`run_turn_digest`] — orchestrator that maps buckets to backend
//!   calls, honors the per-run cost ceiling, and returns a
//!   [`DigestReport`] suitable for the `retention_sweeps` row.

use std::collections::BTreeMap;

use async_trait::async_trait;
use chrono::{DateTime, Datelike, Duration, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// One source turn the bucketizer consumes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Turn {
    /// Envelope id.
    pub event_id: String,
    /// Repo slug from `context.repo`. Required — turns without a
    /// repo are filtered out.
    pub repo: String,
    /// `occurred_at`.
    pub occurred_at: DateTime<Utc>,
    /// Top classifier topic for the turn (`payload.topic` or
    /// `extras.classifier.top_topic`). Required — untagged turns
    /// land in the `(repo, year_week, "other")` bucket so they are
    /// still summarized.
    pub top_topic: String,
    /// `payload.summarized_by` if already set — flips the bucket
    /// out of the candidate set so re-runs are no-ops.
    pub summarized_by: Option<String>,
}

/// One bucket the orchestrator walks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bucket {
    /// Repo slug.
    pub repo: String,
    /// ISO year-week identifier (`YYYY-Www`, e.g. `2026-W17`).
    pub year_week: String,
    /// Top classifier topic; `"other"` for untagged turns.
    pub top_topic: String,
    /// Source event ids in deterministic insertion order.
    pub event_ids: Vec<String>,
}

impl Bucket {
    /// Stable string key used in `cohort_counts_json` /
    /// `tier_transitions_json.turn_digest`.
    pub fn key(&self) -> String {
        format!("{}|{}|{}", self.repo, self.year_week, self.top_topic)
    }
}

/// Plan inputs.
#[derive(Debug, Clone)]
pub struct DigestPlan {
    /// Reference time.
    pub now: DateTime<Utc>,
    /// Minimum age (days) before a turn enters the digest pipeline.
    /// Default 30 per spec.
    pub digest_after_days: i64,
    /// Minimum bucket size — single-turn weeks are not worth a
    /// digest call. Default 5 per spec.
    pub min_bucket_size: usize,
    /// Per-run budget ceiling in US cents. The orchestrator stops
    /// cleanly once cumulative spend exceeds this. Default 500
    /// per spec.
    pub max_usd_cents_per_run: u64,
    /// Cost-per-call estimate the orchestrator uses to decide
    /// whether the next bucket fits inside the remaining budget.
    /// Default 5 ¢ — Sonnet pricing on a 4 KB system + ≤512-token
    /// output round-trips at roughly that rate at 2026 list
    /// prices.
    pub estimated_usd_cents_per_call: u64,
    /// `true` skips every backend mutation but still surfaces
    /// `buckets_pending` so the operator can preview pending
    /// digests.
    pub dry_run: bool,
    /// `true` rebuilds existing digests in place — the orchestrator
    /// calls `persist_digest` even when `lookup_existing` returns
    /// `Some(_)`.
    pub rebuild: bool,
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
        }
    }
}

/// Bucketize a vec of turns into one bucket per `(repo, year_week,
/// top_topic)` whose age + size pass the plan's thresholds.
///
/// Turns where `summarized_by` is already set drop out of the
/// candidate set so a re-run after a successful digest is a no-op.
pub fn bucketize(plan: &DigestPlan, turns: Vec<Turn>) -> Vec<Bucket> {
    let cutoff = plan.now - Duration::days(plan.digest_after_days);
    let mut groups: BTreeMap<(String, String, String), Vec<String>> = BTreeMap::new();
    for t in turns {
        if t.summarized_by.is_some() {
            continue;
        }
        if t.occurred_at >= cutoff {
            continue;
        }
        let key = (t.repo, iso_year_week(t.occurred_at), t.top_topic);
        groups.entry(key).or_default().push(t.event_id);
    }
    let mut out = Vec::with_capacity(groups.len());
    for ((repo, year_week, top_topic), event_ids) in groups {
        if event_ids.len() < plan.min_bucket_size {
            continue;
        }
        out.push(Bucket {
            repo,
            year_week,
            top_topic,
            event_ids,
        });
    }
    out
}

/// Compute the ISO 8601 week label `YYYY-Www` for `ts`. Used as the
/// bucket key so weeks span midnight Sunday→Sunday consistently
/// across timezones.
pub fn iso_year_week(ts: DateTime<Utc>) -> String {
    let iso = ts.iso_week();
    format!("{:04}-W{:02}", iso.year(), iso.week())
}

/// Result of one `digest_bucket` call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DigestResult {
    /// Generated digest body (200–400 tokens per spec).
    pub body: String,
    /// Tokens-in the call consumed (for cost ledger).
    pub tokens_in: u64,
    /// Tokens-out the call produced.
    pub tokens_out: u64,
    /// Estimated USD cents. The orchestrator uses this for the
    /// running budget check.
    pub usd_cents: u64,
}

/// Per-bucket outcome surfaced in the report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BucketOutcome {
    /// Bucket the orchestrator processed.
    pub key: String,
    /// `true` when the digest was newly produced in this run.
    pub digested: bool,
    /// `true` when the bucket already had a digest and `--rebuild`
    /// was off.
    pub already_digested: bool,
    /// `Some(reason)` on a per-bucket failure.
    pub error: Option<String>,
}

/// Counters returned by [`run_turn_digest`].
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct DigestReport {
    /// Buckets considered (after `bucketize`).
    pub examined: u64,
    /// Buckets digested in this run.
    pub buckets_done: u64,
    /// Buckets that already had a digest.
    pub already_digested: u64,
    /// Buckets the budget cut off mid-run.
    pub buckets_pending: u64,
    /// Cumulative spend in US cents.
    pub usd_cents: u64,
    /// Per-bucket outcomes — bookkeeping row writes the JSON
    /// serialisation under `tier_transitions_json.turn_digest`.
    pub outcomes: Vec<BucketOutcome>,
}

impl DigestReport {
    /// JSON-encoded summary row suitable for `tier_transitions_json`.
    pub fn turn_digest_json(&self) -> String {
        serde_json::to_string(&serde_json::json!({
            "buckets_done": self.buckets_done,
            "buckets_pending": self.buckets_pending,
            "already_digested": self.already_digested,
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
/// the live classifier (Sonnet via the existing `cortex-classifier`
/// client) + embedder + Nexus writer + Parquet rewriter. Tests use
/// [`MemoryDigestBackend`] for fast in-memory round-trips with
/// one-shot failure injection.
#[async_trait]
pub trait DigestBackend: Send + Sync {
    /// `Some(digest_event_id)` when a digest already exists for
    /// `(repo, year_week, top_topic)`. Used by the idempotence
    /// guard so a re-run after a successful previous run does not
    /// call the classifier again.
    async fn lookup_existing(
        &self,
        repo: &str,
        year_week: &str,
        top_topic: &str,
    ) -> Result<Option<String>, String>;
    /// Summarize the bucket via the classifier. Production wires
    /// the existing classifier client; tests return a fixed body.
    async fn summarize(&self, bucket: &Bucket) -> Result<DigestResult, String>;
    /// Persist the digest end-to-end:
    /// - emit `cortex.events.enriched` with `kind=memory`,
    ///   `memory_type=turn_digest`,
    /// - embed via the embedder + upsert into `cortex.memory.fp32`,
    /// - insert the `:Memory{memory_type:turn_digest}` Nexus node,
    /// - link `(:Memory)-[:SUMMARIZES]->(:Turn)` for every source
    ///   event id.
    /// Returns the digest event id so the orchestrator can wire the
    /// follow-up `tag_source_turns` call.
    async fn persist_digest(
        &self,
        bucket: &Bucket,
        digest: &DigestResult,
    ) -> Result<String, String>;
    /// Tag every source turn's Parquet row with
    /// `payload.summarized_by = <digest_event_id>`. Production
    /// wires phase9b's `compact_partition` atomic-rewrite helpers;
    /// tests record the calls in a vec.
    async fn tag_source_turns(
        &self,
        digest_event_id: &str,
        event_ids: &[String],
    ) -> Result<(), String>;
}

/// Run the orchestrator against `backend`. The bucket order is
/// stable (sorted by `(repo, year_week, top_topic)` from `bucketize`)
/// so a budget cut-off on day N resumes from the same point on day
/// N+1.
pub async fn run_turn_digest(
    plan: &DigestPlan,
    backend: &dyn DigestBackend,
    turns: Vec<Turn>,
) -> Result<DigestReport, DigestError> {
    let buckets = bucketize(plan, turns);
    let mut report = DigestReport::default();
    report.examined = buckets.len() as u64;
    for bucket in buckets {
        // Budget guard before even calling `lookup_existing` —
        // an idempotence-only run still pays for the lookup, but
        // a budget-exceeded state means the operator wants the
        // run to stop completely until the next budget window.
        if report
            .usd_cents
            .saturating_add(plan.estimated_usd_cents_per_call)
            > plan.max_usd_cents_per_run
        {
            report.buckets_pending += 1;
            continue;
        }
        let existing = backend
            .lookup_existing(&bucket.repo, &bucket.year_week, &bucket.top_topic)
            .await
            .map_err(DigestError::Backend)?;
        if existing.is_some() && !plan.rebuild {
            report.already_digested += 1;
            report.outcomes.push(BucketOutcome {
                key: bucket.key(),
                digested: false,
                already_digested: true,
                error: None,
            });
            continue;
        }
        if plan.dry_run {
            report.outcomes.push(BucketOutcome {
                key: bucket.key(),
                digested: false,
                already_digested: false,
                error: None,
            });
            report.buckets_pending += 1;
            continue;
        }
        match digest_one(&bucket, backend).await {
            Ok(usd) => {
                report.buckets_done += 1;
                report.usd_cents = report.usd_cents.saturating_add(usd);
                report.outcomes.push(BucketOutcome {
                    key: bucket.key(),
                    digested: true,
                    already_digested: false,
                    error: None,
                });
            }
            Err(reason) => {
                report.outcomes.push(BucketOutcome {
                    key: bucket.key(),
                    digested: false,
                    already_digested: false,
                    error: Some(reason),
                });
            }
        }
    }
    Ok(report)
}

async fn digest_one(bucket: &Bucket, backend: &dyn DigestBackend) -> Result<u64, String> {
    let digest = backend.summarize(bucket).await?;
    let usd = digest.usd_cents;
    let event_id = backend.persist_digest(bucket, &digest).await?;
    backend
        .tag_source_turns(&event_id, &bucket.event_ids)
        .await?;
    Ok(usd)
}

// ---- in-memory test double ----------------------------------------

/// Empty `MemoryDigestBackend` test double. Records every backend
/// call so tests can assert call ordering + payloads.
#[derive(Debug, Default)]
pub struct MemoryDigestBackend {
    inner: tokio::sync::Mutex<MemoryDigestState>,
}

#[derive(Debug, Default)]
struct MemoryDigestState {
    pub existing: BTreeMap<String, String>,
    pub summaries: Vec<(String, String, String, usize)>,
    pub persisted: Vec<(String, String, String, String)>,
    pub tag_calls: Vec<(String, Vec<String>)>,
    pub summary_override: Option<DigestResult>,
    pub fail_on: Option<&'static str>,
}

impl MemoryDigestBackend {
    /// Empty state.
    pub fn new() -> Self {
        Self::default()
    }
    /// Pre-populate an "existing digest" for a bucket key so the
    /// idempotence guard tests fire predictably.
    pub async fn pre_existing(
        &self,
        repo: &str,
        year_week: &str,
        top_topic: &str,
        digest_id: &str,
    ) {
        let key = format!("{repo}|{year_week}|{top_topic}");
        self.inner
            .lock()
            .await
            .existing
            .insert(key, digest_id.to_string());
    }
    /// Force `summarize` to return a fixed result.
    pub async fn set_summary(&self, result: DigestResult) {
        self.inner.lock().await.summary_override = Some(result);
    }
    /// Inject a one-shot failure on `step` (`lookup` / `summarize` /
    /// `persist` / `tag`).
    pub async fn inject_failure(&self, step: &'static str) {
        self.inner.lock().await.fail_on = Some(step);
    }
    /// Snapshot recorded summarize calls (returns `(repo, week,
    /// topic, event_count)` per call).
    pub async fn summaries(&self) -> Vec<(String, String, String, usize)> {
        self.inner.lock().await.summaries.clone()
    }
    /// Snapshot persisted digests (returns `(repo, week, topic,
    /// digest_id)`).
    pub async fn persisted(&self) -> Vec<(String, String, String, String)> {
        self.inner.lock().await.persisted.clone()
    }
    /// Snapshot tag-source-turns calls.
    pub async fn tag_calls(&self) -> Vec<(String, Vec<String>)> {
        self.inner.lock().await.tag_calls.clone()
    }
}

#[async_trait]
impl DigestBackend for MemoryDigestBackend {
    async fn lookup_existing(
        &self,
        repo: &str,
        year_week: &str,
        top_topic: &str,
    ) -> Result<Option<String>, String> {
        let mut s = self.inner.lock().await;
        if s.fail_on == Some("lookup") {
            s.fail_on = None;
            return Err("synthetic-lookup-failure".into());
        }
        let key = format!("{repo}|{year_week}|{top_topic}");
        Ok(s.existing.get(&key).cloned())
    }
    async fn summarize(&self, bucket: &Bucket) -> Result<DigestResult, String> {
        let mut s = self.inner.lock().await;
        if s.fail_on == Some("summarize") {
            s.fail_on = None;
            return Err("synthetic-summarize-failure".into());
        }
        s.summaries.push((
            bucket.repo.clone(),
            bucket.year_week.clone(),
            bucket.top_topic.clone(),
            bucket.event_ids.len(),
        ));
        Ok(s.summary_override.clone().unwrap_or_else(|| DigestResult {
            body: format!(
                "[digest:{}|{}|{}] {} turns",
                bucket.repo,
                bucket.year_week,
                bucket.top_topic,
                bucket.event_ids.len()
            ),
            tokens_in: 1024,
            tokens_out: 256,
            usd_cents: 5,
        }))
    }
    async fn persist_digest(
        &self,
        bucket: &Bucket,
        _digest: &DigestResult,
    ) -> Result<String, String> {
        let mut s = self.inner.lock().await;
        if s.fail_on == Some("persist") {
            s.fail_on = None;
            return Err("synthetic-persist-failure".into());
        }
        let digest_id = format!(
            "01DIGEST{}-{}-{}",
            bucket.repo, bucket.year_week, bucket.top_topic
        );
        s.persisted.push((
            bucket.repo.clone(),
            bucket.year_week.clone(),
            bucket.top_topic.clone(),
            digest_id.clone(),
        ));
        // Pre-populate existing so the idempotence test on a re-run
        // returns the persisted id.
        let key = format!("{}|{}|{}", bucket.repo, bucket.year_week, bucket.top_topic);
        s.existing.insert(key, digest_id.clone());
        Ok(digest_id)
    }
    async fn tag_source_turns(
        &self,
        digest_event_id: &str,
        event_ids: &[String],
    ) -> Result<(), String> {
        let mut s = self.inner.lock().await;
        if s.fail_on == Some("tag") {
            s.fail_on = None;
            return Err("synthetic-tag-failure".into());
        }
        s.tag_calls
            .push((digest_event_id.to_string(), event_ids.to_vec()));
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

    fn turn(id: &str, repo: &str, age_days: i64, topic: &str) -> Turn {
        Turn {
            event_id: id.to_string(),
            repo: repo.to_string(),
            occurred_at: now() - Duration::days(age_days),
            top_topic: topic.to_string(),
            summarized_by: None,
        }
    }

    #[test]
    fn iso_year_week_uses_rfc_label() {
        // 2026-04-29 is ISO 2026-W18.
        let ts = DateTime::parse_from_rfc3339("2026-04-29T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(iso_year_week(ts), "2026-W18");
    }

    #[test]
    fn bucket_key_is_pipe_joined() {
        let b = Bucket {
            repo: "alpha".into(),
            year_week: "2026-W17".into(),
            top_topic: "auth".into(),
            event_ids: vec![],
        };
        assert_eq!(b.key(), "alpha|2026-W17|auth");
    }

    #[test]
    fn bucketize_groups_old_turns_by_repo_week_topic() {
        let plan = DigestPlan::default_for(now());
        let mut turns = Vec::new();
        for i in 0..6 {
            turns.push(turn(&format!("01A{i}"), "alpha", 50, "auth"));
        }
        for i in 0..6 {
            turns.push(turn(&format!("01B{i}"), "alpha", 50, "ingestion"));
        }
        let buckets = bucketize(&plan, turns);
        // Two buckets — one per topic.
        assert_eq!(buckets.len(), 2);
        let topics: Vec<&str> = buckets.iter().map(|b| b.top_topic.as_str()).collect();
        assert!(topics.contains(&"auth"));
        assert!(topics.contains(&"ingestion"));
    }

    #[test]
    fn bucketize_filters_below_min_size() {
        let plan = DigestPlan::default_for(now());
        // 4 turns — below the default min of 5.
        let turns: Vec<Turn> = (0..4)
            .map(|i| turn(&format!("01{i}"), "alpha", 50, "auth"))
            .collect();
        let buckets = bucketize(&plan, turns);
        assert!(buckets.is_empty());
    }

    #[test]
    fn bucketize_excludes_fresh_turns() {
        let plan = DigestPlan::default_for(now());
        let mut turns = Vec::new();
        for i in 0..6 {
            // Only 5 days old — under the 30-day threshold.
            turns.push(turn(&format!("01F{i}"), "alpha", 5, "auth"));
        }
        assert!(bucketize(&plan, turns).is_empty());
    }

    #[test]
    fn bucketize_excludes_already_digested_turns() {
        let plan = DigestPlan::default_for(now());
        let mut turns = Vec::new();
        for i in 0..6 {
            let mut t = turn(&format!("01D{i}"), "alpha", 50, "auth");
            t.summarized_by = Some("01EXISTINGDIGEST".to_string());
            turns.push(t);
        }
        assert!(bucketize(&plan, turns).is_empty());
    }

    #[tokio::test]
    async fn run_persists_one_digest_per_bucket_in_call_order() {
        let backend = MemoryDigestBackend::new();
        let plan = DigestPlan::default_for(now());
        let mut turns = Vec::new();
        for i in 0..12 {
            turns.push(turn(&format!("01T{i}"), "alpha", 60, "auth"));
        }
        let report = run_turn_digest(&plan, &backend, turns).await.unwrap();
        assert_eq!(report.examined, 1);
        assert_eq!(report.buckets_done, 1);
        assert_eq!(report.already_digested, 0);
        assert_eq!(report.buckets_pending, 0);
        // Cross-step ordering: summarize → persist → tag.
        assert_eq!(backend.summaries().await.len(), 1);
        assert_eq!(backend.persisted().await.len(), 1);
        let tag_calls = backend.tag_calls().await;
        assert_eq!(tag_calls.len(), 1);
        // The tag call references all 12 source ids.
        assert_eq!(tag_calls[0].1.len(), 12);
        // Spend tracked.
        assert!(report.usd_cents > 0);
    }

    #[tokio::test]
    async fn idempotent_re_run_does_not_call_summarize() {
        let backend = MemoryDigestBackend::new();
        backend
            .pre_existing("alpha", "2026-W11", "auth", "01EXISTING")
            .await;
        let plan = DigestPlan::default_for(now());
        // 6 turns @ 70 d → ISO 2026-W08 of those weeks; but pin
        // the test bucket to W11 by aging the turns 49 days (W12)
        // and pre-populating the matching slot. We use a fixed
        // age so iso_year_week resolves deterministically.
        let week = iso_year_week(now() - Duration::days(60));
        backend
            .pre_existing("alpha", &week, "auth", "01EXISTING")
            .await;
        let turns: Vec<Turn> = (0..6)
            .map(|i| turn(&format!("01T{i}"), "alpha", 60, "auth"))
            .collect();
        let report = run_turn_digest(&plan, &backend, turns).await.unwrap();
        assert_eq!(report.buckets_done, 0);
        assert_eq!(report.already_digested, 1);
        // Classifier was NOT called.
        assert!(backend.summaries().await.is_empty());
        assert!(backend.persisted().await.is_empty());
    }

    #[tokio::test]
    async fn rebuild_flag_re_summarises_existing_buckets() {
        let backend = MemoryDigestBackend::new();
        let week = iso_year_week(now() - Duration::days(60));
        backend
            .pre_existing("alpha", &week, "auth", "01EXISTING")
            .await;
        let mut plan = DigestPlan::default_for(now());
        plan.rebuild = true;
        let turns: Vec<Turn> = (0..6)
            .map(|i| turn(&format!("01T{i}"), "alpha", 60, "auth"))
            .collect();
        let report = run_turn_digest(&plan, &backend, turns).await.unwrap();
        assert_eq!(report.buckets_done, 1);
        assert_eq!(report.already_digested, 0);
        assert_eq!(backend.summaries().await.len(), 1);
    }

    #[tokio::test]
    async fn dry_run_records_pending_without_calling_classifier() {
        let backend = MemoryDigestBackend::new();
        let mut plan = DigestPlan::default_for(now());
        plan.dry_run = true;
        let turns: Vec<Turn> = (0..6)
            .map(|i| turn(&format!("01T{i}"), "alpha", 60, "auth"))
            .collect();
        let report = run_turn_digest(&plan, &backend, turns).await.unwrap();
        assert_eq!(report.buckets_done, 0);
        assert_eq!(report.buckets_pending, 1);
        assert!(backend.summaries().await.is_empty());
    }

    #[tokio::test]
    async fn budget_ceiling_stops_run_cleanly() {
        let backend = MemoryDigestBackend::new();
        let mut plan = DigestPlan::default_for(now());
        plan.max_usd_cents_per_run = 5; // budget for exactly one call
        plan.estimated_usd_cents_per_call = 5;
        // 12 turns across 2 different repos so bucketize returns 2.
        let mut turns = Vec::new();
        for i in 0..6 {
            turns.push(turn(&format!("01A{i}"), "alpha", 60, "auth"));
        }
        for i in 0..6 {
            turns.push(turn(&format!("01B{i}"), "beta", 60, "auth"));
        }
        let report = run_turn_digest(&plan, &backend, turns).await.unwrap();
        assert_eq!(report.examined, 2);
        assert_eq!(report.buckets_done, 1);
        assert_eq!(report.buckets_pending, 1);
    }

    #[tokio::test]
    async fn per_bucket_failure_records_error_and_continues() {
        let backend = MemoryDigestBackend::new();
        backend.inject_failure("summarize").await;
        let plan = DigestPlan::default_for(now());
        // 12 turns split across two buckets — first one trips
        // synthetic-summarize-failure, second proceeds normally.
        let mut turns = Vec::new();
        for i in 0..6 {
            turns.push(turn(&format!("01A{i}"), "alpha", 60, "auth"));
        }
        for i in 0..6 {
            turns.push(turn(&format!("01B{i}"), "beta", 60, "auth"));
        }
        let report = run_turn_digest(&plan, &backend, turns).await.unwrap();
        assert_eq!(report.examined, 2);
        assert_eq!(report.buckets_done, 1);
        // The first bucket recorded the synthetic error.
        let errored: Vec<&BucketOutcome> = report
            .outcomes
            .iter()
            .filter(|o| o.error.is_some())
            .collect();
        assert_eq!(errored.len(), 1);
    }

    #[test]
    fn report_turn_digest_json_round_trips() {
        let r = DigestReport {
            buckets_done: 7,
            already_digested: 2,
            buckets_pending: 1,
            usd_cents: 35,
            ..Default::default()
        };
        let json = r.turn_digest_json();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["buckets_done"], 7);
        assert_eq!(parsed["already_digested"], 2);
        assert_eq!(parsed["buckets_pending"], 1);
        assert_eq!(parsed["usd_cents"], 35);
    }

    #[test]
    fn plan_default_uses_spec_thresholds() {
        let plan = DigestPlan::default_for(now());
        assert_eq!(plan.digest_after_days, 30);
        assert_eq!(plan.min_bucket_size, 5);
        assert_eq!(plan.max_usd_cents_per_run, 500);
        assert!(!plan.dry_run);
        assert!(!plan.rebuild);
    }
}
