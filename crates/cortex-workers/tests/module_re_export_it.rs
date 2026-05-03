//! phase11s §6.2 — compile-time IT pinning the module re-export contract.
//!
//! Every previously-separate crate that phase11s folded into cortex-workers
//! exposed a public surface its consumers imported. This IT references at
//! least one type per merged module under its new path. If a future change
//! accidentally drops a re-export or renames a module, this file fails to
//! compile — the IT is the canary, not the assertions.

#![allow(unused_imports, dead_code)]

// §1 — classifier (lib content; was crate cortex-classifier)
use cortex_workers::classifier::{
    build_offline_stack, BudgetTracker, Classifier, ClassifierError, ClassifierOutput,
    ClassifierSource, EnrichmentInput, HaikuCliClassifier, HaikuCliConfig, InMemoryCache,
    PiiRisk, PricingTable, Severity, StaticClassifier,
};

// §1 — classifier_worker (daemon glue; was the existing
// cortex-workers/src/classifier/ before the namespace was freed for the lib)
use cortex_workers::classifier_worker::{
    ClassifierMode as DaemonMode, ClassifierWorkerConfig, ConsumedMessage,
    LiveSynapConsumer, LiveSynapPublisher, MemorySynapConsumer, MemorySynapPublisher,
    Worker, STREAM_BOOTSTRAP, STREAM_ENRICHED, STREAM_RAW,
};

// §2 — ingestion (was crate cortex-ingestion)
use cortex_workers::ingestion::{
    build_router, AppState, ArchiveWriter, IngestionConfig, MemoryPublisher, Metrics,
    NdJsonZstdArchive, Publisher, SynapPublisher,
};

// §3 — claude_archive is feature-gated; reference its types only when the
// feature is enabled so the default-feature build of this IT stays clean.
#[cfg(feature = "claude-archive")]
use cortex_workers::claude_archive::{
    ArchiveEmitter, Checkpoint, CheckpointStore, MapStats, MappedEnvelope, ReadStats,
    StdoutEmitter, WalkConfig, WalkEntry, WalkKind,
};

// §4 — consolidator (was crate cortex-consolidator)
use cortex_workers::consolidator::cost_telemetry::{CostBudget, CostLedger, GrainCost};
use cortex_workers::consolidator::orchestrator::{Orchestrator, ProducerSelection, Trigger};
use cortex_workers::consolidator::summariser::{
    cost_cents, AnthropicSummariser, Summariser, SummariserError, SummariserKind,
};

// §5 — retention (was crate cortex-retention)
use cortex_workers::retention::{
    new_sweep_id, run_sweep, MemoryVectorizerOps, RecordRef, SweepError, SweepKind,
    SweepPlan, SweepReport, Tier, TierPair, TierTransition, VectorizerOps,
};
use cortex_workers::retention::cas_vacuum::{run as cas_run, VacuumOpts};
use cortex_workers::retention::meili_prune::{run_meili_prune, MeiliBackend, MeiliDoc};
use cortex_workers::retention::metadata_reap::{run as reap_run, ReapPlan};
use cortex_workers::retention::pii_enforce::{run_enforcement, EnforcementPlan, PiiTarget};
use cortex_workers::retention::scheduler::tick;
use cortex_workers::retention::turn_digest::{run_turn_digest, DigestPlan, Turn};

#[test]
fn re_export_compile_check() {
    // The work happens at compile time (the `use` items above). This test
    // exists so `cargo test -p cortex-workers --test module_re_export_it`
    // produces a passing harness frame instead of just a compile-only
    // crate.
    let _ = SweepKind::Turn.as_str();
    let _ = Tier::Fp32.as_str();
    let _ = SummariserKind::Haiku45;
    assert_eq!(SummariserKind::Haiku45 as u8 as i32, 0);
}
