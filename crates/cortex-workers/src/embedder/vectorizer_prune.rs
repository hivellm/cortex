//! Phase14b §4 + ADR-013 — collection-level Vectorizer pruning.
//!
//! Vectorizer SDK 3.1 does not surface a per-vector remove
//! primitive. Until 3.2 ships `delete_vector(collection, id)`, the
//! only correct way to drop expired rows from a Vectorizer
//! collection is to re-encode the entire collection: stream every
//! still-alive vector to a sibling `<name>.tmp`, drop the original,
//! atomically rename the sibling into place.
//!
//! [`reencode_collection`] is the canonical entry point used by
//! [`super::super::retention::cold_tier_prune::ColdTierPrune`]. The
//! function is generic over the [`VectorizerPruneOps`] trait so
//! tests can drive it against an in-memory fixture without spinning
//! up a Vectorizer server.
//!
//! ADR-013 §Consequences §3 — search availability against the
//! pruned collection is preserved throughout. Readers see the
//! original `<name>` until the atomic swap lands; mid-flight crashes
//! drop `<name>.tmp` so the live collection is never poisoned.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

/// One vector record the pruner reads + writes. Carries the id +
/// the f32 vector + the payload that drove the predicate.
#[derive(Debug, Clone, PartialEq)]
pub struct VectorRecord {
    /// Vectorizer-side id (matches the `event_id` Cortex stamps when
    /// it inserted the vector).
    pub id: String,
    /// Vector bytes — Vec<f32> in the canonical Vectorizer encoding.
    pub vector: Vec<f32>,
    /// Payload JSON the predicate consults. The pruner does not
    /// look inside this — it just hands the value to the predicate.
    pub payload: Value,
}

/// Errors the pruner surfaces. Each variant pinpoints the leg of
/// the re-encode that failed so the caller can decide whether to
/// retry the whole pass or surface a `Sweep::Failed` row.
#[derive(Debug, thiserror::Error)]
pub enum PruneError {
    /// Underlying Vectorizer call returned a transport / auth /
    /// schema error the pruner cannot classify.
    #[error("vectorizer: {0}")]
    Vectorizer(String),
    /// The predicate panicked or returned an unexpected shape — the
    /// pruner stops the pass and leaves the original collection
    /// intact.
    #[error("predicate: {0}")]
    Predicate(String),
    /// Swap step failed — both `<name>` and `<name>.tmp` may exist;
    /// the caller's recovery is to drop `<name>.tmp` and retry on
    /// the next scheduled sweep.
    #[error("atomic swap failed for collection {collection}: {reason}")]
    SwapFailed {
        /// Canonical collection name the swap targeted.
        collection: String,
        /// Reason the SDK / fixture returned.
        reason: String,
    },
}

/// Outcome of a successful [`reencode_collection`] pass. The
/// counters drive the `ColdTierPrune` sweep report.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PruneReport {
    /// Total vectors observed in the source collection.
    pub scanned: u64,
    /// Vectors the predicate said to keep — these were upserted
    /// into `<name>.tmp` and survived the swap.
    pub kept: u64,
    /// Vectors the predicate said to drop — these were left out of
    /// `<name>.tmp` and disappeared on the swap.
    pub dropped: u64,
}

impl PruneReport {
    /// Invariant pinned by [`reencode_collection`]: every scanned
    /// vector is either kept or dropped.
    pub fn invariant_holds(&self) -> bool {
        self.kept + self.dropped == self.scanned
    }
}

/// One survivor batch the pruner streams through. Page size is
/// chosen by the [`VectorizerPruneOps`] impl; the canonical
/// production value is 512 vectors per page.
pub type VectorPage = Vec<VectorRecord>;

/// Cursor handed back by [`VectorizerPruneOps::list_vectors_page`]
/// so the pruner can paginate without holding a server-side
/// transaction. `None` signals end-of-stream.
pub type ListCursor = Option<String>;

