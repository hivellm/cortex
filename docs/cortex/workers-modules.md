# cortex-workers — module + bin map

> **Status**: Draft v1 (phase11s §6.1)
> **Owner**: Core team
> **Related**: [ADR-007](../../.rulebook/decisions/007-cortex-workers-as-the-default-host-for-worker-style-daemons.md), [phase11s task](../../.rulebook/tasks/phase11s_workers_consolidation_merge/)

`cortex-workers` is the canonical host for every worker-style daemon
in the Cortex workspace. After the phase11s merge it bundles five
formerly-separate crates as modules under `src/<area>/` plus a single
matching bin under `src/bin/<bin-name>.rs` per area.

This document is the index — operators navigating the merged tree look
here first.

## Module map

| Module path                         | Source area    | What it does                                                                            | Was crate                |
|-------------------------------------|----------------|-----------------------------------------------------------------------------------------|--------------------------|
| `cortex_workers::classifier`        | `src/classifier/`        | Lib: classifier stack (Haiku / static), prompts, cache, budget, types | `cortex-classifier`     |
| `cortex_workers::classifier_worker` | `src/classifier_worker/` | Daemon: bridges `cortex.events.raw` + `cortex.events.bootstrap` → `cortex.events.enriched` | (was `cortex-workers/src/classifier/`) |
| `cortex_workers::embedder`          | `src/embedder/`          | Daemon + lib: tree-sitter symbol chunking + Vectorizer client                          | (in workers since v0.1) |
| `cortex_workers::fulltext`          | `src/fulltext/`          | Daemon + lib: Meilisearch document mapping + indexer                                   | (in workers since v0.1) |
| `cortex_workers::graph`             | `src/graph/`             | Daemon + lib: Cypher template registry + Nexus client + per-kind expansion             | (in workers since v0.1) |
| `cortex_workers::ingestion`         | `src/ingestion/`         | HTTP router accepting events + redacting + archiving + publishing to Synap            | `cortex-ingestion`      |
| `cortex_workers::claude_archive`    | `src/claude_archive/`    | JSONL archive walker that emits bootstrap envelopes from `~/.claude/projects/`        | `cortex-claude-archive` |
| `cortex_workers::consolidator`      | `src/consolidator/`      | LLM-summariser pipeline (Session / Topic / DecisionTrace grains) + cost ledger        | `cortex-consolidator`   |
| `cortex_workers::retention`         | `src/retention/`         | Tier-transition sweep + Meili prune + CAS vacuum + metadata reap + parquet rollup + PII enforcement + scheduler + turn digest | `cortex-retention` |

Phase11r will add `cortex_workers::topic_cards` (living-synthesis tier
on top of `consolidator`) per [ADR-006](../../.rulebook/decisions/) when
phase11r §1 lands. New worker-style modules SHALL follow this pattern
per ADR-007.

## Bin map

| Bin name                       | Source                                                | Feature gate          | Purpose                                                                                |
|--------------------------------|-------------------------------------------------------|-----------------------|----------------------------------------------------------------------------------------|
| `cortex-classifier-worker`     | `src/bin/classifier-worker.rs`                        | (none)                | Live classification daemon. `Budgeted ← Cached ← (HaikuCli \| Static)` stack          |
| `cortex-embedder-worker`       | `src/bin/embedder.rs`                                 | (none)                | Tree-sitter symbol chunker + Vectorizer writer                                         |
| `cortex-fulltext-worker`       | `src/bin/fulltext-indexer.rs`                         | (none)                | Meilisearch document indexer                                                           |
| `cortex-graph-worker`          | `src/bin/graph-writer.rs`                             | (none)                | Nexus Cypher writer + audit                                                            |
| `cortex-graph-backfill`        | `src/bin/graph-backfill.rs`                           | (none)                | One-shot bootstrap re-indexer for the graph layer                                      |
| `cortex-ingestion`             | `src/bin/cortex-ingestion.rs`                         | (none)                | Ingestion HTTP server (`/v1/events`, `/v1/events/batch`, `/healthz`, `/metrics`)      |
| `cortex-claude-archive`        | `src/bin/cortex-claude-archive.rs`                    | `claude-archive` (default OFF) | JSONL walker + tail watcher; opt-in via `--features claude-archive`            |
| `cortex-consolidator`          | `src/bin/cortex-consolidator.rs`                      | (none)                | Unified bin: `estimate` + `run-session` + `run-topic` + `run-decision` + `nightly`    |
| `cortex-retention-sweep`       | `src/bin/cortex-retention-sweep.rs`                   | (none)                | Dry-run plan validator for the tier-transition sweep (live path stays in `cortex-ops sweep`) |

`cortex-ops` (lives in `cortex-cli`, not `cortex-workers`) continues
to host the operator surface for `sweep` / `pii-enforce` / `cas-vacuum`
/ etc. against live backends. The standalone bins here are the
process-supervisor-friendly alternative for systemd / docker-compose.

