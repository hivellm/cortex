# Proposal: phase4e_bootstrap_analysis_promotion

## Why

The architecture promises `Analysis` as a first-class entity
([architecture.md §4.1](../../../docs/architecture.md)) and the
dashboard ships an Analysis view ([gui/src/views/Analysis.tsx](../../../gui/src/views/Analysis.tsx)).
But the bootstrap walker has no `FileClass::Analysis`, no
`[cortex.analyses]` config block, and no `analysis.imported` event
kind — so analyses written under `docs/analysis/**/*.md` are
silently demoted to generic `artifact.doc` events. They land in
`cortex-Cortex-docs` instead of a dedicated analyses index, and
Nexus has zero `Analysis` nodes (audit 2026-04-27 22:36 UTC):

```
nodes: Artifact 3634, Repo 3, Session 9, Turn 28,
       LawViolation 72, Decision 12, Memory 24
       Analysis 0   ← this gap
```

The 2026-04-28 system audit captured under
`docs/analysis/cortex/00-10.md` is the immediate trigger: 11 files
of structured analysis exist on disk and there is no path that
would route them to the Analysis surface. The same gap will hit
every future audit, deep-analysis report (spec 15), and any
analysis a user writes by hand.

## What Changes

A new analysis-promotion path through bootstrap, parallel to the
existing decisions / laws / memories paths:

- **`[cortex.analyses]`** block in `cortex.toml` with a
  `promote_patterns` field. Default in [cortex.toml](../../../cortex.toml)
  promotes `docs/analysis/**/*.md` and `docs/analyses/**/*.md`.
- **`AnalysesConfig`** in [crates/cortex-bootstrap/src/config.rs](../../../crates/cortex-bootstrap/src/config.rs)
  mirroring the existing `PromoteConfig` shape.
- **`FileClass::Analysis`** variant in [crates/cortex-bootstrap/src/walker.rs](../../../crates/cortex-bootstrap/src/walker.rs);
  `classify_path` checks analyses promotions before falling through
  to the doc-extension default. Rescue walk also honours the new
  promote patterns so a `.gitignore`d `docs/analysis/` (unlikely
  but possible) is still rescued.
- **`emit_analysis_imported`** in [crates/cortex-bootstrap/src/emitter.rs](../../../crates/cortex-bootstrap/src/emitter.rs)
  producing kind `analysis.imported` with payload
  `{ title, status, body, source_path }`. Title derives from H1;
  status from a `Status:` front-matter line if present (defaults
  to `draft`).
- **Classifier-worker mapping** in
  [crates/cortex-classifier-worker/src/](../../../crates/cortex-classifier-worker/src/)
  routes the new bootstrap kind onto a `cortex_core::events::Kind`
  variant + family `analyses`, so the embedder/graph/fulltext
  workers fan out to a dedicated `cortex-{repo}-analyses` index /
  collection / sub-graph.
- **Graph mapper** in [crates/cortex-graph/src/mapper.rs](../../../crates/cortex-graph/src/mapper.rs)
  emits `(:Analysis {id, title, status, repo})` and
  `(:Analysis)-[:ANALYZES]->(:Repo)` per analysis event. Future
  cross-references (analysis → decision, analysis → memory) are
  out-of-scope for this task — they need bidirectional name
  resolution that is not yet built.
- **Cortex-core `Kind`** gains an `Analysis` variant where the
  enriched-event family enum is defined, so all four downstream
  workers can match on it without string compares.

After this lands, running `cortex-bootstrap .` against the Cortex
repo emits 11 `analysis.imported` events for the existing
`docs/analysis/cortex/*.md` files, populates the
`cortex-Cortex-analyses` index/collection, and surfaces them in
the dashboard's Analysis view.

## Impact

- **Affected specs:** 09 (bootstrap CLI — adds analyses block);
  06/07/08 (workers — add the analyses family routing);
  04 (cortex-core Kind enum); 16 (dashboard — Analysis view
  becomes populated, no code change needed there beyond what
  already exists).
- **Affected code:**
  - [crates/cortex-bootstrap/src/config.rs](../../../crates/cortex-bootstrap/src/config.rs) — add `AnalysesConfig`.
  - [crates/cortex-bootstrap/src/walker.rs](../../../crates/cortex-bootstrap/src/walker.rs) — add `FileClass::Analysis`, classify, rescue-walk gate.
  - [crates/cortex-bootstrap/src/emitter.rs](../../../crates/cortex-bootstrap/src/emitter.rs) — add `emit_analysis_imported`, branch in `emit_for_file`.
  - [crates/cortex-classifier-worker/src/](../../../crates/cortex-classifier-worker/src/) — kind mapping for `analysis.imported`.
  - [crates/cortex-core/](../../../crates/cortex-core/) — add `Kind::Analysis` variant if not present.
  - [crates/cortex-graph/src/mapper.rs](../../../crates/cortex-graph/src/mapper.rs) — Analysis node + ANALYZES edge.
  - [crates/cortex-fulltext/src/routing.rs](../../../crates/cortex-fulltext/src/routing.rs) — `analyses` family routing (one-line addition).
  - [cortex.toml](../../../cortex.toml) — `[cortex.analyses]` block.
- **Breaking change:** NO. New kind, new family; existing kinds
  unchanged.
- **User benefit:** the `docs/analysis/cortex/*.md` audit lands on
  the Analysis surface instead of disappearing into the docs
  bucket; the gap between architecture promise and reality
  (`Analysis 0` nodes) closes; future deep-analysis reports
  (spec 15) reuse this path instead of needing their own.

## Source

- 2026-04-28 system analysis under
  [docs/analysis/cortex/](../../../docs/analysis/cortex/) — the
  immediate trigger.
- [docs/architecture.md §4.1](../../../docs/architecture.md) —
  defines `Analysis` as a core entity type.
- 2026-04-27 audit recorded in [phase4c proposal](../phase4c_graph_richer_edges_defines/proposal.md)
  — documents the `Analysis 0` gap.
