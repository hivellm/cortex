---
type: pattern
title: Worker-style daemons live in cortex-workers behind a feature flag
relatedTasks:
  - phase11s_workers_consolidation_merge
tags:
  - architecture
  - workspace
  - cargo
  - features
  - workers
---

# Worker-style daemons live in cortex-workers behind a feature flag

## Pattern

When you need to add a new daemon that consumes from `cortex.events.*`,
emits audit envelopes, runs a Synap subscribe loop, or interacts with
the shared cost ledger — host it as a module under
`cortex_workers::<area>` rather than minting a new top-level crate.

## When to apply

ANY of:

- The daemon shares lifecycle with the existing workers
  (classifier-worker, embedder-worker, fulltext-worker, graph-worker,
  graph-backfill).
- The daemon needs the `CostBudget` / `CostLedger` from
  `cortex_workers::consolidator::cost_telemetry` (LLM-driven
  producers).
- The daemon is operator-facing as a single bin (systemd / docker
  entry point).
- The daemon's heavy deps are already shared with cortex-workers
  (no Cargo savings from isolation).

## How to apply

1. Module placement: `cortex-workers/src/<area>/` with a `mod.rs`
   declaring sub-modules. Bin at `cortex-workers/src/bin/<bin-name>.rs`.
2. Add `pub mod <area>;` to `cortex-workers/src/lib.rs` so external
   consumers reach it as `cortex_workers::<area>::*`.
3. Bin registered in `cortex-workers/Cargo.toml` under `[[bin]]`.
4. If the daemon adds ≥3 unique deps not already in cortex-workers,
   gate them behind a feature flag with `default = []`. Mark the
   deps `optional = true`. Wrap the module declaration with
   `#[cfg(feature = "<feature>")] pub mod <area>;` and add
   `required-features = ["<feature>"]` to the bin stanza.
5. If the daemon's deps are already shared with workers, ship
   unconditionally (no feature flag). The cosmetic feature with no
   dep savings is an anti-pattern — it misleads operators.

## Reference

Five crates merged via this pattern in phase11s:
- `cortex-classifier` → `cortex_workers::classifier`
  (no feature, deps already shared)
- `cortex-ingestion` → `cortex_workers::ingestion`
  (axum + tower added unconditionally; bin: `cortex-ingestion`)
- `cortex-claude-archive` → `cortex_workers::claude_archive`
  (FEATURE-GATED behind `claude-archive` because indicatif + sysinfo
  + ignore are unique to it; bin: `cortex-claude-archive`,
  required-features = ["claude-archive"])
- `cortex-consolidator` → `cortex_workers::consolidator`
  (no feature, deps already shared; bin: `cortex-consolidator`,
  unified from two divergent sources)
- `cortex-retention` → `cortex_workers::retention`
  (no feature, rusqlite + cron added unconditionally; bin:
  `cortex-retention-sweep`)

## Anti-pattern

Minting a new top-level crate "for compile parallelism" or "for
isolation" when the daemon shares lifecycle with cortex-workers.
The five merged crates carried five copies of the same Synap loop +
audit-envelope builder + cost-ledger plumbing. The merge eliminated
that drift while preserving every operator-facing bin name.

## Validation

The `module_re_export_it` regression IT pins the public surface
post-merge. The `feature_gates_it` IT pins the `claude-archive`
gate contract. The `bin_surface_parity_it` IT pins that every
operator bin survives the merge with a working `--help`. ADR-007
documents the rule with the per-section deviations the phase11s
merge surfaced.
