//! Phase9a — Vectorizer tier-transition sweeper.
//!
//! Spec 02 §"Quantization & tier sweep" promises a daily sweep that
//! re-encodes Vectorizer records FP32 → PQ at 30 days and PQ →
//! Binary at 365 days. This crate implements the sweep contract.
//!
//! Library shape (the binary path lives in `cortex-cli`'s
//! `cortex-ops` bin so it shares the existing operator surface):
//!
//! - [`SweepPlan`] — declarative inputs (now, age thresholds,
//!   batch size, hot-collection allow-list).
//! - [`TierPair`] — one source → destination collection pair plus
//!   the cutoff age (`now - age_days`).
//! - [`VectorizerOps`] trait — minimal surface the sweep needs:
//!   list-by-cutoff, get-by-id, upsert, delete. Production wires
//!   the live SDK; tests use [`MemoryVectorizerOps`] for fast,
//!   deterministic round-trips.
//! - [`run_sweep`] — orchestrator that runs the plan, writes the
//!   `retention_sweeps` row via [`cortex_storage::MetadataStore`],
//!   and returns a [`SweepReport`].
//!
//! The sweep is idempotent: every step is a re-encode + upsert into
//! the destination + delete from the source. Re-running the same
//! plan after success is a no-op because the destination already
//! holds the record (`get_by_id` short-circuits).

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod cas_vacuum;
pub mod parquet_rollup;
pub mod pii_enforce;
pub mod turn_digest;

use std::collections::BTreeMap;

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Quantization tier name used in the canonical collection names
/// (`cortex.<kind>.<tier>`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    /// Full-precision vectors (fresh data).
    Fp32,
    /// Product-quantised vectors (warm tier, ~30+ days old).
    Pq,
    /// Binary-quantised vectors (cold tier, ~365+ days old).
    Binary,
}

impl Tier {
    /// Stable string identifier (matches the JSON enum tag).
    pub fn as_str(self) -> &'static str {
        match self {
            Tier::Fp32 => "fp32",
            Tier::Pq => "pq",
            Tier::Binary => "binary",
        }
    }
}

/// Canonical kind whose collection participates in the sweep. Hot
/// kinds (`decision`, `analysis`, `memory`, `law`) deliberately do
/// NOT appear here — they are always-hot per the spec.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SweepKind {
    /// `cortex.turn.{fp32,pq}` → `cortex.cold.binary`.
    Turn,
    /// `cortex.tool_call.{fp32,pq}` → `cortex.cold.binary`.
    ToolCall,
    /// `cortex.code_chunk.{fp32,pq}` → `cortex.cold.binary`.
    CodeChunk,
}

impl SweepKind {
    /// Stable string identifier.
    pub fn as_str(self) -> &'static str {
        match self {
            SweepKind::Turn => "turn",
            SweepKind::ToolCall => "tool_call",
            SweepKind::CodeChunk => "code_chunk",
        }
    }
    /// Source collection name for [`Tier::Fp32`] / [`Tier::Pq`]
    /// reads.
    pub fn collection_for(self, tier: Tier) -> String {
        match tier {
            Tier::Binary => "cortex.cold.binary".to_string(),
            other => format!("cortex.{}.{}", self.as_str(), other.as_str()),
        }
    }
}

/// One source → destination tier pair the sweep evaluates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TierPair {
    /// Canonical kind (turn / tool_call / code_chunk).
    pub kind: SweepKind,
    /// Source tier (records older than the cutoff are removed).
    pub from_tier: Tier,
    /// Destination tier (records older than the cutoff are written).
    pub to_tier: Tier,
    /// Age in days at which a record crosses the boundary.
    pub age_days: i64,
}

