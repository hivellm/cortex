# 7. cortex-workers as the default host for worker-style daemons

**Status**: proposed
**Date**: 2026-05-03
**Related Tasks**: phase11s_workers_consolidation_merge, phase11j_consolidation_tier, phase11r_topic_card_mcp_enrichment

## Context

Cortex grew to 14 workspace members through organic, phase-by-phase
crate creation. Five of those members (`cortex-classifier`,
`cortex-ingestion`, `cortex-claude-archive`, `cortex-consolidator`,
`cortex-retention`) shared the same lifecycle as the existing workers
in `cortex-workers`: Synap subscriber, audit envelope emit, shared
cost ledger, settings reload watcher, run loop. Each crate
re-instantiated client setup (Synap / Vectorizer / Nexus), tracing
init, build.rs scaffolding, and operator-facing bin naming.

Three frictions:

1. **Duplicated boilerplate** drifting across crates — five copies of
   the same Synap subscribe loop, four copies of the audit envelope
   builder, two copies of the cost-ledger pattern (one in
   `cortex-consolidator`, one to-be-built in `cortex-topic-cards` per
   phase11r §2.7).
2. **Workspace member sprawl** — operators reading `Cargo.toml` could
   not tell which member was a worker daemon vs a library vs a
   service. Five members were single-binary daemons.
3. **Fragmented cost telemetry** — phase11j's `CostBudget` +
   `CostLedger` lived inside `cortex-consolidator`. phase11r §2.1
   would have imported them from across a crate boundary; future
   producers (TopicCards, EvalRunner, GoldFixtureSynth) repeat the
   pattern.

The user-feedback law `feedback_no_new_crates.md` states "features
novas vão pros crates principais" — that rule was already breached
when those five crates landed, but the breach went unsurfaced.
phase11s closes the retroactive gap.

## Decision

Future worker-style daemons SHALL live inside `cortex-workers` as a
module under `src/<area>/` plus a binary under `src/bin/<bin-name>.rs`.
Heavy or area-specific dependencies SHALL be gated behind a Cargo
feature flag. The default `cortex-workers` build SHALL stay lean —
operators that never run a particular daemon do not pay for its deps.

Concretely, after phase11s the workspace shrank from 14 → 9 members:

- `cortex-classifier` → `cortex_workers::classifier` (lib only;
  unconditional)
- `cortex-ingestion` → `cortex_workers::ingestion` + bin
  `cortex-ingestion` (axum + tower + tower-http added unconditionally
  to workers; lean enough)
- `cortex-claude-archive` → `cortex_workers::claude_archive` + bin
  `cortex-claude-archive` gated behind `--features claude-archive`
  (default OFF). Heavy deps (`indicatif`, `sysinfo`, `ignore`) are
  optional and only linked when the feature is on.
- `cortex-consolidator` → `cortex_workers::consolidator` + bin
  `cortex-consolidator` (no feature gate — all deps already shared)
- `cortex-retention` → `cortex_workers::retention` + new bin
  `cortex-retention-sweep` (no feature gate; `rusqlite` + `cron`
  added unconditionally, both small)

The two bins that previously had divergent implementations
(`cortex-consolidator` lived in BOTH `cortex-cli/src/bin/` and
`cortex-consolidator/src/bin/`) MUST reconcile into a single bin
under `cortex-workers/src/bin/`. The phase11s §4.1 audit recovered
both surfaces (phase11j §2.9 run-* / nightly subcommands + phase11q
§1 estimate subcommand) and merged them into a unified
five-subcommand bin.

For the `cortex-retention-sweep` bin, the live `LiveVectorizerOps`
adapter still lives in `cortex-cli/src/bin/cortex-ops.rs`. The new
bin ships with a dry-run-only path today; lifting the live adapter
into `cortex_workers::retention::live` is left to a follow-up task
so this ADR's surgical-changes contract holds.

## Alternatives Considered

1. **Keep five separate crates for compile parallelism.** Rust's
   compile parallelism within a crate is module-level, so merging
   does cost some end-to-end build time. Empirically (per §6.4
   measurement) the workspace cold-build dropped because removing
   five per-crate codegen-units + link steps outweighs the
   intra-workers serialisation. Even if it had cost time, the
   coherence + boilerplate-dedupe gain dominates.

2. **Mint a new `cortex-daemons` umbrella crate.** Would have moved
   the sprawl from "many small crates" to "one umbrella + many
   small crates" without solving the cost-ledger / Synap-loop
   duplication. cortex-workers already hosts four daemons
   (classifier-worker, embedder-worker, fulltext-worker,
   graph-worker, graph-backfill) so the natural host already
   exists.

