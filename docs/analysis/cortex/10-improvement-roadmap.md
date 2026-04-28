# 10 — Improvement roadmap (prioritized)

This file synthesizes [02–09](00-index.md) into an ordered, actionable plan. Each item lists: **what**, **why**, **rough effort**, and **success criterion** (what proves it landed).

## Sequencing principle

> **Coverage → Verification → Topology → Quality → Governance → Reach.**

You cannot evaluate retrieval quality on a partial index. You cannot ship governance on top of an unmeasured retriever. You cannot expand to 17 repos if the 3 you have are silently degraded.

## Sprint 1 — Close the data loop (2 weeks)

### 1.1 Fix Meilisearch fan-out (P1 — phase4a)

- **What:** worker's startup replays missing-repo events from the archive; sweep stale legacy indexes.
- **Why:** keyword lane is single-repo today; phase 4 cannot start without it.
- **Effort:** 2-3 days.
- **Success:** all 3 currently-vectorized repos appear in Meili with non-zero doc counts; six legacy `cortex-{code,decisions,docs,governance,misc,turns}` indexes gone.

### 1.2 Build the consistency doctor (P1 — phase4d)

- **What:** `cortex doctor consistency` subcommand in `cortex-ops`; coverage mode + probe mode; CI integration.
- **Why:** without this, every drift fix can silently regress. Detection-by-accident must end.
- **Effort:** 3-4 days.
- **Success:** running `cortex doctor consistency` against the live stack returns non-zero exit when any backend is missing a partition that another has; CI runs it post-bootstrap.

### 1.3 Vectorizer post-upsert verification

- **What:** after each `insert_texts` batch, sample 1-in-N inserted chunks via `list_stored_chunk_ids` and assert presence.
- **Why:** SDK reports `total_failed=4-5/64` and `vector_count=0` despite vectors being queryable. We need an objective "did it land?" signal.
- **Effort:** 1 day.
- **Success:** embedder logs `verified=N/N` after each batch; mismatches escalate to a warn-level metric.

### 1.4 Embedder JWT auto-login

- **What:** if `CORTEX_EMBEDDER_VECTORIZER_PASSWORD` doesn't look like a JWT (no dots), embedder calls `/auth/login` itself and caches the token.
- **Why:** today this 401s silently; recorded as a follow-up in the 2026-04-27 learning. Small friction with a recurring cost.
- **Effort:** 0.5 day.
- **Success:** plain password env var no longer 401s; embedder boots clean.

## Sprint 2 — Topology depth + repo breadth (2 weeks)

### 2.1 Symbol nodes + DEFINES edges (P2 — phase4c)

- **What:** `cortex-graph::mapper` reads the existing `symbol` field per chunk and emits `(:Symbol)-[:DEFINES]->(:Artifact)`.
- **Why:** highest-leverage / lowest-cost graph improvement — the data is already produced upstream. Unlocks "where is X defined" queries.
- **Effort:** 1-2 days.
- **Success:** Cypher `MATCH (s:Symbol)-[:DEFINES]->(a:Artifact) RETURN count(*)` > 0; doctor probe finds the same `path` via graph as via vector for symbol-level queries.

### 2.2 Multi-repo bootstrap orchestrator (P2 — phase4b)

- **What:** new orchestrator command + accumulating state file + per-repo lockfile.
- **Why:** extends coverage from 3 to 17 repos with one invocation; preserves prior runs in the checkpoint.
- **Effort:** 3-4 days.
- **Success:** running the orchestrator on a clean local stack populates 17 repos; doctor reports parity across all backends; state file shows last-run timestamp per repo.

### 2.3 Worker supervisor / Make targets

- **What:** `make start-workers` brings up all four workers; `make health-workers` asserts each is consuming.
- **Why:** today the four workers are launched ad-hoc; if classifier-worker dies, events pile up silently. Operational gap.
- **Effort:** 0.5 day.
- **Success:** `make health-workers` returns non-zero when any worker is down; documented in README.

## Sprint 3 — Retrieval quality (2 weeks)

### 3.1 Labeled query set + retrieval eval harness

- **What:** ~50 hand-written query/expected-result pairs across the 3-then-17 indexed repos. CI job scores `recall@10` and `MRR` per intent.
- **Why:** Phase-4 hardening line item. Without this, we cannot prove Cortex makes models better. R9 in [09-risks-and-debt.md](09-risks-and-debt.md).
- **Effort:** 3-5 days (including curating the query set).
- **Success:** CI publishes a recall@10 number per release; regressions block merge.

### 3.2 Pre-thinking bundle audit trail

- **What:** persist `query_id` per bundle assembled; expose in dashboard with the input prompt + assembled bundle + downstream model action.
- **Why:** spec 12 promises this; today bundles are ephemeral. Needed for retrospective quality analysis.
- **Effort:** 2-3 days.
- **Success:** dashboard view "Bundles" lists recent assemblies with click-through to full bundle text.

