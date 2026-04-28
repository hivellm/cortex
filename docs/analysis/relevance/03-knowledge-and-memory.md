# 03 — Knowledge entries + memory entry to capture

These are the durable artifacts the analysis surfaces. Capture them via the rulebook MCP (`rulebook_knowledge_add`, `rulebook_decision_create`, `rulebook_memory_save`) when the daemon is reachable; otherwise this file is the source of truth and the next bootstrap run picks it up via `phase4e_bootstrap_analysis_promotion`.

Tag every entry with `analysis:relevance` so future searches surface them next to this file.

---

## Patterns

### `relevance:scope-default-server-side`

Every retrieval call MUST resolve `Scope.repo` server-side. Order of resolution:

1. `request.scope.repo` (explicit) →
2. `x-cortex-repo` header (set by MCP server / dashboard / pre-thinking) →
3. caller-derived cwd → repo slug →
4. `422 scope_repo_required` (no implicit fallback to `cortex-unknown-*`).

**Why:** without enforcement, MCP `cortex_query` calls without a scope route to the `unknown` slug and return zero hits across all three lanes. Operators interpret this as "Cortex is broken" when the cause is route-to-nowhere.

**Source:** F-003.

---

### `relevance:overlay-extras-contract`

Live lanes (Meili, Vectorizer, Nexus) MUST copy upstream `decision_id` / `turn_id` / `law_id` / `model` / `summary` / `decision_status` into `LaneHit.extras`. The orchestrator's overlay derivation (`derive_decisions`, `derive_similar_turns`, etc.) reads from `extras` and silently emits empty arrays when the keys are missing.

**Test obligation:** every new lane impl MUST include an integration test that asserts at least one overlay populates from a fixture with the relevant id present.

**Why:** prevents silent regression — overlays that look fine in unit tests against the in-memory test double but go empty against the live stack.

**Source:** F-007.

---

## Anti-patterns

### `relevance:positional-only-rrf-with-imbalanced-lanes`

Pure positional RRF (`1/(K+rank)`) without lane-score weighting, when one lane returns ≪ another, fuses in a way that lets a single weak hit from the sparse lane outrank dense, semantically-correct hits from the rich lane. The lane-native score is already captured into `LaneHit.score` — discarding it during fusion is a bug, not a design choice.

**Mitigation:** blend lane-native score with positional rank: `score = 0.7·(1/(K+rank)) + 0.3·lane_native_score`. Validate with a "weak graph hit doesn't outrank dense vector top-3" probe.

**Source:** F-005.

---

### `relevance:user-prompt-as-search-query`

Forwarding the verbatim natural-language prompt as the `query` field across all three lanes wastes budget on framing words ("why is", "should we", "can you"). The vector lane embeds the entire question; semantic match is dominated by framing tokens rather than load-bearing technical tokens.

**Mitigation:** rewrite the user prompt into a search query before fan-out (deterministic noun-phrase extractor for the cheap path; one-shot Sonnet for the expensive path). The Sonnet analyzer at `crates/cortex-api/src/analyzer.rs` is already wired — same surface, different prompt.

**Source:** F-004.

---

## ADR candidates

### ADR — Score-aware RRF

**Status:** proposed. Blend lane-native scores into RRF as a weighted tiebreak (e.g. `0.7·positional + 0.3·native`). Tests in `crates/cortex-api/src/fusion.rs` already cover positional invariants — extend with imbalanced-lane probes before merging.

**Why now:** F-005 + F-002 compound. As `phase4c` adds richer graph edges, more graph hits flow into fusion; positional-only RRF will surface bad-but-confident graph results above dense vector top-3.

**Source:** R2 step 5.

---

### ADR — Default scope to caller-derived repo with `x-cortex-repo` override

**Status:** proposed. Make `/v1/query` reject (`422`) requests where scope cannot be resolved, instead of falling through to `cortex-unknown-{family}`.

**Why now:** F-003 silently breaks every MCP `cortex_query` call without an explicit scope. The cost of pausing is one wasted request; the cost of route-to-nowhere is days of "Cortex doesn't work" debugging.

**Source:** R1 step 1.

---

## Memory entry

Save via `rulebook_memory_save` once the MCP daemon is reachable:

- **type:** `observation`
- **tags:** `["analysis", "relevance"]`
- **content:**
  > Cortex relevance is bottlenecked by 8 gaps; 3 already tracked (`phase4a`, `phase4c`, R10), 5 new (F-003 scope default, F-004 query rewriting, F-005 score-aware RRF, F-006 intent coverage, F-007 overlay extras contract, F-008 retrieval harness). Highest-leverage pair: F-003 + F-007 — both 1-day fixes that flip overlays and scope behaviour from broken to working in production; do them BEFORE `phase4a` so coverage uplift is observable.

This memory + the four files in `docs/analysis/relevance/` together form the durable record. The next pre-thinking session that searches `analysis:relevance` will find both.
