# Proposal: phase27d_scip-precise-extraction

Source: docs/analysis/graphify-comparison/ (R4, file 04b)

## Why

Cortex's code edges (`calls`/`imports`/`defines` from
`crates/cortex-workers/src/graph/extractors/`) come from tree-sitter
heuristic name/scope resolution over ~10 languages. They cannot reliably
disambiguate which `foo()` across crates a call refers to, so the graph
lane is bounded by heuristic precision. graphify solves this with **SCIP
ingestion**: it consumes language-server symbol indexes (rust-analyzer,
gopls, tsserver) as JSON to emit *precise* cross-file references. Cortex
is a Rust codebase with rust-analyzer already in the dev loop, so exact
xrefs are available for free — this is the single biggest precision win
for the graph. (Relates to the planned `phase23c_ua-extraction-contract`,
but SCIP specifically is not yet planned anywhere.)

## What Changes

- New SCIP ingestion path (`cortex-scip`): run `rust-analyzer scip` (and
  later `scip-typescript`, etc.) in bootstrap/CI, parse the SCIP index,
  and emit **precise** `calls`/`references`/`defines` edges tagged
  `Extracted` (confidence 1.0 — see phase27a), superseding heuristic
  edges where SCIP covers the file.
- Port graphify's two-pass resolver (`scip_ingest.py`): build a
  symbol→node-id index, then emit edges resolving each reference to its
  exact definition (same-document precedence, global fallback), stubbing
  unresolved targets as `scip_external` so edges never dangle (converts
  the writer's current endpoint-missing soft-drops into resolvable
  anchors).
- Start Rust-only (highest value); extend per language as SCIP indexers
  become available.

## Impact

- Affected specs: `docs/specs/07-graph-writer.md` (precise edges +
  `scip_external` anchors), bootstrap/CI spec for the indexer step.
- Affected code: new `cortex-scip` ingest module + bootstrap/CI
  orchestration; graph projection/writer for SCIP-derived edges.
- Breaking change: NO (additive ingest path; supersedes heuristic edges
  where covered).
- User benefit: exact cross-references in the graph lane (no more
  ambiguous `calls`), the best graph precision in the ecosystem.
- Prereq: pairs with phase27a (confidence tiers). Relates to
  phase23c_ua-extraction-contract — reconcile, do not duplicate.
