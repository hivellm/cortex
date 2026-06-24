# Proposal: phase23b_ua-incremental-indexer

## Why

Re-indexing a repo's graph and embeddings from scratch on every change is slow and
expensive — a typical commit touches 1–5 files but triggers full-repo work. The
Understand-Anything analysis documents a fingerprint-based incremental algorithm
(git-hash staleness → changed-file diff → tiered change classifier → surgical
node/edge merge) that re-does only the changed files' worth of graph, and only
escalates architecture-level re-synthesis when the change is structural enough to
warrant it. This is the single highest-ROI borrow from UA. It depends on the ontology
from phase23a being in place.

Source: `docs/analysis/understand-anything/04-incremental-patching.md`,
`docs/analysis/understand-anything/02-findings.md` (F-1, F-2).

## What Changes

- Persist a per-repo `last_indexed_commit_hash` fingerprint.
- Add a staleness check: `git diff <last>..HEAD --name-only` yields the changed-file
  set; equal hashes are a no-op.
- Add a change classifier producing `SKIP` / `PARTIAL_UPDATE` / `ARCHITECTURE_UPDATE`
  / `FULL_UPDATE` from the structural-change count (thresholds configurable per repo,
  defaulting to UA's 10 / 30 / 50%).
- Add a `file_path → {node_ids}` index for O(1) removal, and a merge that
  bitemporal-closes nodes/edges for changed files (not hard delete — preserves
  history), rebinds renames (`git diff --name-status` R), re-extracts changed files,
  re-embeds only those nodes, and advances the fingerprint.
- Gate consolidation/topic-card re-synthesis on the classifier tier (ARCH/FULL only).
- Trigger surface: re-use Cortex's existing commit capture + SessionStart hook to fire
  the staleness check (cheap `meta`-hash compare as a daemon-down fallback).

## Impact

- Affected specs: incremental indexer / staleness (this task's spec delta).
- Affected code: `crates/cortex-workers` graph + embedding indexer, `cortex-storage`
  fingerprint persistence + node-id↔file index, consolidation scheduler gating.
- Breaking change: NO (new code path; first run with no fingerprint falls back to full
  index).
- User benefit: sub-second re-index on typical commits, no wasted re-embeds, history
  preserved via bitemporal close instead of delete.
