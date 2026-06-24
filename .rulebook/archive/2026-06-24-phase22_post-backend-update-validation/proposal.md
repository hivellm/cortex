# Proposal: phase22_post-backend-update-validation

## Why

Cortex's hybrid retrieval is currently degraded to **keyword-only** because
two of the three lanes are dead at the backend level, confirmed by a live
MCP query battery on 2026-06-06:

- **Vector/semantic lane** returns nothing useful — `hivehub/vectorizer:3.3.0`
  serves BM25-512 only; it coerces every collection to `bm25` and `/embed`
  ignores the model param (always 512-dim BM25). A paraphrase query that
  required semantic matching returned irrelevant generic files. Filed as
  [hivellm/vectorizer#306](https://github.com/hivellm/vectorizer/issues/306).
- **Graph lane** mostly works (85–100% of nodes are propertied per the
  phase20 audit) but is bottlenecked by the Nexus 2.2.0 Cypher engine:
  `$param` binding returns null
  ([nexus#3](https://github.com/hivellm/nexus/issues/3)) forcing
  inline-literal Cypher everywhere, and a property-corruption class
  ([nexus#4](https://github.com/hivellm/nexus/issues/4)) leaves an
  edge-seed straggler cohort property-less.

These are tracked as upstream issues the maintainer will ship fixes for.
This task is the **validation gate** that runs once those backend updates
land: it re-measures retrieval quality end-to-end, proves the dense + graph
lanes actually contribute again, drops the inline-literal Cypher
workarounds when `$param` binding is fixed, and fills the labelled corpus
that unblocks the two phase18 eval gates left blocked on data
(§3.8 temporal MRR, §5.4 cross-project MRR).

Without this task there is no structured acceptance for "did the backend
fixes actually restore Cortex retrieval quality?" — the risk is shipping
backend bumps and assuming recovery without measuring it.

## What Changes

A validation + cleanup sweep gated on the upstream backend fixes, in 5
phases. Each phase that depends on a specific upstream fix is marked with a
hard precondition; if the fix has not shipped, that phase stays blocked
(LAW-CORTEX-001 exemption 2) rather than producing a false green.

- **P0 — Preconditions + baseline.** Assert the deployed backend versions
  expose the fixes (Vectorizer serves a dense provider at 768; Nexus binds
  `$param`; Nexus property corruption resolved). Snapshot a pre-update
  retrieval baseline (the same MCP battery + `cortex-eval` numbers) so the
  delta is measurable.
- **P1 — Dense lane validation (gated on vectorizer#306).** Re-index a
  corpus with the dense provider; assert collections report a dense
  provider at dim 768 (not `bm25`/512); run the MCP battery and assert the
  semantic/paraphrase query returns `source: vector` hits with relevant
  top-K; assert `cortex-eval --suite retrieval` MRR@10 ≥ 0.60.
- **P2 — Graph lane validation + workaround removal (gated on nexus#3/#4).**
  Assert `$param`-bound Cypher returns rows; **remove the inline-literal +
  `sanitize_literal` workarounds** across the graph writer + timeline /
  branch / history / supersession / cross-project call sites, replacing
  them with parameterized queries; assert no property-less straggler cohort
  remains after a re-seed; assert the graph lane returns within its budget
  slice.
- **P3 — Labelled corpus → unblock phase18 eval gates.** Author the
  labelled time-sensitive + cross-project query corpus
  (`tests/golden/retrieval.csv` `expected_event_ids`, cross-project subset)
  that phase18 §3.8 + §5.4 were blocked on; run both eval gates and record
  the temporal +10% MRR delta + the cross-project positive delta. Flip
  phase18 §3.8 / §5.4 from blocked to done.
- **P4 — Full hybrid acceptance + Synap observability.** Re-run the whole
  `cortex-eval` battery (retrieval / consolidation / classification /
  access-control) green; confirm all three lanes contribute in a fused
  result; verify the Synap stream/consumer/lag metrics
  ([synap#196](https://github.com/hivellm/synap/issues/196)) are exposed
  and wire a coverage probe; re-enable the CI workflows disabled during the
  degraded window (Doctor consistency gate, eval, Relevance harness gate).

## Impact

- **Affected specs:** updates to `docs/specs/31-temporal-classifier.md`
  (§3.8 gate result), `34-cross-project-axis.md` (§5.4 gate result); new
  `docs/runbooks/post-backend-update-validation.md`.
- **Affected code:** `crates/cortex-workers/src/graph/**` (drop
  inline-literal Cypher → parameterized once nexus#3 lands),
  `crates/cortex-cli/src/bin/cortex-ops/{timeline,branch_cmd,query_cmd,backfill_cross_project}.rs`
  (same workaround removal), `crates/cortex-api/src/lanes/nexus_graph_lane.rs`
  (parameterized templates), `tests/golden/{retrieval,access_control}.csv`
  (labelled corpus), `crates/cortex-eval/**` (gate runs), `.env`
  (`CORTEX_EMBEDDER_DIM` 512 → 768 once vectorizer#306 lands),
  `docker-compose.yml` (dense Vectorizer config if required).
- **Breaking change:** NO — validation + workaround removal. The
  parameterized-Cypher swap is behaviour-preserving (same queries, safe
  binding) and only lands after nexus#3 is verified fixed.
- **User benefit:** measurable proof that retrieval quality recovered to
  full hybrid (keyword + dense + graph) after the backend fixes; the
  injection-prone inline-literal Cypher is removed; the two phase18 eval
  gates close; CI gates come back on.

## Source

Live MCP query-battery + backend-capability probes captured 2026-06-06
(this session). Upstream issues: vectorizer#306 / #300, nexus#3 / #4 / #5,
synap#196. phase20_retrieval-relevance-recovery (archived) for the coverage
+ graph-property baseline.

## Dependencies

- **Hard (per phase):** P1 gated on vectorizer#306 (dense provider shipped);
  P2 gated on nexus#3 (param binding) + nexus#4 (property corruption); P4's
  Synap probe gated on synap#196. Each gated phase stays `⏸ blocked` with a
  one-line reason until its upstream fix is deployed — do not mark a gated
  item green against the degraded backend.
- **Soft:** phase21 access-control eval suite (P4 re-runs it); phase18 §3.8
  + §5.4 (P3 unblocks them).