impl TierPair {
    /// Convenience name `<kind>:<from>-><to>` used as the JSON key
    /// in `tier_transitions_json` and the dashboard breakdown.
    pub fn pair_key(&self) -> String {
        format!(
            "{}:{}->{}",
            self.kind.as_str(),
            self.from_tier.as_str(),
            self.to_tier.as_str()
        )
    }
    /// Cutoff timestamp = `now - age_days`. Records whose
    /// `occurred_at` is strictly older than this MUST move.
    pub fn cutoff(&self, now: DateTime<Utc>) -> DateTime<Utc> {
        now - Duration::days(self.age_days)
    }
}

/// Plan inputs — the CLI builds one from defaults + overrides.
#[derive(Debug, Clone)]
pub struct SweepPlan {
    /// Reference time (overridable via `--time-travel`).
    pub now: DateTime<Utc>,
    /// Tier pairs to evaluate, in order.
    pub pairs: Vec<TierPair>,
    /// Maximum records per Vectorizer batch call.
    pub batch_size: u32,
    /// Maximum acceptable error rate before the sweep fails the
    /// run. `0.05` = 5% per spec.
    pub max_error_rate: f64,
    /// `true` skips every Vectorizer mutation (still emits the
    /// transition list so `--dry-run` is observable).
    pub dry_run: bool,
}

impl SweepPlan {
    /// Default plan per spec 02: FP32 → PQ at 30 d for every kind,
    /// PQ → Binary at 365 d. `batch_size = 256`, `max_error_rate =
    /// 0.05`.
    pub fn default_for(now: DateTime<Utc>) -> Self {
        let mut pairs = Vec::new();
        for kind in [SweepKind::Turn, SweepKind::ToolCall, SweepKind::CodeChunk] {
            pairs.push(TierPair {
                kind,
                from_tier: Tier::Fp32,
                to_tier: Tier::Pq,
                age_days: 30,
            });
            pairs.push(TierPair {
                kind,
                from_tier: Tier::Pq,
                to_tier: Tier::Binary,
                age_days: 365,
            });
        }
        Self {
            now,
            pairs,
            batch_size: 256,
            max_error_rate: 0.05,
            dry_run: false,
        }
    }
}

/// One record the sweep observed crossing a tier boundary.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TierTransition {
    /// Envelope id.
    pub event_id: String,
    /// Canonical kind.
    pub kind: String,
    /// Tier the record left.
    pub from_tier: String,
    /// Tier the record landed in.
    pub to_tier: String,
    /// `age_exceeded` for normal flow; future tasks (9d PII) plug
    /// in `pii_redaction` etc.
    pub reason: String,
}

/// Counters returned by [`run_sweep`].
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct SweepReport {
    /// Records observed crossing a boundary, regardless of dry-run.
    pub records_demoted: u64,
    /// Records the source held but the upsert failed for; no delete
    /// is attempted on these.
    pub records_dropped: u64,
    /// Per-pair counts in `<kind>:<from>-><to>` keys.
    pub tier_transitions: BTreeMap<String, u64>,
    /// One row per moved record — used by the CLI to emit
    /// `cortex.events.enriched` events on the bus.
    pub transitions: Vec<TierTransition>,
}

impl SweepReport {
    /// JSON-encoded `tier_transitions` map suitable for
    /// `metadata::finish_retention_sweep(...)`.
    pub fn tier_transitions_json(&self) -> String {
        serde_json::to_string(&self.tier_transitions).unwrap_or_else(|_| "{}".into())
    }

    /// `records_dropped / max(records_demoted + records_dropped, 1)`
    /// — used by [`run_sweep`] to enforce the 5% spec ceiling.
    pub fn error_rate(&self) -> f64 {
        let total = self.records_demoted + self.records_dropped;
        if total == 0 {
            0.0
        } else {
            self.records_dropped as f64 / total as f64
        }
    }
}

