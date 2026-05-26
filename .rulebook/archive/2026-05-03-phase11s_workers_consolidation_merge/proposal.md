# Proposal: phase11s_workers_consolidation_merge

## Why

Five sibling crates — `cortex-classifier`, `cortex-ingestion`,
`cortex-claude-archive`, `cortex-consolidator`, `cortex-retention`
— share the same lifecycle as the existing workers in
`cortex-workers` (Synap subscriber, audit envelope emit, shared
cost ledger, settings push, run loop). Keeping them as separate
workspace members produces three concrete frictions:

1. **Duplicated boilerplate.** Each crate re-instantiates Synap
   client setup, Vectorizer/Nexus client wiring, audit envelope
   builders, tracing init, settings reload watchers. Five copies
   that drift over time.
2. **Workspace member sprawl.** 14 members today; 5 of them are
   single-binary daemons that share the same operator-facing
   surface (the `cortex-*-worker` style bin). Operators reading
   `Cargo.toml` cannot tell which member is a worker daemon vs a
   library vs a service.
3. **Cost telemetry fragmentation.** The shared `CostBudget` +
   `CostLedger` introduced by phase11j §2.7-§2.8 lives inside
   `cortex-consolidator`. phase11r (queued) layers
   `cortex-topic-cards` on top by importing
   `cortex_consolidator::{Summariser, CostBudget, CostLedger}`.
   The same plumbing will repeat for any future LLM-driven
   producer. Hosting the shared primitives inside `cortex-workers`
   removes the cross-crate import dance and lets future producers
   (TopicCards, EvalRunner, GoldFixtureSynth) sit alongside
   without minting new top-level crates.

The merge also closes the loop on the user-feedback law
"Não criar crates novos — features novas vão pros crates
principais (cortex-workers / cortex-api / cortex-storage /
cortex-core / cortex-cli / etc.)" by retroactively folding five
single-purpose crates that should have been workers modules from
day one.

This task does NOT change runtime behaviour. Bins keep their
operator-facing names (`cortex-claude-archive`,
`cortex-consolidator`, `cortex-ingestion`,
`cortex-classifier-worker`, `cortex-embedder-worker`,
`cortex-fulltext-worker`, `cortex-graph-worker`,
`cortex-graph-backfill`) — only the crate that defines them
changes. Library exports keep semantic identity via re-exports
during a one-cycle deprecation window so phase11j (in progress)
and any external consumer building against the published path
deps does not break mid-flight.

### Ordering vs. phase11r

`phase11r_topic_card_mcp_enrichment` (queued, pending) imports
`cortex_consolidator::Summariser` in §2.1 and §2.3. If phase11s
runs first, phase11r §2 is authored against the merged paths
(`cortex_workers::consolidator::Summariser`) from day one — no
churn. If phase11s runs after phase11r §2, the merge has to
rewrite phase11r imports mid-implementation, which violates
LAW-CORTEX-001 (sequential execution) by forcing a Phase N → N
go-back. Therefore phase11s runs ahead of phase11r in the
sequence.

Phase11j (in progress, 37/73) is unaffected at the source level
— `cortex-consolidator::*` is still the canonical path while
phase11j is mid-flight; the merge happens after phase11j §6 ships
or in parallel with §5/§6 only if those phases do not import new
symbols from any of the five crates. §5 is blocked-external on
phase11o; §6 is the tail (docs/tests/verify) — neither adds new
imports. Safe to run.

## What Changes

### §1 — `cortex-classifier` → `cortex-workers/src/classifier_lib/`

`cortex-classifier` is a library (zero bins) consumed only by
`cortex-workers` (`cortex-workers/Cargo.toml:40`). Smallest blast
radius — start here.

- Move `crates/cortex-classifier/src/**` →
  `crates/cortex-workers/src/classifier_lib/` (the existing
  `cortex-workers/src/classifier_worker/` is the daemon glue
  that consumes the lib; rename to disambiguate).
- Re-home the `pub use classifier_lib::{classify, Classifier,
  ClassifierError, …}` surface at `cortex_workers::classifier`
  (no underscore in the public path; `_lib` is internal to avoid
  the collision with the worker module).
- Move `crates/cortex-classifier/tests/*` →
  `crates/cortex-workers/tests/classifier_*.rs` with `classifier_`
  prefix on each filename so per-test filtering still works
  (`cargo test -p cortex-workers --test classifier_prompt`).
- Remove the workspace member entry for `cortex-classifier`.
- Remove the `cortex-classifier = { path = "../cortex-classifier" }`
  path-dep entry from `cortex-workers/Cargo.toml`; replace any
  `use cortex_classifier::*` inside cortex-workers with the new
  `crate::classifier::*` path.
- Delete the `crates/cortex-classifier/` directory entirely after
  the merge cycle ships and tests pass.