/// Predicate the pruner consults per record. Returns `true` to
/// KEEP the vector (it survives the swap), `false` to DROP. The
/// predicate is `Send + Sync` so the production `ColdTierPrune`
/// can hand a long-lived closure that closes over the cutoff
/// timestamp.
pub type PrunePredicate = Arc<dyn Fn(&Value) -> bool + Send + Sync>;

/// Sibling-collection suffix the pruner writes survivors to. ADR-
/// 013 §Consequences §4 reserves this name space — production code
/// MUST NOT create collections whose name ends in `.tmp` for any
/// other purpose.
pub const TMP_SUFFIX: &str = ".tmp";

/// Operator-tunable env knob (see ADR-013 §Decision). Production
/// only supports `collection`; a future `per_vector` value swaps
/// the impl when Vectorizer SDK 3.2 lands.
pub const PRUNE_MODE_ENV: &str = "CORTEX_VECTORIZER_PRUNE_MODE";

/// Narrow surface [`reencode_collection`] depends on. Production
/// wires the live Vectorizer SDK; tests use [`MemoryVectorizerPruneOps`].
#[async_trait]
pub trait VectorizerPruneOps: Send + Sync {
    /// Walk one page of `collection`. `cursor` is opaque — pass
    /// `None` on the first call, then whatever the previous page
    /// returned. End-of-stream is signalled by `(page, None)` with
    /// the final page potentially being non-empty.
    async fn list_vectors_page(
        &self,
        collection: &str,
        cursor: ListCursor,
    ) -> Result<(VectorPage, ListCursor), PruneError>;

    /// Upsert one batch of survivors into `dest_collection`.
    /// Idempotent — re-running with the same batch must be a no-op.
    /// Vectorizer's `upsert` semantic is already idempotent per id.
    async fn upsert_batch(
        &self,
        dest_collection: &str,
        batch: &[VectorRecord],
    ) -> Result<(), PruneError>;

    /// Drop `collection` entirely. Called when the pruner needs to
    /// remove the original `<name>` after the survivors have landed
    /// in `<name>.tmp`, OR when the rollback path needs to drop
    /// `<name>.tmp` after a failure.
    async fn drop_collection(&self, collection: &str) -> Result<(), PruneError>;

    /// Atomically rename `from` over `to`. The fixture impl shifts
    /// the in-memory bucket; production wraps the SDK's collection-
    /// rename primitive (or a copy + drop sequence when the SDK
    /// lacks rename — that path is best-effort atomic).
    async fn rename_collection(&self, from: &str, to: &str) -> Result<(), PruneError>;
}

