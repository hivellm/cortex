//! Phase13a §3.8 + §4.2 — integration test exercising the canonical
//! 7-sweep registry end-to-end. Validates the load-bearing gate of
//! ADR-009: every sweep produces exactly one `retention_sweeps` row
//! per invocation, regardless of whether the sweep had work to do
//! or not.
//!
//! The test wires:
//!
//! - `TierSweep` over `MemoryVectorizerOps` (empty store).
//! - `ParquetRollupSweep` over a `TempDir` archive root.
//! - `CasVacuumSweep` over an empty SQLite CAS DB.
//! - `PiiEnforceSweep` over a `StaticTargets` empty slice +
//!   `MemoryPiiBackend`.
//! - `MeiliPruneSweep` over `MemoryMeiliBackend`.
//! - `MetadataReapSweep` reusing the ctx's metadata handle.
//! - `ConsolidationPruneSweep` over `MemoryVectorizerClient` +
//!   `RecordingMeili` + an empty doc provider.
//!
//! Together they cover every backend the production daemon wires
//! today; the assertions are scoped to the row-materialisation
//! contract (`SweepRegistry::run_all` produces 7 rows in
//! `retention_sweeps`, each with the expected `name`-shaped
//! payload in `tier_transitions_json`).

use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use cortex_storage::MetadataStore;
use cortex_workers::embedder::vectorizer_client::MemoryVectorizerClient;
use cortex_workers::pruner::meili_sink::{MeiliPruneError, MeiliPruneOps};
use cortex_workers::retention::cas_vacuum::VacuumOpts;
use cortex_workers::retention::cas_vacuum_sweep::CasVacuumSweep;
use cortex_workers::retention::consolidation_prune_sweep::{
    ConsolidationPruneSweep, StaticConsolidationDocs,
};
use cortex_workers::retention::meili_prune::{MemoryMeiliBackend, PrunePlan};
use cortex_workers::retention::meili_prune_sweep::MeiliPruneSweep;
use cortex_workers::retention::metadata_reap::ReapPlan;
use cortex_workers::retention::metadata_reap_sweep::MetadataReapSweep;
use cortex_workers::retention::parquet_rollup_sweep::ParquetRollupSweep;
use cortex_workers::retention::pii_enforce::{EnforcementPlan, MemoryPiiBackend};
use cortex_workers::retention::pii_enforce_sweep::{PiiEnforceSweep, StaticTargets};
use cortex_workers::retention::tier_sweep::TierSweep;
use cortex_workers::retention::{MemoryVectorizerOps, SweepPlan};
use cortex_workers::sweep::{canonical_registry, into_handle, Sweep, SweepCtx, SweepReport};
use serde_json::Value;
use tempfile::TempDir;
use tokio::sync::Mutex;

fn fixed_now() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-05-19T03:00:00Z")
        .unwrap()
        .with_timezone(&Utc)
}

/// Recording `MeiliPruneOps` stub mirroring the pattern in
/// `pruner::engine` tests — captures every call so the suite can
/// assert the engine walked the index.
#[derive(Default)]
struct RecordingMeiliOps {
    updates: StdMutex<Vec<(String, Vec<Value>)>>,
    deletes: StdMutex<Vec<(String, Vec<String>)>>,
}

#[async_trait]
impl MeiliPruneOps for RecordingMeiliOps {
    async fn update_documents(&self, index: &str, docs: &[Value]) -> Result<(), MeiliPruneError> {
        self.updates
            .lock()
            .expect("updates lock")
            .push((index.to_string(), docs.to_vec()));
        Ok(())
    }
    async fn delete_documents(&self, index: &str, ids: &[String]) -> Result<(), MeiliPruneError> {
        self.deletes
            .lock()
            .expect("deletes lock")
            .push((index.to_string(), ids.to_vec()));
        Ok(())
    }
}

type CanonicalSweepSet = (
    Box<dyn Sweep>,
    Box<dyn Sweep>,
    Box<dyn Sweep>,
    Box<dyn Sweep>,
    Box<dyn Sweep>,
    Box<dyn Sweep>,
    Box<dyn Sweep>,
);