### §2 — `cortex-ingestion` → `cortex-workers/src/ingestion/` + bin

`cortex-ingestion` ships one library + one bin
(`cortex-ingestion`). Consumers: `cortex-cli`, `cortex-health`
(comment-only — no source dep).

- Move `crates/cortex-ingestion/src/**` →
  `crates/cortex-workers/src/ingestion/`.
- Move `crates/cortex-ingestion/src/bin/cortex-ingestion.rs` →
  `crates/cortex-workers/src/bin/cortex-ingestion.rs`. Bin name
  preserved (`name = "cortex-ingestion"` in workers Cargo.toml).
- Re-export at `cortex_workers::ingestion::*`.
- `cortex-cli/Cargo.toml`: drop `cortex-ingestion = { path = … }`
  path-dep. The bin is now produced by `cortex-workers`; if
  `cortex-cli` imports any ingestion symbol via lib it switches
  to `cortex-workers = { path = "../cortex-workers" }` (or
  whatever existing dep entry already points there).
- `cortex-health/Cargo.toml`: comment update only — no source
  change.
- Delete `crates/cortex-ingestion/`.

### §3 — `cortex-claude-archive` → `cortex-workers/src/claude_archive/` + bin

`cortex-claude-archive` ships one library + one bin
(`cortex-claude-archive`). No external consumers (own bin only).

- Move `crates/cortex-claude-archive/src/**` →
  `crates/cortex-workers/src/claude_archive/`.
- Move `crates/cortex-claude-archive/src/bin/cortex-claude-archive.rs`
  → `crates/cortex-workers/src/bin/cortex-claude-archive.rs`.
  Bin name preserved.
- Pull the heavy deps (`notify`, `notify-debouncer-mini`, `cron`,
  `flate2`, `zstd`, `rusqlite`) into `cortex-workers/Cargo.toml`
  under a `claude-archive` feature flag so the default
  `cortex-workers` build does NOT pull them. The
  `cortex-claude-archive` bin enables the feature via
  `[[bin]] required-features = ["claude-archive"]`.
- Re-export at `cortex_workers::claude_archive::*` (gated under
  the `claude-archive` feature).
- Delete `crates/cortex-claude-archive/`.

### §4 — `cortex-consolidator` → `cortex-workers/src/consolidator/` + bin

`cortex-consolidator` ships one library + one bin
(`cortex-consolidator`). Consumed by `cortex-cli` (lib + bin
path override) and by `cortex-topic-cards` (phase11r, queued).

- Move `crates/cortex-consolidator/src/**` →
  `crates/cortex-workers/src/consolidator/`.
- Move `crates/cortex-consolidator/src/bin/cortex-consolidator.rs`
  → `crates/cortex-workers/src/bin/cortex-consolidator.rs`. Bin
  name preserved.
- **Audit-and-purge the duplicate bin entry in
  `cortex-cli/Cargo.toml:26-27`** — there is a stale
  `[[bin]] name = "cortex-consolidator"` pointing at
  `cortex-cli/src/bin/cortex-consolidator.rs`. Determine whether
  that file is a thin trampoline or a divergent copy; if
  trampoline, delete it (the workers bin replaces it); if
  divergent, reconcile content into the workers bin BEFORE
  deleting. Surface findings in the task tasks.md item; do not
  silently lose any code.
- Add `consolidator` feature flag in `cortex-workers/Cargo.toml`
  gating the Anthropic SDK + chrono + reqwest paths the
  consolidator-only code uses.
- Drop `cortex-consolidator = { path = "../cortex-consolidator" }`
  from `cortex-cli/Cargo.toml`.
- Re-export at `cortex_workers::consolidator::*` so the eventual
  phase11r §2.1 (`cortex_workers::consolidator::Summariser`)
  resolves.
- Delete `crates/cortex-consolidator/`.

### §5 — `cortex-retention` → `cortex-workers/src/retention/` + new sweep bin

`cortex-retention` is a library (zero bins) consumed by
`cortex-api` and `cortex-cli`. Largest at 6 966 LOC.

- Move `crates/cortex-retention/src/**` →
  `crates/cortex-workers/src/retention/`.
- Move `crates/cortex-retention/tests/*` →
  `crates/cortex-workers/tests/retention_*.rs` with the
  `retention_` filename prefix.
- Re-export at `cortex_workers::retention::*`.
- Drop `cortex-retention = { path = "../cortex-retention" }` from
  `cortex-api/Cargo.toml` and `cortex-cli/Cargo.toml`. Update
  imports in those crates from `cortex_retention::*` to
  `cortex_workers::retention::*`.