/// Errors the sweep returns.
#[derive(Debug, Error)]
pub enum SweepError {
    /// Per-record drop rate exceeded the configured ceiling.
    #[error(
        "error rate {actual:.2} exceeds ceiling {ceiling:.2} (demoted={demoted}, dropped={dropped})"
    )]
    ErrorRateExceeded {
        /// Observed drop rate.
        actual: f64,
        /// Configured ceiling.
        ceiling: f64,
        /// Demoted count when the ceiling tripped.
        demoted: u64,
        /// Dropped count when the ceiling tripped.
        dropped: u64,
    },
    /// Vectorizer client surfaced an error the sweep could not
    /// classify as recoverable.
    #[error("vectorizer: {0}")]
    Vectorizer(String),
}

/// Tier-aware Vectorizer surface the sweep depends on. Production
/// wires the live SDK; tests use [`MemoryVectorizerOps`] for fast,
/// in-process round-trips.
#[async_trait]
pub trait VectorizerOps: Send + Sync {
    /// Return every record in `collection` whose `occurred_at` is
    /// strictly older than `cutoff`. The sweep paginates by
    /// `batch_size` upstream of this call.
    async fn list_older_than(
        &self,
        collection: &str,
        cutoff: DateTime<Utc>,
        batch_size: u32,
    ) -> Result<Vec<RecordRef>, SweepError>;
    /// `true` when `dest` already holds an entry under `event_id`.
    /// Sweep MUST short-circuit on `Ok(true)` to keep idempotence.
    async fn dest_has(&self, dest_collection: &str, event_id: &str) -> Result<bool, SweepError>;
    /// Re-encode `record` into the `dest_tier`'s representation and
    /// upsert into `dest_collection`. Returns the new vector size in
    /// bytes for telemetry (currently unused by the sweep but
    /// surfaced for tests and the dashboard).
    async fn reencode_and_upsert(
        &self,
        dest_collection: &str,
        dest_tier: Tier,
        record: &RecordRef,
    ) -> Result<u32, SweepError>;
    /// Remove `event_id` from `source_collection`. Called only after
    /// the destination upsert succeeded.
    async fn delete_from(&self, source_collection: &str, event_id: &str) -> Result<(), SweepError>;
}

/// Minimal record shape the sweep moves. The Vectorizer SDK has a
/// richer payload — production [`VectorizerOps`] adapts the SDK row
/// into this shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordRef {
    /// Envelope id.
    pub event_id: String,
    /// Canonical kind.
    pub kind: String,
    /// `occurred_at` parsed from the record's metadata.
    pub occurred_at: DateTime<Utc>,
    /// Optional pre-encoded vector bytes — production loaders fill
    /// this lazily; tests carry the bytes inline.
    pub bytes: Vec<u8>,
}