3. **Push everything into `cortex-cli` as subcommands.** The bins
   would have lost their cron-friendly names (systemd / docker
   key off bin name; `cortex-cli ingestion` is awkward). Also
   couples operator-facing CLI to the worker daemons in a way that
   makes deployment more brittle.

4. **Feature-gate every merged module.** Initially considered for
   `cortex-consolidator` — discarded because the deps are already
   shared with the rest of cortex-workers (no dep savings; gate is
   cosmetic). Only `claude-archive` justified its feature flag
   because `indicatif` / `sysinfo` / `ignore` are unique to it.

## Consequences

**Positive:**

- Workspace 14 → 9 members. Operator reading `Cargo.toml` sees
  service crates (`cortex-api`, `cortex-mcp-server`, `cortex-cli`)
  separately from the worker host (`cortex-workers`).
- Cost ledger (`CostBudget` / `CostLedger`) sits inside the workers
  module hierarchy, ready for phase11r §2.7 (TopicCards) and any
  future LLM-driven producer to share.
- Bin names preserved across the merge — every `--bin <name>`
  invocation that worked pre-merge keeps working post-merge.
- `claude-archive` feature flag pattern available for future
  optional daemons. Default cortex-workers build does not link
  `notify` / `sysinfo` / `ignore`.

**Negative:**

- `cortex-workers` lib is now ~50 k LOC. Files are easier to lose
  inside the larger tree; `Glob` / `Grep` discipline becomes the
  load-bearing navigation aid (the canonical tree is
  `cortex-workers/src/<area>/<file>.rs` so `Glob` against
  `crates/cortex-workers/src/**/*.rs` produces the full picture).
- Shared `[features]` block means feature flag interactions need
  careful per-feature regression tests (the §6.2 IT
  `feature_gates_it.rs` pins the contract).
- Re-using a single Cargo.toml means dep additions touch every
  module's compile graph. Mitigated by the `optional = true`
  pattern for area-specific heavy deps.

**Future-proofing:**

- Future worker-style daemons (e.g. phase11r's TopicCards
  rewrite-worker, phase11n's dashboard publisher) MUST land inside
  `cortex_workers::<area>` per this ADR. Minting a new top-level
  crate requires a written deviation in the proposal explaining
  why this ADR does not apply.
- The `claude-archive` precedent applies: any daemon whose deps
  are heavy (≥ 3 unique crates not already in workers) SHOULD
  ship behind a feature flag with the same `default = []`
  treatment.
- Reconciliation of duplicate bins (the §4.1 outcome) sets the
  pattern: when two crates export bins of the same name, the
  unified bin lives in `cortex-workers/src/bin/`, both surfaces
  become subcommands, and the call site picks the right
  subcommand at runtime.

## Implementation Notes

phase11s §1-§5 each shipped one merged crate with full gates
(`cargo check` + `cargo test` + `cargo build --bin`) before deleting
the old crate dir via `git rm -r`. The §6 tail (this file + docs +
4 ITs + workspace-wide gates) closes the loop.

§3.1 deviation: the original §3 plan listed `notify`,
`notify-debouncer-mini`, `cron`, `flate2`, `rusqlite` as the
heavy deps to gate behind `claude-archive`. None were actually used
by `cortex-claude-archive`. The real unique deps were `indicatif`,
`sysinfo`, `ignore` — which is what the feature actually gates.

§4.2 deviation: the original §4 plan called for a `consolidator`
feature flag with `default = ["consolidator"]`. After verifying
deps, every dep `cortex-consolidator` brought was already in
cortex-workers from earlier phases. Feature flag would have gated
only the module declaration with zero dep savings. Not added.

§5.4 deviation: the original §5 plan called for `--scope` /
`--max-age-days` / `CORTEX_RETENTION_*` env vars + audit-envelope
emission on `cortex-retention-sweep`. None of those exist on the
existing `cortex-ops sweep` surface. The bin ships dry-run-only
today; pinning the bin-name slot is the immediate value, the live
sweep path stays on `cortex-ops sweep` until the
`LiveVectorizerOps` adapter is lifted.

§5.5 deviation: there is no `cortex-cli sweep` subcommand. The
actual operator entry is `cortex-ops sweep` (a subcommand of the
`cortex-ops` bin in cortex-cli). That subcommand already called
`cortex_retention::run_sweep` in-process; the §5.3 sed automatically
repointed it to `cortex_workers::retention::run_sweep`. Parity
preserved by construction.
