# Proposal: phase4i_doctor_query_overlap_mode

## Why

`phase4d_indexing_consistency_doctor` shipped coverage mode (per-
partition counts across backends). The proposal also called for
**probe mode** — running the same query against vector / keyword /
graph lanes and computing the Jaccard overlap of top-K result
paths — but it needs same-query semantics across three lane shapes
that don't exist in `cortex-ops` yet (it has no vectorizer-sdk or
nexus-graph-sdk dependency, no shared query result type). That's
its own design surface.

## What Changes

- Add a `--query <q>` (repeatable) flag to `cortex-ops doctor-consistency`.
- For each query, run identical text searches against:
  - Meili — `POST /indexes/{uid}/search` against every canonical
    index, top-K result paths.
  - Vectorizer — KNN search against every collection, top-K
    chunk paths.
  - Nexus — `MATCH (a:Artifact) WHERE toLower(a.body) CONTAINS
    toLower($q) RETURN a.path LIMIT k` (substring match — not
    semantic, but it's the cheapest signal).
- Compute pairwise Jaccards `|A∩B|/|A∪B|` for the three pairs
  (vec↔meili, vec↔nexus, meili↔nexus) and a triple-intersection
  size.
- Threshold: `min_overlap_jaccard` (default 0.2). When any pair
  falls below the threshold, mark the run failed.
- Per-query report: query, per-lane top-K, three Jaccards, triple
  intersection.

## Impact

- Affected specs: spec-08 doctor section gains a probe-mode
  sub-section.
- Affected code:
  - `crates/cortex-ops/src/doctor/probe.rs` (new submodule)
  - `crates/cortex-ops/src/main.rs` — wire `--query` plumbing
  - tests: against in-memory probe trait
- Breaking change: NO. New flag.
- Depends on: phase4h (Vectorizer + Nexus probes are the search
  client surface this builds on).
- User benefit: catches **semantic** drift between lanes — when
  one backend silently stops indexing a class of envelopes, the
  query overlap collapses before counts diverge.

## Source

- Carved out of `phase4d_indexing_consistency_doctor` items 3.1–
  3.4 because the Jaccard/probe path is its own design axis.