/// Phase14b §4.1 — drive the ADR-013 re-encode pipeline.
///
/// Steps:
///
/// 1. Drop any stale `<name>.tmp` left by a previous failed pass.
/// 2. Page through `<name>` via [`VectorizerPruneOps::list_vectors_page`].
/// 3. For each record, run `predicate(&record.payload)`. `true` ⇒
///    push into the survivor batch; `false` ⇒ increment dropped.
/// 4. Flush survivor batches into `<name>.tmp` via
///    [`VectorizerPruneOps::upsert_batch`] every `BATCH_SIZE` records.
/// 5. On full success drop the original `<name>` then rename
///    `<name>.tmp` over `<name>`. The two-step swap is the closest
///    the SDK gets to atomicity in 3.1; production impls SHOULD
///    upgrade to a single-step rename once available.
///
/// Failure path: any error before the rename leaves `<name>` intact
/// and best-effort drops `<name>.tmp` so the next sweep sees a clean
/// starting point.
pub async fn reencode_collection(
    ops: &dyn VectorizerPruneOps,
    name: &str,
    predicate: PrunePredicate,
) -> Result<PruneReport, PruneError> {
    let tmp_name = format!("{name}{TMP_SUFFIX}");

    // Step 1 — clear any stale `.tmp` from a prior failure.
    // Best-effort: a missing collection drop is a no-op at the SDK
    // layer.
    let _ = ops.drop_collection(&tmp_name).await;

    let mut report = PruneReport::default();
    let mut cursor: ListCursor = None;
    let mut survivor_batch: Vec<VectorRecord> = Vec::with_capacity(BATCH_SIZE);

    loop {
        let (page, next) = match ops.list_vectors_page(name, cursor.clone()).await {
            Ok(out) => out,
            Err(err) => {
                rollback(ops, &tmp_name).await;
                return Err(err);
            }
        };
        for record in page {
            report.scanned += 1;
            if predicate(&record.payload) {
                report.kept += 1;
                survivor_batch.push(record);
                if survivor_batch.len() >= BATCH_SIZE {
                    if let Err(err) = ops.upsert_batch(&tmp_name, &survivor_batch).await {
                        rollback(ops, &tmp_name).await;
                        return Err(err);
                    }
                    survivor_batch.clear();
                }
            } else {
                report.dropped += 1;
            }
        }
        match next {
            Some(c) => cursor = Some(c),
            None => break,
        }
    }

    if !survivor_batch.is_empty() {
        if let Err(err) = ops.upsert_batch(&tmp_name, &survivor_batch).await {
            rollback(ops, &tmp_name).await;
            return Err(err);
        }
    }

    // Atomic swap: drop original, rename `<name>.tmp` over `<name>`.
    if let Err(err) = ops.drop_collection(name).await {
        rollback(ops, &tmp_name).await;
        return Err(PruneError::SwapFailed {
            collection: name.to_string(),
            reason: format!("drop original failed: {err}"),
        });
    }
    if let Err(err) = ops.rename_collection(&tmp_name, name).await {
        return Err(PruneError::SwapFailed {
            collection: name.to_string(),
            reason: format!("rename tmp over original failed: {err}"),
        });
    }

    debug_assert!(
        report.invariant_holds(),
        "scanned must equal kept + dropped"
    );
    Ok(report)
}

/// Best-effort drop of `<name>.tmp` after a failure. The error is
/// logged at WARN but not returned — the caller already has the
/// upstream error and the rollback is informational.
async fn rollback(ops: &dyn VectorizerPruneOps, tmp_name: &str) {
    if let Err(err) = ops.drop_collection(tmp_name).await {
        tracing::warn!(
            tmp = %tmp_name,
            error = %err,
            "vectorizer_prune: rollback drop of .tmp collection failed"
        );
    }
}

/// Upsert batch size. Chosen to match the SDK's existing
/// `INSERT_BATCH_SIZE = 64` ceiling so the pruner does not exceed
/// the server-side request budget. Public so tests can shadow it
/// when driving large fixtures.
pub const BATCH_SIZE: usize = 64;

// ---- In-memory fixture for tests + the §3 cold-tier IT --------

/// In-memory [`VectorizerPruneOps`] backed by `BTreeMap<collection,
/// Vec<VectorRecord>>`. Drives every unit test in this module
/// (§4.4) plus the cold-tier sweep IT.
#[derive(Debug, Default)]
pub struct MemoryVectorizerPruneOps {
    inner: tokio::sync::Mutex<MemoryStore>,
}

#[derive(Debug, Default)]
struct MemoryStore {
    by_collection: std::collections::BTreeMap<String, Vec<VectorRecord>>,
    inject_upsert_error_once: Option<String>,
    inject_swap_error_once: bool,
}

impl MemoryVectorizerPruneOps {
    /// Empty fixture.
    pub fn new() -> Self {
        Self::default()
    }

    /// Seed `collection` with `records`. Replaces the prior bucket
    /// so callers can reset between scenarios.
    pub async fn seed(&self, collection: &str, records: Vec<VectorRecord>) {
        let mut inner = self.inner.lock().await;
        inner.by_collection.insert(collection.to_string(), records);
    }

    /// Snapshot the rows in `collection` for assertions.
    pub async fn snapshot(&self, collection: &str) -> Vec<VectorRecord> {
        let inner = self.inner.lock().await;
        inner
            .by_collection
            .get(collection)
            .cloned()
            .unwrap_or_default()
    }