/// Run the sweep against `ops`. Returns the [`SweepReport`] on
/// success, [`SweepError::ErrorRateExceeded`] when the per-record
/// drop rate exceeds the configured ceiling.
pub async fn run_sweep(
    plan: &SweepPlan,
    ops: &dyn VectorizerOps,
) -> Result<SweepReport, SweepError> {
    let mut report = SweepReport::default();
    for pair in &plan.pairs {
        let cutoff = pair.cutoff(plan.now);
        let source = pair.kind.collection_for(pair.from_tier);
        let dest = pair.kind.collection_for(pair.to_tier);
        let candidates = ops.list_older_than(&source, cutoff, plan.batch_size).await?;
        for record in candidates {
            // Idempotence check — destination already holds it
            // ⇒ this is a re-run, no-op.
            if ops.dest_has(&dest, &record.event_id).await? {
                if !plan.dry_run {
                    // Best-effort source cleanup so a partially-
                    // applied prior run converges. Failures here
                    // are demote-class drops.
                    if ops.delete_from(&source, &record.event_id).await.is_err() {
                        report.records_dropped += 1;
                    }
                }
                continue;
            }
            if plan.dry_run {
                report.records_demoted += 1;
                bump(&mut report.tier_transitions, &pair.pair_key());
                report.transitions.push(TierTransition {
                    event_id: record.event_id.clone(),
                    kind: record.kind.clone(),
                    from_tier: pair.from_tier.as_str().to_string(),
                    to_tier: pair.to_tier.as_str().to_string(),
                    reason: "age_exceeded(dry_run)".into(),
                });
                continue;
            }
            match ops.reencode_and_upsert(&dest, pair.to_tier, &record).await {
                Ok(_) => {
                    if let Err(e) = ops.delete_from(&source, &record.event_id).await {
                        // Source-side delete failure leaves the
                        // record in both collections; treat as a
                        // drop so the operator notices but don't
                        // fail the whole sweep.
                        tracing::warn!(
                            error = %e,
                            event_id = %record.event_id,
                            collection = %source,
                            "retention sweep: source delete failed (record will be re-evaluated next sweep)"
                        );
                        report.records_dropped += 1;
                        continue;
                    }
                    report.records_demoted += 1;
                    bump(&mut report.tier_transitions, &pair.pair_key());
                    report.transitions.push(TierTransition {
                        event_id: record.event_id.clone(),
                        kind: record.kind.clone(),
                        from_tier: pair.from_tier.as_str().to_string(),
                        to_tier: pair.to_tier.as_str().to_string(),
                        reason: "age_exceeded".into(),
                    });
                }
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        event_id = %record.event_id,
                        from = %source,
                        to = %dest,
                        "retention sweep: re-encode/upsert failed"
                    );
                    report.records_dropped += 1;
                }
            }
        }
    }
    if report.error_rate() > plan.max_error_rate {
        return Err(SweepError::ErrorRateExceeded {
            actual: report.error_rate(),
            ceiling: plan.max_error_rate,
            demoted: report.records_demoted,
            dropped: report.records_dropped,
        });
    }
    Ok(report)
}

fn bump(map: &mut BTreeMap<String, u64>, key: &str) {
    *map.entry(key.to_string()).or_insert(0) += 1;
}

/// Mint a new sweep ULID using the `ulid` crate.
pub fn new_sweep_id() -> String {
    ulid::Ulid::new().to_string()
}

// ---- in-memory test double --------------------------------------

/// In-memory [`VectorizerOps`] that stores `RecordRef` rows per
/// collection. The sweep tests drive this end-to-end without any
/// real Vectorizer.
#[derive(Debug, Default)]
pub struct MemoryVectorizerOps {
    inner: tokio::sync::Mutex<MemoryStore>,
}

#[derive(Debug, Default)]
struct MemoryStore {
    by_collection: BTreeMap<String, Vec<RecordRef>>,
    /// When `Some`, every call to `reencode_and_upsert` for this
    /// collection returns `SweepError::Vectorizer(reason)` once
    /// before succeeding. Lets tests exercise the dropped-record
    /// path deterministically.
    inject_upsert_error_once: BTreeMap<String, String>,
}

impl MemoryVectorizerOps {
    /// Empty store.
    pub fn new() -> Self {
        Self::default()
    }
    /// Seed `collection` with `records`. Used by tests + the dry-run
    /// integration smoke.
    pub async fn seed(&self, collection: &str, records: Vec<RecordRef>) {
        let mut inner = self.inner.lock().await;
        inner
            .by_collection
            .entry(collection.to_string())
            .or_default()
            .extend(records);
    }
    /// Snapshot the rows in `collection` for assertion.
    pub async fn snapshot(&self, collection: &str) -> Vec<RecordRef> {
        let inner = self.inner.lock().await;
        inner
            .by_collection
            .get(collection)
            .cloned()
            .unwrap_or_default()
    }
    /// Inject a one-shot upsert error so tests can drive the
    /// dropped-record path deterministically.
    pub async fn inject_upsert_error_once(&self, collection: &str, reason: impl Into<String>) {
        let mut inner = self.inner.lock().await;
        inner
            .inject_upsert_error_once
            .insert(collection.to_string(), reason.into());
    }
}

