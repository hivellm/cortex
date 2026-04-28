# 02 — Execution plan

Four phases. Each phase fits in one sprint (~5 days). The ordering is load-bearing: the harness (R3) must land before R4 so improvements can be attributed; the leverage fixes (R1) must land before `phase4a` so the coverage delta from `phase4a` is observable.

---

## Phase R1 — Stop the bleed (highest leverage, ~5 days)

| # | Action | Closes | Effort | Notes |
|---|--------|--------|--------|-------|
| 1 | **Default `Scope.repo` server-side.** In `crates/cortex-api/src/service.rs` (the `/v1/query` handler), reject requests where `scope.repo` is `None` *and* the caller is not pre-thinking — or default to a header (`x-cortex-repo`) the MCP server / dashboard already supplies. Return `422` + spec-11 `reason: "scope_repo_required"`. | F-003 | 1–2 h | Largest single uplift. |
| 2 | **Stamp overlay extras on live lanes.** Extend `MeiliKeywordLane::project` and the `VectorizerLane` projection to copy `decision_id`, `turn_id`, `law_id`, `model`, `summary`, `decision_status` from the upstream document body / payload extras into `LaneHit.extras`. Re-run `derive_decisions` / `derive_similar_turns` integration tests against a live stack to confirm overlays populate. | F-007 | ~1 day | Flips `response.results.decisions` / `similar_turns` from empty-in-prod to populated. |
| 3 | **Land `phase4a` Meili fan-out.** Already proposed; merge once R1.1 + R1.2 are in so the coverage delta is observable. | F-001 | (already scoped) | Vanishes the moment Rulebook + Vectorizer indexes have docs. |

**Exit criterion for R1:** a probe set of 10 prompts across the 3 indexed repos produces non-empty `decisions` / `similar_turns` overlays where applicable, and zero `cortex-unknown-{family}` route attempts in the audit envelope.

---

## Phase R2 — Make graph and ranking pull their weight (~5 days)

| # | Action | Closes | Effort | Notes |
|---|--------|--------|--------|-------|
| 4 | **Land `phase4c` `(:Symbol)-[:DEFINES]->(:Artifact)`.** Already proposed. | F-002 | (already scoped) | Graph lane starts answering symbol-level questions; `pre_change_context` graph leg becomes useful. |
| 5 | **Score-aware RRF.** Feed lane-native scores into RRF as a tiebreak weighted with positional rank, e.g. `score = 0.7·(1/(K+rank)) + 0.3·lane_native_score`. Tests in `crates/cortex-api/src/fusion.rs` already cover positional invariants — extend with a "weak graph hit doesn't outrank dense vector top-3" probe. | F-005 | ~1 day | Lane-native scores already captured into `LaneHit.score` — they just aren't used. |
| 6 | **Intent table coverage.** Add `how does`, `what is`, `explain`, `where is`, `find usages` → either a new `Intent::Explain` (preferred — its plan is vector + keyword on `code` + `docs`, no decision/law overlays) or route to `free_search` instead of falling through to `pre_change_context`. | F-006 | half-day | Stops navigational prompts from spending budget on overlays. |

**Exit criterion for R2:** the same 10-prompt probe set produces graph neighbours for symbol-level prompts, and fused order on imbalanced-lane probes does not put weak graph hits above dense vector top-3.

---

## Phase R3 — Measure or it didn't happen (~5 days)

| # | Action | Closes | Effort | Notes |
|---|--------|--------|--------|-------|
| 7 | **Relevance harness.** Build a labeled set of ~50 query / expected-doc-id pairs covering the 5 intents. Wire a CI job that scores `recall@10` and `MRR` per intent, gates merge if delta worse by >2%. Persist scores in `.rulebook/learnings/`. | F-008 | ~3 days | Without this, R4 cannot be evaluated. The `query_id` audit envelope already carries everything the scorer needs. |
| 8 | **Query rewriting pre-pass.** Add a one-shot Sonnet (or even a deterministic noun-phrase extractor) pre-pass that rewrites the user prompt into a search query before fan-out. The Sonnet analyzer at `crates/cortex-api/src/analyzer.rs` is already wired — same surface, different prompt. Score uplift via the harness from step 7. | F-004 | ~2 days | Block on the harness so the win can be measured. |

**Exit criterion for R3:** every PR that touches a retrieval-path file shows a `recall@10` / `MRR` delta in CI; a regression > 2% blocks merge.

---

## Phase R4 — Polish (open-ended)

| # | Action | Effort | Notes |
|---|--------|--------|-------|
| 9 | **Coverage indicator in pre-thinking metadata.** Surface `coverage = {repo, vector_docs, meili_docs, graph_nodes}` so the model knows *why* a bundle is thin (R3 mitigation in `09-risks-and-debt.md`). | half-day | Lets the LLM degrade gracefully when a backend is empty. |
| 10 | **Backfill ADRs** for the per-row Cypher / static-classifier / Sonnet-analyzer / Meili-as-Lexum decisions (per `10-improvement-roadmap.md` §5.4) — these are load-bearing implicit decisions that future relevance work will trip over. | 1–2 days | Use `rulebook_decision_create` so they index alongside other ADRs. |

---

## Sequencing rationale

- **R1 before `phase4a`.** If `phase4a` lands first, F-007 (empty overlays in prod) hides the coverage uplift — bundles still feel weak because decisions never surface. Fix F-007 first so `phase4a`'s impact shows.
- **R3 before R4 step 8.** Query rewriting is the highest-variance change here; without the harness from R3 step 7 there is no honest way to ship it.
- **R2 step 5 before R2 step 4.** Score-aware RRF gates whether richer graph edges (R2 step 4) help or hurt fusion. If RRF is positional-only, more graph hits push the ranking around in unpredictable ways.

## What this plan does not cover

- Multi-tenant relevance (per-caller bias). Out of scope until a second adapter beyond claude-code lands.
- Personalization / re-ranking based on session history. Pre-thinking carries `similar_turns` but no per-session learning yet — separate roadmap item.
- Cross-repo retrieval. Today's design is repo-scoped by intent; cross-repo would need a new `Intent::CrossRepo` plus changes to the intent table that we're explicitly *not* making here.