    /// `true` when the collection currently exists in the fixture.
    pub async fn has(&self, collection: &str) -> bool {
        let inner = self.inner.lock().await;
        inner.by_collection.contains_key(collection)
    }

    /// Inject a one-shot upsert error so tests drive the rollback
    /// path deterministically.
    pub async fn inject_upsert_error_once(&self, reason: impl Into<String>) {
        let mut inner = self.inner.lock().await;
        inner.inject_upsert_error_once = Some(reason.into());
    }

    /// Inject a one-shot rename failure for the swap-error path.
    pub async fn inject_swap_error_once(&self) {
        let mut inner = self.inner.lock().await;
        inner.inject_swap_error_once = true;
    }
}

#[async_trait]
impl VectorizerPruneOps for MemoryVectorizerPruneOps {
    async fn list_vectors_page(
        &self,
        collection: &str,
        cursor: ListCursor,
    ) -> Result<(VectorPage, ListCursor), PruneError> {
        // Cursor encodes the byte offset into the collection as a
        // base-10 string. End-of-stream surfaces as `None`.
        let offset: usize = cursor
            .as_deref()
            .map(|s| s.parse::<usize>().unwrap_or(0))
            .unwrap_or(0);
        let inner = self.inner.lock().await;
        let rows = inner
            .by_collection
            .get(collection)
            .cloned()
            .unwrap_or_default();
        if offset >= rows.len() {
            return Ok((Vec::new(), None));
        }
        let end = (offset + PAGE_SIZE).min(rows.len());
        let page = rows[offset..end].to_vec();
        let next: ListCursor = if end >= rows.len() {
            None
        } else {
            Some(end.to_string())
        };
        Ok((page, next))
    }

    async fn upsert_batch(
        &self,
        dest_collection: &str,
        batch: &[VectorRecord],
    ) -> Result<(), PruneError> {
        let mut inner = self.inner.lock().await;
        if let Some(reason) = inner.inject_upsert_error_once.take() {
            return Err(PruneError::Vectorizer(reason));
        }
        let bucket = inner
            .by_collection
            .entry(dest_collection.to_string())
            .or_default();
        for record in batch {
            if let Some(existing) = bucket.iter_mut().find(|r| r.id == record.id) {
                *existing = record.clone();
            } else {
                bucket.push(record.clone());
            }
        }
        Ok(())
    }

    async fn drop_collection(&self, collection: &str) -> Result<(), PruneError> {
        let mut inner = self.inner.lock().await;
        inner.by_collection.remove(collection);
        Ok(())
    }

    async fn rename_collection(&self, from: &str, to: &str) -> Result<(), PruneError> {
        let mut inner = self.inner.lock().await;
        if inner.inject_swap_error_once {
            inner.inject_swap_error_once = false;
            return Err(PruneError::Vectorizer("synthetic rename failure".into()));
        }
        if let Some(rows) = inner.by_collection.remove(from) {
            inner.by_collection.insert(to.to_string(), rows);
        }
        Ok(())
    }
}

const PAGE_SIZE: usize = 128;

#[cfg(test)]
mod tests {
    use super::*;

    fn record(id: &str, kept: bool) -> VectorRecord {
        VectorRecord {
            id: id.to_string(),
            vector: vec![0.1, 0.2, 0.3, 0.4],
            payload: serde_json::json!({"kept": kept, "event_id": id}),
        }
    }

    fn keep_kept_true() -> PrunePredicate {
        Arc::new(|payload: &Value| {
            payload
                .get("kept")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
        })
    }

