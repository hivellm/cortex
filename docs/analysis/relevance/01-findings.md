# 01 — Findings

Each finding has stable id `F-NNN`, evidence with file:line citations, the relevance impact on pre-thinking + MCP consultive bundles, a confidence level, and the existing rulebook task that tracks it (when any).

---

## F-001 — Keyword lane is single-repo; vector and graph cover three. Bundles for two of three indexed repos serve zero keyword hits.

**Evidence:** [`docs/analysis/cortex/03-data-quality.md:38-48`](../cortex/03-data-quality.md) — Meili docs `Cortex=589, Rulebook=0, Vectorizer=0` while Vectorizer holds `17,629 / 9,264 / 101,293`. Routing code at [`crates/cortex-fulltext/src/routing.rs`](../../../crates/cortex-fulltext/src/routing.rs) is correct; the failure is at the worker consumer (offset / state).

**Impact:** Pre-thinking bundles for any prompt whose `scope.repo ∈ {Rulebook, Vectorizer}` get a degenerate keyword pass (zero hits). RRF then leans entirely on the vector lane, where BM25-as-embedding scored `0.136` on the audit's "classifier worker" probe — the bundle is "weak" but the index is the actual fault.

**Confidence:** High (audited 2026-04-27).

**Tracked by:** [`phase4a_fulltext_fanout_parity_and_stale_meili_cleanup`](../../../.rulebook/tasks/phase4a_fulltext_fanout_parity_and_stale_meili_cleanup/proposal.md).

---

## F-002 — Graph lane has only 2 edge types out of ~12 in the architecture spec; the chunker emits `symbol` per chunk but `cortex-graph::mapper` drops it.

**Evidence:** [`docs/analysis/cortex/03-data-quality.md:64-98`](../cortex/03-data-quality.md). Edge counts: `IN_REPO=10245`, `REMEMBERS=30`. Vectorizer payload sample contains `"symbol": "PreThinkingTool"` but [`crates/cortex-graph/src/mapper.rs`](../../../crates/cortex-graph/src/mapper.rs) does not consume it.

**Impact:** Graph lane cannot answer "where is `X` defined / called / imported / supersedes". `pre_change_context` runs `edge_artifact_touched_neighbours` (see [`crates/cortex-api/src/strategies.rs:120`](../../../crates/cortex-api/src/strategies.rs)) which returns nothing useful in this topology. The graph overlay in pre-thinking is therefore almost always empty; users perceive it as "graph doesn't work".

**Confidence:** High.

**Tracked by:** [`phase4c_graph_richer_edges_defines`](../../../.rulebook/tasks/phase4c_graph_richer_edges_defines/proposal.md).

---

## F-003 — `Scope.repo` defaults to `None` in `/v1/query`; `repo_scoped(req, family)` then routes to the `unknown` slug, returning zero hits across all lanes.

**Evidence:** [`crates/cortex-api/src/strategies.rs:19-29`](../../../crates/cortex-api/src/strategies.rs) — `repo_scoped` falls back to `UNKNOWN_REPO_SLUG` when scope is empty. [`crates/cortex-api/src/types.rs:60-74`](../../../crates/cortex-api/src/types.rs) — `Scope.repo` is `Option<String>` defaulting to `None`. Pre-thinking pipeline derives `scope.repo` from `cwd` ([`crates/cortex-pre-thinking/src/scope.rs:95-111`](../../../crates/cortex-pre-thinking/src/scope.rs)), but **direct callers of `/v1/query` (MCP `cortexQuery`, dashboard, GUI search)** do not pass a `cwd` — they pass the user's literal `QueryRequest`.

**Impact:** Any MCP `cortexQuery` call (or curl from the dashboard search bar) without explicit `scope.repo` lands on `cortex-unknown-{family}` — empty index, empty collection, empty graph. Hits: `0`. The user sees "Cortex returned nothing" and assumes broken; reality is route-to-nowhere.

**Confidence:** High — verified by reading both files.

**Tracked by:** R10 in [`docs/analysis/cortex/09-risks-and-debt.md`](../cortex/09-risks-and-debt.md), no implementation task yet.

---

## F-004 — Query is the user prompt verbatim, in every lane. No enrichment, no rewriting, no expansion, no decoupling of code-search from question-answering.

**Evidence:** [`crates/cortex-api/src/strategies.rs:101-117`](../../../crates/cortex-api/src/strategies.rs) — `VectorRequest.query = req.query.clone()`, `KeywordRequest.query = req.query.clone()`. [`crates/cortex-pre-thinking/src/pipeline.rs:117`](../../../crates/cortex-pre-thinking/src/pipeline.rs) — pipeline forwards `input.user_prompt` to `req.query` as-is.

**Impact:** A prompt like *"why is the meili fan-out broken — should we just rewrite the worker?"* hits the keyword lane with the entire English sentence, not extracted noun phrases. Meili's typo-tolerant tokenizer copes, but the vector lane sends the full question to the embedder; semantic match is dominated by the framing words ("why is ... broken") rather than the load-bearing tokens (`meili`, `fan-out`, `worker`). The intent table at [`crates/cortex-pre-thinking/src/intent_select.rs:21-112`](../../../crates/cortex-pre-thinking/src/intent_select.rs) *routes* by keyword but does not *rewrite* the query.

**Severity:** Medium — measurable on a probe set but easy to demonstrate.

**Confidence:** High.

**Tracked by:** Not tracked. Candidate for a new task.

---

## F-005 — RRF assumes lane outputs are equally informative. Vector lane scores reach 0.9; Meili `_rankingScore` is in `[0,1]`; graph hits get `0.0` unless an explicit score is set. RRF normalises by rank only, so ties / sparse lanes massively distort the fused order.

