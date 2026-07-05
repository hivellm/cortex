# Proposal: phase28_docs-truth-reconciliation

## Why
`docs/specs/00-index.md` documents 43 distinct spec files sharing only 39 available leading numbers because four numbers — 20, 26, 27, and 28 — are each reused by two different files, discovered during the 2026-07-05 platform verification pass while reconciling the index against the real file list. Ambiguous spec numbers ("spec 20", "spec 27", ...) create real confusion in any tooling or documentation that assumes a 1:1 number-to-file mapping. `README.md`, `docs/architecture.md`, `docs/specs/00-index.md`, and `docs/specs/20-mcp-tool-surface.md` were already corrected directly in that same session; this task is the remaining work that was deliberately deferred out of that pass: renumbering the four colliding files, fixing every cross-reference to them, and adding a permanent guard so the collision can't silently recur.

## What Changes
1. Rename `docs/specs/20-opencode-adapter.md` → `docs/specs/23-opencode-adapter.md` (23 is the lowest currently-free slot); update its own `# NN — Title` header.
2. Rename `docs/specs/26-eval.md` → `docs/specs/29-eval.md` (next free slot); update its header.
3. Of the `27-*` pair, `docs/specs/27-retrieval-rerank.md` is the less externally cross-referenced file (~5 inbound references, all in archived task docs, vs. ~12 for `27-consolidation.md`, which include a live code comment in `crates/cortex-workers/src/consolidator/trigger_producer.rs` and a test-data reference) — rename `27-retrieval-rerank.md` → `docs/specs/37-retrieval-rerank.md`; `27-consolidation.md` keeps its number.
4. Of the `28-*` pair, `docs/specs/28-gui-contract.md` is the less externally cross-referenced file (~2 inbound references vs. ~4 for `28-phantom-link-verifier.md`) — rename `28-gui-contract.md` → `docs/specs/38-gui-contract.md`; `28-phantom-link-verifier.md` keeps its number.
5. Grep the repo (`docs/`, `crates/`, `.rulebook/`) for every inbound link/reference to the four renamed files' OLD filenames and update them to the new filenames.
6. Replace `docs/specs/00-index.md`'s "Known numbering collisions" callout (added during the 2026-07-05 pass) with a short changelog-style note recording that the collisions were resolved by this task, and update the spec table's number/filename columns for the four moved rows.
7. Add an automated check (a small script, or a `cortex-ops doctor` check) that fails when two files in `docs/specs/` share the same leading number, so this class of drift can't silently recur.

## Impact
- Affected specs: `docs/specs/20-opencode-adapter.md` (→23), `docs/specs/26-eval.md` (→29), `docs/specs/27-retrieval-rerank.md` (→37), `docs/specs/28-gui-contract.md` (→38), `docs/specs/00-index.md` (table + callout updated), plus every file with an inbound link to the four renamed files (other docs, code comments, `.rulebook/archive/**`)
- Affected code: a new spec-number-uniqueness check (implementer decides whether it lives in `crates/cortex-cli/src/bin/cortex-ops/doctor.rs` or a standalone script)
- Breaking change: NO — filenames change but content doesn't; every consumer is internal docs/tooling
- User benefit: every spec number maps to exactly one file, removing ambiguity for anyone (human or tooling) referencing "spec NN"; the new automated check prevents the same drift from recurring