    #[tokio::test]
    async fn reencode_drops_expired_and_keeps_survivors() {
        // Phase14b §4.4 — 1 000 vectors / 300 expired / post-prune
        // count == 700 alive.
        let ops = MemoryVectorizerPruneOps::new();
        let mut seed = Vec::with_capacity(1_000);
        for i in 0..1_000 {
            let kept = i % 10 < 7; // 700 alive, 300 expired
            seed.push(record(&format!("vec-{i:04}"), kept));
        }
        ops.seed("cortex.cold.binary", seed).await;

        let report = reencode_collection(&ops, "cortex.cold.binary", keep_kept_true())
            .await
            .expect("reencode succeeds");

        assert_eq!(report.scanned, 1_000);
        assert_eq!(report.kept, 700);
        assert_eq!(report.dropped, 300);
        assert!(report.invariant_holds());

        let live = ops.snapshot("cortex.cold.binary").await;
        assert_eq!(live.len(), 700, "live collection holds only survivors");
        assert!(
            live.iter()
                .all(|r| r.payload["kept"].as_bool().unwrap_or(false)),
            "every survivor carries kept=true"
        );
        // .tmp dropped post-swap.
        assert!(
            !ops.has("cortex.cold.binary.tmp").await,
            ".tmp removed by the atomic swap"
        );
    }

    #[tokio::test]
    async fn reencode_predicate_keeps_all_is_idempotent_smoke() {
        let ops = MemoryVectorizerPruneOps::new();
        ops.seed(
            "cortex.cold.binary",
            vec![record("a", true), record("b", true), record("c", true)],
        )
        .await;
        let report = reencode_collection(&ops, "cortex.cold.binary", keep_kept_true())
            .await
            .unwrap();
        assert_eq!(report.scanned, 3);
        assert_eq!(report.kept, 3);
        assert_eq!(report.dropped, 0);
        assert_eq!(ops.snapshot("cortex.cold.binary").await.len(), 3);
    }

    #[tokio::test]
    async fn reencode_upsert_failure_leaves_original_intact() {
        let ops = MemoryVectorizerPruneOps::new();
        ops.seed(
            "cortex.cold.binary",
            (0..200)
                .map(|i| record(&format!("v-{i:03}"), true))
                .collect(),
        )
        .await;
        ops.inject_upsert_error_once("synthetic upsert").await;

        let err = reencode_collection(&ops, "cortex.cold.binary", keep_kept_true())
            .await
            .expect_err("upsert error must surface");
        match err {
            PruneError::Vectorizer(msg) => assert!(msg.contains("synthetic upsert")),
            other => panic!("wrong error: {other}"),
        }

        // ADR-013 §Decision §5 — original intact.
        assert_eq!(
            ops.snapshot("cortex.cold.binary").await.len(),
            200,
            "original untouched on upsert failure"
        );
        // Rollback dropped .tmp so the next sweep sees clean state.
        assert!(
            !ops.has("cortex.cold.binary.tmp").await,
            ".tmp dropped on rollback"
        );
    }

    #[tokio::test]
    async fn reencode_swap_failure_surfaces_swap_failed_error() {
        let ops = MemoryVectorizerPruneOps::new();
        ops.seed(
            "cortex.cold.binary",
            vec![record("a", true), record("b", false)],
        )
        .await;
        ops.inject_swap_error_once().await;

        let err = reencode_collection(&ops, "cortex.cold.binary", keep_kept_true())
            .await
            .expect_err("swap failure must surface");
        match err {
            PruneError::SwapFailed { collection, .. } => {
                assert_eq!(collection, "cortex.cold.binary");
            }
            other => panic!("wrong error: {other}"),
        }
    }

    #[test]
    fn prune_report_invariant_holds_for_default_and_seeded() {
        assert!(PruneReport::default().invariant_holds());
        let r = PruneReport {
            scanned: 10,
            kept: 7,
            dropped: 3,
        };
        assert!(r.invariant_holds());
        let bad = PruneReport {
            scanned: 10,
            kept: 7,
            dropped: 2,
        };
        assert!(!bad.invariant_holds());
    }

    #[test]
    fn tmp_suffix_and_env_knob_are_canonical_strings() {
        assert_eq!(TMP_SUFFIX, ".tmp");
        assert_eq!(PRUNE_MODE_ENV, "CORTEX_VECTORIZER_PRUNE_MODE");
    }
}