### 3.3 Cross-repo scope default

- **What:** `/v1/query` defaults `scope.repo` to the calling adapter's `cwd`-derived repo; cross-repo recall is opt-in via `scope.repos: ["X","Y"]`.
- **Why:** prevent R10 (cross-repo memory bleed). Today nothing prevents a Vectorizer-repo answer from citing Cortex-repo memories.
- **Effort:** 1-2 days.
- **Success:** integration test: a query from Vectorizer's repo with no scope override does not return Cortex-only memories.

## Sprint 4 — Governance MVP (2 weeks)

### 4.1 Static law registry + evaluator

- **What:** `.cortex/laws/*.yaml`, loaded at boot. `POST /v1/laws/evaluate` accepts a hypothetical tool call, returns matches + severities.
- **Why:** ship enforcement teeth without waiting for the full DSL/Deno sandbox. Detail in [06-governance-gap.md](06-governance-gap.md).
- **Effort:** 4-5 days.
- **Success:** PreToolUse from Claude Code adapter that violates a `severity: critical` rule is rejected; matching and non-matching cases covered by integration tests.

### 4.2 LawViolation write path + trust-score materialization

- **What:** non-blocking matches write `(:LawViolation)-[:OF]->(:Law)` + `[:OBSERVED_IN]->(:ToolCall|:Turn)` into Nexus. Daily job materializes `trust = 1 - (violations_7d / total_calls_7d)` per `(model, repo)`.
- **Why:** dashboard's `/v1/dashboard/trust` is a stub. Make it real.
- **Effort:** 2-3 days.
- **Success:** dashboard "Trust" view shows non-stub data; doctor probe confirms violation nodes land in Nexus.

### 4.3 Dashboard Laws view: real catalog

- **What:** read from `.cortex/laws/*.yaml` registry, not from reverse-engineered violation envelopes.
- **Why:** today's view is misleading (commit `3f8bbe3` derives the catalog from violations).
- **Effort:** 0.5 day.
- **Success:** Laws view shows the registry contents even when violations table is empty.

## Sprint 5 — Reach + ergonomics (open-ended)

### 5.1 Multi-connection GUI (phase3_gui_multi_connection)

- **Effort:** 3-4 days.
- **Success:** GUI ships with `local` connection plus user-defined remotes; auth plumbed; React Query keys scoped per connection.

### 5.2 Dashboard auth (phase2f_dashboard_auth)

- **Effort:** 2-3 days.
- **Success:** `/v1/dashboard/*` requires a bearer token; configurable allowed-origin list.

### 5.3 Cursor / Codex / Gemini adapters (spec 17)

- **Effort:** 1 week each, in parallel.
- **Success:** sessions in those tools emit envelopes; PRD G1 ("100% of AI interactions") becomes meaningfully true.

### 5.4 Backfill ADRs

- **What:** 5-7 ADRs covering the load-bearing implicit decisions identified in [07](07-quality-and-tests.md): Sonnet-vs-Haiku split for classify-vs-analyze, per-row Cypher in response to Nexus drift, Meilisearch as Lexum stand-in, in-memory fallback at boot, env-gated integration tests, etc.
- **Effort:** 2-3 days total (mostly writing).
- **Success:** [.rulebook/decisions/](../../../.rulebook/decisions/) reflects the actual decision log; spec status emojis link to ADRs.

### 5.5 Coverage tooling

- **What:** wire `cargo llvm-cov` into Makefile; CI publishes coverage on PRs.
- **Why:** AGENTS.md asserts ≥95% coverage but no tooling enforces it.
- **Effort:** 0.5 day.
- **Success:** `make coverage` produces a per-crate report; PRs annotated with coverage delta.

## Visibility / housekeeping

These don't need a sprint slot — fold into ongoing work:

- **Archive `phase1_classifier_worker`** — checklist is done; STATE.md is stale.
- **Complete or close `phase3_tool_call_hash_preview`** — proposal is template-only.
- **Update `architecture.md §5.2.1`** to reflect Static + Sonnet as the default classification path; Haiku CLI as experimental.
- **Annotate spec status emojis** with verified-by-evidence lines (link to consistency-doctor reports once they exist).
- **Update CLAUDE.md / AGENTS.md** if any quality bar shifts as a result of this analysis.

## Sequencing in one sentence

> Sprint 1 closes the data loop and proves it; Sprint 2 deepens topology and breadth; Sprint 3 measures retrieval quality; Sprint 4 lands governance teeth; Sprint 5 is reach + ergonomics. After Sprint 4 the platform delivers on the architecture promise — capture, classify, retrieve, and **govern** — across the actual repo footprint.