#[async_trait]
impl VectorizerOps for MemoryVectorizerOps {
    async fn list_older_than(
        &self,
        collection: &str,
        cutoff: DateTime<Utc>,
        _batch_size: u32,
    ) -> Result<Vec<RecordRef>, SweepError> {
        let inner = self.inner.lock().await;
        Ok(inner
            .by_collection
            .get(collection)
            .map(|rows| {
                rows.iter()
                    .filter(|r| r.occurred_at < cutoff)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default())
    }
    async fn dest_has(&self, dest_collection: &str, event_id: &str) -> Result<bool, SweepError> {
        let inner = self.inner.lock().await;
        Ok(inner
            .by_collection
            .get(dest_collection)
            .map(|rows| rows.iter().any(|r| r.event_id == event_id))
            .unwrap_or(false))
    }
    async fn reencode_and_upsert(
        &self,
        dest_collection: &str,
        _dest_tier: Tier,
        record: &RecordRef,
    ) -> Result<u32, SweepError> {
        let mut inner = self.inner.lock().await;
        if let Some(reason) = inner.inject_upsert_error_once.remove(dest_collection) {
            return Err(SweepError::Vectorizer(reason));
        }
        let bucket = inner.by_collection.entry(dest_collection.to_string()).or_default();
        // Upsert: replace if event_id already present.
        if let Some(existing) = bucket.iter_mut().find(|r| r.event_id == record.event_id) {
            *existing = record.clone();
        } else {
            bucket.push(record.clone());
        }
        Ok(record.bytes.len() as u32)
    }
    async fn delete_from(&self, source_collection: &str, event_id: &str) -> Result<(), SweepError> {
        let mut inner = self.inner.lock().await;
        if let Some(bucket) = inner.by_collection.get_mut(source_collection) {
            bucket.retain(|r| r.event_id != event_id);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(id: &str, kind: SweepKind, age_days: i64, now: DateTime<Utc>) -> RecordRef {
        RecordRef {
            event_id: id.to_string(),
            kind: kind.as_str().to_string(),
            occurred_at: now - Duration::days(age_days),
            bytes: vec![0u8; 16],
        }
    }

    fn fixed_now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-04-29T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    #[test]
    fn collection_for_returns_canonical_names() {
        assert_eq!(
            SweepKind::Turn.collection_for(Tier::Fp32),
            "cortex.turn.fp32"
        );
        assert_eq!(
            SweepKind::ToolCall.collection_for(Tier::Pq),
            "cortex.tool_call.pq"
        );
        // Binary is the cold tier — every kind shares one collection.
        assert_eq!(
            SweepKind::CodeChunk.collection_for(Tier::Binary),
            "cortex.cold.binary"
        );
    }

    #[test]
    fn tier_pair_pair_key_round_trips() {
        let pair = TierPair {
            kind: SweepKind::ToolCall,
            from_tier: Tier::Pq,
            to_tier: Tier::Binary,
            age_days: 365,
        };
        assert_eq!(pair.pair_key(), "tool_call:pq->binary");
    }

    #[test]
    fn tier_pair_cutoff_subtracts_age_days() {
        let now = fixed_now();
        let pair = TierPair {
            kind: SweepKind::Turn,
            from_tier: Tier::Fp32,
            to_tier: Tier::Pq,
            age_days: 30,
        };
        let cutoff = pair.cutoff(now);
        assert_eq!(cutoff, now - Duration::days(30));
    }

    #[test]
    fn default_plan_includes_all_six_pairs() {
        let plan = SweepPlan::default_for(fixed_now());
        let keys: Vec<String> = plan.pairs.iter().map(|p| p.pair_key()).collect();
        assert!(keys.contains(&"turn:fp32->pq".to_string()));
        assert!(keys.contains(&"turn:pq->binary".to_string()));
        assert!(keys.contains(&"tool_call:fp32->pq".to_string()));
        assert!(keys.contains(&"tool_call:pq->binary".to_string()));
        assert!(keys.contains(&"code_chunk:fp32->pq".to_string()));
        assert!(keys.contains(&"code_chunk:pq->binary".to_string()));
        assert_eq!(plan.batch_size, 256);
        assert!((plan.max_error_rate - 0.05).abs() < 1e-9);
    }

    #[test]
    fn report_error_rate_handles_empty_run() {
        let r = SweepReport::default();
        assert_eq!(r.error_rate(), 0.0);
    }

    #[test]
    fn report_error_rate_computes_demoted_plus_dropped() {
        let r = SweepReport {
            records_demoted: 90,
            records_dropped: 10,
            ..Default::default()
        };
        assert!((r.error_rate() - 0.10).abs() < 1e-9);
    }

    #[test]
    fn report_tier_transitions_json_round_trips() {
        let mut t: BTreeMap<String, u64> = BTreeMap::new();
        t.insert("turn:fp32->pq".into(), 4);
        let r = SweepReport {
            records_demoted: 4,
            tier_transitions: t.clone(),
            ..Default::default()
        };
        let json = r.tier_transitions_json();
        let parsed: BTreeMap<String, u64> = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, t);
    }

    #[tokio::test]
    async fn fp32_record_at_31_days_moves_to_pq() {
        let now = fixed_now();
        let ops = MemoryVectorizerOps::new();
        ops.seed(
            "cortex.turn.fp32",
            vec![rec("01TURN", SweepKind::Turn, 31, now)],
        )
        .await;
        let plan = SweepPlan::default_for(now);
        let report = run_sweep(&plan, &ops).await.unwrap();
        assert_eq!(report.records_demoted, 1);
        assert_eq!(report.records_dropped, 0);
        assert_eq!(
            *report.tier_transitions.get("turn:fp32->pq").unwrap_or(&0),
            1
        );
        // Source emptied, destination populated.
        assert!(ops.snapshot("cortex.turn.fp32").await.is_empty());
        assert_eq!(ops.snapshot("cortex.turn.pq").await.len(), 1);
        // The transition list carries the full row for the bus emit.
        let t = &report.transitions[0];
        assert_eq!(t.event_id, "01TURN");
        assert_eq!(t.from_tier, "fp32");
        assert_eq!(t.to_tier, "pq");
        assert_eq!(t.reason, "age_exceeded");
    }

    #[tokio::test]
    async fn pq_record_at_366_days_moves_to_binary() {
        let now = fixed_now();
        let ops = MemoryVectorizerOps::new();
        ops.seed(
            "cortex.tool_call.pq",
            vec![rec("01TC", SweepKind::ToolCall, 366, now)],
        )
        .await;
        let plan = SweepPlan::default_for(now);
        let report = run_sweep(&plan, &ops).await.unwrap();
        assert_eq!(report.records_demoted, 1);
        assert_eq!(
            *report
                .tier_transitions
                .get("tool_call:pq->binary")
                .unwrap_or(&0),
            1
        );
        assert!(ops.snapshot("cortex.tool_call.pq").await.is_empty());
        assert_eq!(ops.snapshot("cortex.cold.binary").await.len(), 1);
    }

    #[tokio::test]
    async fn fresh_record_under_threshold_is_left_alone() {
        let now = fixed_now();
        let ops = MemoryVectorizerOps::new();
        ops.seed(
            "cortex.turn.fp32",
            vec![rec("01FRESH", SweepKind::Turn, 5, now)],
        )
        .await;
        let plan = SweepPlan::default_for(now);
        let report = run_sweep(&plan, &ops).await.unwrap();
        assert_eq!(report.records_demoted, 0);
        assert_eq!(ops.snapshot("cortex.turn.fp32").await.len(), 1);
        assert!(ops.snapshot("cortex.turn.pq").await.is_empty());
    }

    #[tokio::test]
    async fn idempotent_re_run_is_a_no_op() {
        let now = fixed_now();
        let ops = MemoryVectorizerOps::new();
        // Pre-populate the destination to mimic a successful first run.
        let r = rec("01DONE", SweepKind::Turn, 31, now);
        ops.seed("cortex.turn.pq", vec![r.clone()]).await;
        // Source still has the same id (mid-flight crash before
        // delete) — the sweep should clean it up without a second
        // upsert.
        ops.seed("cortex.turn.fp32", vec![r.clone()]).await;
        let plan = SweepPlan::default_for(now);
        let report = run_sweep(&plan, &ops).await.unwrap();
        // No new demotion — the destination already had the row.
        assert_eq!(report.records_demoted, 0);
        // Source-side cleanup proceeded.
        assert!(ops.snapshot("cortex.turn.fp32").await.is_empty());
        // Destination unchanged.
        assert_eq!(ops.snapshot("cortex.turn.pq").await.len(), 1);
    }

    #[tokio::test]
    async fn dry_run_records_demoted_but_does_not_mutate_collections() {
        let now = fixed_now();
        let ops = MemoryVectorizerOps::new();
        ops.seed(
            "cortex.turn.fp32",
            vec![rec("01DRY", SweepKind::Turn, 31, now)],
        )
        .await;
        let mut plan = SweepPlan::default_for(now);
        plan.dry_run = true;
        let report = run_sweep(&plan, &ops).await.unwrap();
        assert_eq!(report.records_demoted, 1);
        assert_eq!(report.transitions[0].reason, "age_exceeded(dry_run)");
        // Source untouched, destination empty — `--dry-run` MUST be
        // observe-only.
        assert_eq!(ops.snapshot("cortex.turn.fp32").await.len(), 1);
        assert!(ops.snapshot("cortex.turn.pq").await.is_empty());
    }

    #[tokio::test]
    async fn upsert_error_counts_as_dropped_under_ceiling() {
        let now = fixed_now();
        let ops = MemoryVectorizerOps::new();
        // Three eligible records — one upsert will fail (one-shot
        // injection). Drop rate = 1/3 ≈ 33%, above the default 5%
        // ceiling, so the sweep MUST surface ErrorRateExceeded.
        ops.seed(
            "cortex.turn.fp32",
            vec![
                rec("01A", SweepKind::Turn, 31, now),
                rec("01B", SweepKind::Turn, 31, now),
                rec("01C", SweepKind::Turn, 31, now),
            ],
        )
        .await;
        ops.inject_upsert_error_once("cortex.turn.pq", "synthetic-failure")
            .await;
        let plan = SweepPlan::default_for(now);
        match run_sweep(&plan, &ops).await {
            Err(SweepError::ErrorRateExceeded {
                dropped, demoted, ..
            }) => {
                assert_eq!(dropped, 1);
                assert_eq!(demoted, 2);
            }
            other => panic!("expected ErrorRateExceeded, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn upsert_error_below_ceiling_succeeds() {
        let now = fixed_now();
        let ops = MemoryVectorizerOps::new();
        // 21 eligible records, 1 fails — drop rate ~4.7%, below the
        // 5% default ceiling.
        let mut seed = Vec::new();
        for i in 0..21 {
            seed.push(rec(&format!("01{i:020}"), SweepKind::Turn, 31, now));
        }
        ops.seed("cortex.turn.fp32", seed).await;
        ops.inject_upsert_error_once("cortex.turn.pq", "transient")
            .await;
        let plan = SweepPlan::default_for(now);
        let report = run_sweep(&plan, &ops).await.unwrap();
        assert_eq!(report.records_dropped, 1);
        assert_eq!(report.records_demoted, 20);
    }

    #[test]
    fn new_sweep_id_is_26_chars_ulid() {
        let a = new_sweep_id();
        let b = new_sweep_id();
        assert_eq!(a.len(), 26);
        assert_eq!(b.len(), 26);
        assert_ne!(a, b);
    }

    #[test]
    fn tier_round_trips_via_serde() {
        let json = serde_json::to_string(&Tier::Pq).unwrap();
        assert_eq!(json, "\"pq\"");
        let parsed: Tier = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, Tier::Pq);
    }
}