## Feature flags

| Feature          | Default | Adds deps                       | Enables                                                                          |
|------------------|---------|---------------------------------|----------------------------------------------------------------------------------|
| `claude-archive` | OFF     | `indicatif`, `sysinfo`, `ignore` | `cortex_workers::claude_archive` module + `cortex-claude-archive` bin             |

When `claude-archive` is OFF (the default), `cargo build` will refuse
to build the `cortex-claude-archive` bin with the standard cargo
`required-features` error message. This is the contract the
`feature_gates_it` regression IT pins.

## Public re-exports — what consumers import

External consumers of cortex-workers reach the moved code via these
canonical paths:

```rust
// classifier lib (was cortex_classifier::*)
use cortex_workers::classifier::{
    Classifier, ClassifierStack, ClassifierOutput, ClassifierSource,
    EnrichmentInput, BudgetTracker, InMemoryCache, PricingTable,
    Severity, PiiRisk, build_offline_stack, build_stack,
};

// classifier worker daemon (was cortex_workers::classifier::* before phase11s)
use cortex_workers::classifier_worker::{
    ClassifierWorkerConfig, Worker, MemorySynapConsumer, MemorySynapPublisher,
    STREAM_RAW, STREAM_BOOTSTRAP, STREAM_ENRICHED,
};

// ingestion (was cortex_ingestion::*)
use cortex_workers::ingestion::{
    build_router, AppState, ArchiveWriter, IngestionConfig, MemoryPublisher,
    Metrics, NdJsonZstdArchive, Publisher, SynapPublisher,
};

// claude archive (was cortex_claude_archive::*; gated)
#[cfg(feature = "claude-archive")]
use cortex_workers::claude_archive::{
    Checkpoint, CheckpointStore, MapStats, MappedEnvelope, ReadStats,
    StdoutEmitter, WalkConfig, WalkEntry, WalkKind,
};

// consolidator (was cortex_consolidator::*)
use cortex_workers::consolidator::{
    cost_telemetry::{CostBudget, CostLedger},
    orchestrator::{Orchestrator, ProducerSelection, Trigger},
    summariser::{AnthropicSummariser, Summariser, SummariserKind, cost_cents},
};

// retention (was cortex_retention::*)
use cortex_workers::retention::{
    run_sweep, MemoryVectorizerOps, RecordRef, SweepKind, SweepPlan,
    SweepReport, Tier, TierPair, VectorizerOps,
};
use cortex_workers::retention::{cas_vacuum, meili_prune, metadata_reap, parquet_rollup, pii_enforce, scheduler, turn_digest};
```

The `module_re_export_it` regression IT pins this surface against
silent drops. If you remove a public re-export, that test is the gate
that fails.

## Where the production wiring lives

Three live-adapter paths still live OUTSIDE cortex-workers and call
back into the lib API in-process:

1. **Live retention sweep** — `cortex-cli/src/bin/cortex-ops.rs`
   subcommand `sweep` carries the production `LiveVectorizerOps`
   that talks to the deployed Vectorizer. The standalone
   `cortex-retention-sweep` bin in cortex-workers exposes only the
   `MemoryVectorizerOps` dry-run path; lifting the live adapter
   into `cortex_workers::retention::live` is a follow-up task.
2. **Retention scheduler** — `cortex-api/src/retention_daemon.rs`
   imports `cortex_workers::retention::scheduler::{tick, …}` to
   run the per-job cron tick in-process inside the API daemon.
3. **Consolidator orchestrator** — phase11j §3 routing wiring
   (live envelope read path) is queued behind phase11o
   (`vectorizer_demotion_api`); until it lands, the
   `cortex-consolidator` bin's `run-session` / `run-topic` /
   `run-decision` subcommands surface plan-only stubs.

## Migration tips

If you are porting code that imported from one of the merged crates:

| Old import (pre-phase11s)        | New import                                    | Notes                                            |
|----------------------------------|-----------------------------------------------|--------------------------------------------------|
| `cortex_classifier::Foo`         | `cortex_workers::classifier::Foo`             |                                                  |
| `cortex_ingestion::Foo`          | `cortex_workers::ingestion::Foo`              |                                                  |
| `cortex_claude_archive::Foo`     | `cortex_workers::claude_archive::Foo`         | Requires `--features claude-archive`             |
| `cortex_consolidator::Foo`       | `cortex_workers::consolidator::Foo`           |                                                  |
| `cortex_retention::Foo`          | `cortex_workers::retention::Foo`              |                                                  |

The `cortex_workers::classifier::ClassifierMode` is the LIB variant.
The DAEMON's `ClassifierMode` enum (Disabled / Cli / Static) lives at
`cortex_workers::classifier_worker::ClassifierMode`. Same name,
different types — the namespace separation is intentional.
