# Proposal: phase11p_corpus_cleanup_sweep

## Why

Snapshot of the live Meili stack on 2026-05-03 reports **292,444 docs across 190 indexes**. The dashboard's `events_total: 25,097` is a filtered window; the real corpus is much larger and accumulating waste. Audit findings:

| Symptom | Volume | Root cause |
|---|---|---|
| **91 empty indexes** | 0 docs each (47% of all indexes) | Bootstrap created repos that were renamed/abandoned (`csharp`, `go`, `python`, `rust`, `tests`, `x`, …). The `MeiliFulltextIndexer` boot-time sweep exists (`fulltext::sweep::sweep_empty_non_canonical`) but has not run since the last redeploy. |
| **TML repo bloat** | 215,364 docs (74% of total); `cortex-tml-code` alone holds 189,872 | `../Tml/cortex.toml` likely missing build / vendor / generated excludes (`target/`, `dist/`, `build/`, `node_modules/`, `vendor/`). |
| **Globals (`cortex_decisions` / `cortex_laws`)** | Absent from Meili | phase11k §2 dual-write shipped 2026-05-03; the running fulltext-worker binary predates that build. |
| **`law_violation` re-emission** | 3,804 envelopes across 8 repos for ~50 unique `LAW-*` ids | Each bootstrap re-emits the same AGENTS.override.md / spec doc. The `bootstrap_seen` ledger should dedupe by `content_hash`; phase11p §4 audits whether it's doing so or whether the new phase11k §3.2 split path bypasses it. |

This task is mechanical only — no LLM, no destructive ops without operator confirmation. The companion task `phase11q_corpus_consolidation_run` covers the LLM-driven Haiku/Opus consolidation pass.

## What Changes

In this repo only:

1. **Sweep empty Meili indexes** — extend `cortex-ops` (or add a one-shot `cortex-fulltext-worker --sweep-empty` flag) that walks `/indexes` via the Meili admin API, calls the existing `is_canonical_index_name` predicate, and `DELETE /indexes/{uid}` for any non-canonical OR canonical-but-zero-doc target. Dry-run first, list candidates, require operator `--apply` for the destructive call.
2. **Redeploy fulltext-worker** with the post-phase11k binary so dual-write `cortex_decisions` / `cortex_laws` activates and settings v5 lands across every per-repo index. Document the redeploy as a runbook step in `docs/cortex/redeploy-after-phase11k.md`.
3. **TML excludes audit** — read `../Tml/cortex.toml` and `../TmlDocs/cortex.toml`; produce a diff PR (against the TML repo, scope is read-only here) adding `target/`, `dist/`, `build/`, `node_modules/`, `vendor/`, `generated/`, plus any `.generated.{ts,go,rs}` patterns the audit surfaces.
4. **`law_violation` dedupe pass** — confirm the `bootstrap_seen` ledger trips on AGENTS / spec re-emits. If not, write a one-shot `cortex-ops dedupe-laws` that scans the per-repo governance indexes and `DELETE` documents whose `(law_id, content_hash)` collides with an older sibling in the same index. Dry-run + `--apply` gating identical to §1.
5. **Acceptance metric** — after Onda 1 lands, the Meili index count drops from 190 → ~99, and `law_violation` count in the dashboard drops from 3,804 to under 500.

## Impact

- **Affected code:** `crates/cortex-cli/src/bin/cortex-ops.rs` (new `sweep-empty` and `dedupe-laws` subcommands), `crates/cortex-workers/src/fulltext/sweep.rs` (extend canonical-but-zero predicate), `docs/cortex/redeploy-after-phase11k.md` (new runbook), TML `cortex.toml` (read-only audit diff produced here for upstream PR).
- **Breaking change:** NO. Sweep + dedupe are destructive on the corpus but additive on the codebase; both are gated behind `--apply`.
- **Cost:** zero LLM tokens. CPU for the sweep is bounded by Meili's index count (~190 stat calls); the dedupe scans up to 3,804 governance documents.
- **User benefit:** corpus shrinks ~70% (Meili index count and `law_violation` row count both drop); dashboard counters become honest; phase11k §2 globals start populating immediately after redeploy.

## Source

Live Meili snapshot on 2026-05-03 via `curl -H Authorization http://127.0.0.1:17004/stats`. phase11k §2 dual-write contract in `crates/cortex-workers/src/fulltext/routing.rs::index_for_event_global`. `bootstrap_seen` ledger lives in `crates/cortex-cli/src/bootstrap/runner.rs::run_repo_with_dedup`.
