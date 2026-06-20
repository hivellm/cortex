# 06 — Community-aware entity dedup — **MED**

## What graphify does

`dedup.py:deduplicate_entities()` collapses duplicate/near-duplicate nodes *before* clustering, with a carefully staged pipeline that avoids both false merges and O(n²):
1. **Exact-label normalization** (case/punctuation).
2. **Entropy gate** — skip labels < 2.5 bits/char (won't auto-merge "AI", "DB", "x"); prevents over-merging ambiguous short names.
3. **MinHash + LSH blocking** (`_minhash.py`, custom, no scipy) — 3-gram shingles, 128 perms, threshold 0.7 → O(n) candidate pairs.
4. **Jaro-Winkler verify** (≥ 92%) — catches typos/plurals/spacing.
5. **Same-community boost** (+0.05 when both nodes share a Leiden community) — graph structure disambiguates homonyms ("User" in auth vs. shipping) that string similarity alone can't.
6. **Union-find merge** preferring shorter, non-chunk-suffixed survivor IDs; rewire edges, drop self-loops.
7. **Optional LLM tiebreaker** for the 0.75–0.85 band, batched (~$0.01 / 10k nodes), off by default.

## What Cortex does today

- **MinHash exists, but only in memory consolidation** — `crates/cortex-cli/src/ops/memory_consolidate.rs` (+ `cortex-ops.rs`), used to dedup memory entries during consolidation. So the *technique* is already in the codebase.
- **No entity-level dedup over the graph**, and **no community-aware** disambiguation (communities don't exist — file 02). The graph writer dedups by exact node identity (`NodeOp::with_identity`) only; two heuristically-extracted symbols with slightly different ids/labels stay separate, and two genuinely-same concepts from different sources aren't merged.
- Retrieval has **anchor-dedupe** (collapsing near-identical *hits* at query time) — different concern (result hygiene), not graph entity resolution.

**Gap:** the graph accumulates near-duplicate symbol/concept nodes (esp. once schema/infra/LLM sources land, files 04–05), inflating it and splitting evidence across twins. No structural signal is used to merge them.

## Recommendation for Cortex

When community detection lands (file 02), add a **graph entity-resolution pass** reusing the existing MinHash code:
- Lift the MinHash util out of `cortex-cli/ops` into a shared crate (or `cortex-workers`) so both memory consolidation and graph dedup share it.
- Port graphify's **staging order**: entropy gate → MinHash/LSH blocking → string verify (a Rust `rapidfuzz`/Jaro-Winkler) → **same-community boost** → union-find merge with survivor-id preference.
- Run it in the same nightly graph worker as community detection (resolution before/after partition, graphify does dedup-before-cluster but uses community as a *boost*, so iterate: cluster → dedup-with-boost → re-cluster, or compute a cheap pre-partition for the boost).
- Keep the optional LLM tiebreaker behind a flag + the daily budget tracker Cortex already has.

## Effort / impact

- **Impact:** MED — graph hygiene + better evidence aggregation; compounding value as sources multiply (04/05).
- **Effort:** MED — technique already in-repo; work is generalizing it to graph nodes + wiring the community boost. **Prereq: file 02 for the boost** (a label-only version can ship earlier without it).