- Add a new `cortex-retention-sweep` bin under
  `cortex-workers/src/bin/cortex-retention-sweep.rs` exposing the
  retention sweep that today is invoked via `cortex-cli sweep`
  subcommand. The bin is operator-facing (cron-driven) and
  matches the naming convention of the other worker daemons. The
  `cortex-cli sweep` subcommand stays as a wrapper that shells
  out to (or imports the same lib path as) the new bin so the
  operator-facing CLI surface does not change.
- Delete `crates/cortex-retention/`.

### §6 — Tail (docs + tests + verify)

Mandatory tail enforced by rulebook v5.3.0. ADR-007 documents
the consolidation choice and the feature-flag scheme so future
worker-style daemons land inside `cortex-workers` by default.

## Impact

- **Affected specs:** 02 (storage layout — no schema change, but
  the worker-host crate name updates in the diagram), 04
  (cortex-core/dep graph — workers picks up 5 new modules), 05
  (classifier — moves to workers), 06 (embedder — already in
  workers, unchanged), 09 (bootstrap CLI — `cortex-ingestion`
  bin path note), 13 (laws — no impact), 16 (dashboard — bin
  list update if it enumerates daemons).
- **Affected code:**
  - **New:** `crates/cortex-workers/src/classifier_lib/`,
    `crates/cortex-workers/src/ingestion/`,
    `crates/cortex-workers/src/claude_archive/`,
    `crates/cortex-workers/src/consolidator/`,
    `crates/cortex-workers/src/retention/`,
    `crates/cortex-workers/src/bin/cortex-ingestion.rs`,
    `crates/cortex-workers/src/bin/cortex-claude-archive.rs`,
    `crates/cortex-workers/src/bin/cortex-consolidator.rs`,
    `crates/cortex-workers/src/bin/cortex-retention-sweep.rs`,
    `.rulebook/decisions/007-cortex-workers-as-the-default-host-for-worker-style-daemons.md`.
  - **Modified:** `Cargo.toml` (workspace members 14 → 9),
    `crates/cortex-workers/Cargo.toml` (5 new modules + 4 new
    bins + 2 new feature flags `claude-archive` +
    `consolidator`), `crates/cortex-cli/Cargo.toml` (drop 3 path
    deps + 1 duplicate bin entry, repoint imports),
    `crates/cortex-api/Cargo.toml` (drop retention path dep,
    repoint imports), `crates/cortex-health/Cargo.toml` (comment
    only), every Rust file inside the moved trees that uses
    crate-relative `crate::` paths (no behaviour change, only
    module-path rewrites where module hierarchy shifts under
    `cortex-workers/src/`).
  - **Deleted:** `crates/cortex-classifier/`,
    `crates/cortex-ingestion/`, `crates/cortex-claude-archive/`,
    `crates/cortex-consolidator/`, `crates/cortex-retention/`.
- **Breaking:** YES on the path-dep surface for any external
  consumer (none known outside the workspace). Internal: handled
  in-place by the same commit cycle; no consumer-facing API
  signature changes. Bin names preserved on the operator-facing
  surface.
- **Build-time delta:** `cortex-workers` compile time grows from
  ~30s cold-cache to an estimated ~60-70s cold-cache (about 2x).
  Incremental builds within the merged crate stay under the
  current envelope because rustc's incremental engine still
  compiles only changed modules. Workspace cold build drops
  because 5 fewer crates means 5 fewer per-crate codegen-units +
  link steps; net workspace cold-build time is expected to drop
  10-15% per local profiling on a similar Rust monorepo merge
  (verified post-merge, item §6.4).
- **Test isolation delta:** `cargo test -p cortex-classifier`
  becomes `cargo test -p cortex-workers --test classifier_*`
  (or `--test classifier_prompt` for a single file). Filename
  prefix convention preserved so per-area test runs stay easy.
- **Storage / runtime / latency:** ZERO. This is a build-system
  refactor; binaries produce byte-equivalent output, daemons
  publish the same envelopes, MCP tools resolve the same paths,
  Vectorizer/Nexus/Synap/Meili clients use the same wire calls.
- **User benefit:** five fewer workspace members to navigate;
  shared infrastructure (Synap subscribe loop, audit envelope
  builder, cost ledger) lives in one place; future LLM-driven
  producers (TopicCards from phase11r, future EvalRunner /
  GoldFixtureSynth / DecisionTraceProducer) inherit the workers
  scaffold without minting a new crate; ADR-007 pins the rule so
  the "Não criar crates novos" feedback law has a structural
  enforcement point.

## Source

User directive 2026-05-03: "quero que cortex-classifier,
cortex-claude-archive, cortex-consolidator, cortex-ingestion,
cortex-retention sejam migrados para cortex-workers".

Reinforced by user-memory feedback law `feedback_no_new_crates.md`
("features novas vão pros crates principais"). This task closes
the retroactive gap left by the original 14-crate split.