fn build_canonical_sweep_set(
    archive_root: PathBuf,
    cas_path: PathBuf,
    now: DateTime<Utc>,
) -> CanonicalSweepSet {
    let tier: Box<dyn Sweep> = Box::new(TierSweep::new(
        Arc::new(MemoryVectorizerOps::new()),
        SweepPlan::default_for(now),
    ));
    let parquet: Box<dyn Sweep> = Box::new(ParquetRollupSweep::new(archive_root));
    let cas: Box<dyn Sweep> = Box::new(CasVacuumSweep::new(cas_path, VacuumOpts::default_for(now)));
    let pii: Box<dyn Sweep> = Box::new(PiiEnforceSweep::new(
        Arc::new(StaticTargets::new(Vec::new())),
        Arc::new(MemoryPiiBackend::new()),
        EnforcementPlan::default_for(now),
    ));
    let meili: Box<dyn Sweep> = Box::new(MeiliPruneSweep::new(
        Arc::new(MemoryMeiliBackend::new()),
        PrunePlan::default_for(now),
    ));
    let metadata_reap: Box<dyn Sweep> =
        Box::new(MetadataReapSweep::new(ReapPlan::default_for(now)));
    let consolidation: Box<dyn Sweep> = Box::new(ConsolidationPruneSweep::new(
        Arc::new(StaticConsolidationDocs::new(Vec::new())),
        Arc::new(MemoryVectorizerClient::default()),
        Arc::new(RecordingMeiliOps::default()),
        "cortex_consolidations",
    ));
    (tier, parquet, cas, pii, meili, metadata_reap, consolidation)
}

#[tokio::test]
async fn canonical_registry_writes_one_retention_sweeps_row_per_sweep() {
    let archive_dir = TempDir::new().expect("temp archive dir");
    let cas_dir = TempDir::new().expect("temp cas dir");
    let cas_path = cas_dir.path().join("cas.sqlite");
    let now = fixed_now();

    let (tier, parquet, cas, pii, meili, metadata_reap, consolidation) =
        build_canonical_sweep_set(archive_dir.path().to_path_buf(), cas_path, now);
    let registry = canonical_registry(tier, parquet, cas, pii, meili, metadata_reap, consolidation);

    let store = MetadataStore::open_in_memory().expect("metadata store");
    let handle: Arc<Mutex<MetadataStore>> = into_handle(store);
    let ctx = SweepCtx::new(handle.clone(), "cortex.sweep.it").with_now(now);

    let reports: Vec<SweepReport> = registry.run_all(&ctx).await.expect("run_all");
    assert_eq!(reports.len(), 7, "expected 7 reports for the canonical set");
    let names: Vec<&str> = reports.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(
        names,
        vec![
            "tier_sweep",
            "parquet_rollup",
            "cas_vacuum",
            "pii_enforce",
            "meili_prune",
            "metadata_reap",
            "consolidation_prune",
        ]
    );

    // The bookkeeping contract: one `retention_sweeps` row per
    // invocation. The dashboard reads only this table (ADR-014 /
    // §4.3); the assertion below is the load-bearing gate.
    let rows = handle
        .lock()
        .await
        .list_recent_sweeps(50)
        .expect("list_recent_sweeps");
    assert_eq!(rows.len(), 7, "expected 7 retention_sweeps rows");
    for row in &rows {
        assert!(
            row.finished_at.is_some(),
            "row {} has no finished_at",
            row.sweep_id
        );
        assert!(
            row.status == "success" || row.status == "failed",
            "row {} status is non-terminal: {}",
            row.sweep_id,
            row.status
        );
        // `tier_transitions_json` carries the full `SweepReport`
        // payload — parsing it back must succeed.
        let payload: SweepReport =
            serde_json::from_str(row.tier_transitions_json.as_deref().unwrap_or("{}"))
                .expect("payload deserialises as SweepReport");
        assert!(!payload.name.is_empty());
    }
}