**Evidence:** [`crates/cortex-api/src/fusion.rs:16-49`](../../../crates/cortex-api/src/fusion.rs) — `rrf_fuse` uses pure positional rank `1/(60+rank)`. [`crates/cortex-api/src/meili_lane.rs:103-105`](../../../crates/cortex-api/src/meili_lane.rs) — Meili's `_rankingScore` is captured into `LaneHit.score` but **never used by `rrf_fuse`** (it sums positional ranks; the actual scores are discarded).

**Impact:** When the keyword lane returns 1 doc and the vector lane returns 50, the keyword doc gets rank 1 → score `1/61 ≈ 0.0164`; the vector doc rank 1 also gets `1/61`. But rank 50 of the vector lane gets `1/110 ≈ 0.0091`, while a *single hit* from the graph lane (rank 1) gets `1/61` — i.e., a single weak graph hit can outrank dense, semantically-correct vector hits. With graph hits scoreless, F-002 compounds into *bad-but-confident* graph results dominating fusion when graph is non-empty.

**Confidence:** High (RRF is canonical, but its known weakness is exactly imbalanced lanes).

**Tracked by:** Closed by `phase6c_relevance_score_aware_rrf` — score-aware blend in `crates/cortex-api/src/fusion.rs` (`fused = α · positional + (1-α) · normalized_native`, default `α = 0.7`); `CORTEX_RRF_ALPHA` / `CORTEX_RRF_K` env knobs stamped on the audit envelope as `fusion_alpha` / `fusion_k`; regression test `weak_graph_hit_does_not_outrank_dense_vector_top3` in `fusion::tests` pins the win condition.

---

## F-006 — Intent selector misses the most common operator phrasings. Default fallback `pre_change_context` is correct in spirit but burns the wrong plan.

**Evidence:** [`crates/cortex-pre-thinking/src/intent_select.rs:21-112`](../../../crates/cortex-pre-thinking/src/intent_select.rs) — 5 keywords for `decision_lookup`, 6 for `similar_problems`, 4 for `law_check`, 5 for `pre_change_context`. Missing common phrasings: *"how does X work"*, *"what is X"*, *"explain X"*, *"show me where X"*, *"find usages of X"*. All these fall through to `pre_change_context`, which runs `edge_artifact_touched_neighbours` (graph lane) — wasteful for a navigational query.

**Impact:** Most navigational / explanatory prompts get a `pre_change_context` plan that fans out to 3 lanes + 4 overlays — the right *retrieval* but wrong *post-fusion*. Decisions overlay activates on a "how does X work" query, surfacing irrelevant decisions to the bundle, eating budget.

**Confidence:** Medium-high.

**Tracked by:** Closed by `phase6d_relevance_intent_table_expansion` — new `Intent::Explain` variant with a 11-keyword routing table + extended phrasings on `decision_lookup`/`similar_problems`/`law_check`; selector now returns `MatchedIntent { intent, trigger }` and the audit envelope stamps `intent_trigger`. Spec updated at `docs/specs/12-pre-thinking-injection.md` §intent selection.

---

## F-007 — Decision overlay derivation is fragile: it requires `extras["decision_id"]` to be stamped on the `LaneHit`, but the live lanes do not stamp it.

**Evidence:** [`crates/cortex-api/src/orchestrator.rs:341-368`](../../../crates/cortex-api/src/orchestrator.rs) — `derive_decisions` filters by `h.extras.get("decision_id")`. The Meili lane ([`crates/cortex-api/src/meili_lane.rs:175-216`](../../../crates/cortex-api/src/meili_lane.rs)) stamps only `extras["source"] = "keyword"` — no `decision_id`. The vector lane ([`crates/cortex-api/src/vectorizer_lane.rs`](../../../crates/cortex-api/src/vectorizer_lane.rs)) similarly does not stamp `decision_id`. Only the legacy `MemoryKeywordLane` test double stamps it from seed data.

**Impact:** With live lanes, `response.results.decisions` is **always empty**, regardless of whether decisions actually matched. Pre-thinking bundles never include the "## Recent decisions you should know about" section in production. The same shape applies to `similar_turns` (requires `extras["turn_id"]`) and `law_id` overlays.

**Confidence:** High — direct read of the projection code.

**Tracked by:** Closed by `phase6b_relevance_lane_extras_contract` (lane-projection contract documented in `docs/specs/11-query-api.md` §Lane projection contract; both live lanes now stamp the contract keys; regression guard at `crates/cortex-api/src/lane_contract.rs`; end-to-end overlay tests in `crates/cortex-api/tests/http.rs`).

---

## F-008 — Relevance is unmeasured. There is no labeled query set, no recall@k / MRR harness, no canary queries that catch regressions.

**Evidence:** [`docs/analysis/cortex/09-risks-and-debt.md:76-82`](../cortex/09-risks-and-debt.md) (R9). No `query_eval` task in `.rulebook/tasks/`. The `query_id` carried through audit ([`crates/cortex-api/src/audit.rs:53-76`](../../../crates/cortex-api/src/audit.rs)) is published to `cortex.events.query_audit` but no consumer reads it for quality scoring.

**Impact:** Every "the bundle feels weak" conversation is qualitative. Fixes for F-001..F-007 cannot be ranked or proven; regressions land silently. Coverage gaps (F-001) and ranking gaps (F-005) look identical from the operator's seat.

**Confidence:** High.

**Tracked by:** R9. **Closed by:** [`phase6e_relevance_recall_mrr_harness`](../../../.rulebook/tasks/phase6e_relevance_recall_mrr_harness/proposal.md) — labeled query set + harness binary + CI gate landed; reports persist to `.rulebook/learnings/relevance/`.
