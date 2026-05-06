# 01 — Consolidation pipeline review

> **User pain**: "consolidation doesn't work, the data so far doesn't
> result in anything actually relevant."
>
> **Verdict**: pipeline has all the pieces but two are **disconnected**
> (no trigger daemon; output lost into nonexistent ingest URL) and two
> are **cosmetic** (tests don't validate semantics; consolidations don't
> re-enter retrieval). Result: generates nothing → indexes nothing →
> search does not improve.

---

## Observable symptoms

1. Dashboard `/v1/dashboard/consolidations` loads but data is not
   relevant.
2. Consolidations are never produced automatically in production (no
   daemon, only manual CLI `cortex-consolidator nightly`).
3. Even when generated manually, they don't feed turn/decision retrieval.
4. Fidelity tests pass but don't validate semantic quality of summaries.
5. GUI "Consolidations" tab shows stale data without a "regenerate"
   button.

---

## Findings

### Finding 1 — No consolidator worker daemon in production
- **File**: `crates/cortex-workers/src/bin/` (binary list)
- **Problem**: only the `cortex-consolidator` CLI exists. No daemon
  responds continuously to `Trigger::SessionEnd` /
  `Trigger::NightlyTopic` / `Trigger::DecisionLanded`.
- **Evidence**: 10 binaries in `src/bin/` — `cortex-ingestion`,
  `embedder`, `fulltext-indexer`, `graph-writer`, `classifier-worker`,
  `cortex-topic-cards`, etc. **None** is a consolidator daemon. Triggers
  defined in `consolidator/orchestrator.rs:18-41` are only consumed by
  the CLI. `run_nightly()` in `cortex-consolidator.rs:492+` is an async
  function executed manually.
- **Severity**: **P0** — without a trigger, consolidations are never
  produced automatically.

### Finding 2 — Consolidator output flow broken (envelope vanishes)
- **File**: `crates/cortex-workers/src/bin/cortex-consolidator.rs:500-600`
  (`publish_consolidation`)
- **Problem**: sends envelope to `ingest_url` (default
  `http://127.0.0.1:17010`). Without an explicit URL configured in
  production, envelopes are silently dropped.
- **Evidence**: line 78-79 default = localhost; line 248-261
  `resolve_ingest_url()` returns `None` when string is empty. No
  fallback to local file or stdout. `publish_consolidation()` is never
  reachable when `ingest_url = None`.
- **Severity**: **P0** — produced consolidations are lost before
  reaching Meili.

### Finding 3 — Consolidations don't influence retrieval
- **File**: `crates/cortex-workers/src/fulltext/builders.rs:249-270` +
  `crates/cortex-api/src/meili_loader.rs:48` / `:227-228`
- **Problem**: consolidations live in a separate index
  (`cortex_consolidations`), don't feed turn/decision search. No
  relevance boost, no bidirectional feedback loop.
- **Evidence**: isolated routing — `Kind::Consolidation =>
  "consolidations"` (`routing.rs:66`). Vectorizer pushes them to a
  separate collection (`cortex-{slug}-consolidations`). No reverse link:
  consolidation doesn't list which turns it summarized; turn doesn't
  list consolidations covering its session.
- **Severity**: **P1** — consolidations exist but don't improve user
  experience.

### Finding 4 — Fidelity IT passes but doesn't catch hallucinations
- **File**: `crates/cortex-workers/tests/consolidator_consolidation_fidelity_it.rs:168-220`
- **Problem**: 7 structural invariants (non-empty takeaways, source_ids,
  title length, summary length range). None validates whether a takeaway
  is semantically true.
- **Evidence**: takeaway "ef_search = 128 holds recall@10 ≥ 0.92" passes
  because it's < 280 chars, not because it's a fact. "LLM-as-judge mode"
  mentioned (line 28-33) requires `ANTHROPIC_API_KEY` — never runs in
  default CI. `CannedSummariser` returns hardcoded JSON.
- **Severity**: **P1** — green tests ≠ working feature.

### Finding 5 — Already-digested cascade incomplete
- **File**: `crates/cortex-workers/src/retention/turn_digest.rs:194-345`
  vs `crates/cortex-workers/src/consolidator/source/session.rs:40-53`
- **Problem**: commit `694958a` only fixed the cascade for
  `tool-call-digest`. Consolidator has no equivalent flag; always
  re-processes the entire session even when part was already
  consolidated before.
- **Evidence**: `turn_digest.rs:311` tracks `already_digested`;
  `tool_call_digest.rs:325` same. `consolidator/source/session.rs:40-53`
  loads the WHOLE session without checking. No flag in
  `ConsolidationPayload`.
- **Severity**: **P2** — wasted LLM budget on re-processing.

### Finding 6 — Dashboard without operational capability
- **File**: `crates/cortex-api/src/dashboard/consolidations.rs` (new)
- **Problem**: only GET `/consolidations` and GET
  `/consolidations/{id}`. No POST trigger, no orchestrator/cost-ledger
  status endpoint.
- **Evidence**: `gui/src/views/Consolidations.tsx:21-34` refetches every
  60s but no "generate now" button. Operator has no way to force
  regeneration.
- **Severity**: **P2** — cosmetic UI.

### Finding 7 — Orphan retention layer (nothing to retain)
- **File**: `crates/cortex-cli/src/bin/cortex-ops/consolidation.rs:1-237`
- **Problem**: `consolidation-prune` loads consolidations from Meili and
  demotes them across tiers (hot → warm → cold → expired). Without an
  automatic generator (Finding 1), there's nothing to demote.
- **Evidence**: `consolidation.rs:112-123` (dry-run) only operates on
  pre-existing consolidations. Pruner waits on input that never arrives.
- **Severity**: **P1** — feature ready with no data to process.

---

## Hypotheses for "doesn't work" (ranked)

1. **Consolidator never runs in production** (P0) — without a daemon +
   automatic trigger, it only exists when the operator runs the CLI
   manually.
2. **Consolidations generated but discarded** (P0) —
   `publish_consolidation()` posts to a nonexistent localhost; nothing
   reaches Meili/Vectorizer.
3. **Consolidations isolated from retrieval** (P1) — separate index, no
   boost, no bidirectional links; user never sees the summary.
4. **Generic/cliché summaries** (P1) — template + prompt may be too
   generic; output lacks session-specific insight.
5. **Already-digested logic missing** (P2) — re-processing and budget
   waste once daemon turns on without the fix.

---

## Rework plan (5 phases)

### Phase 1 — Observability (gate: logs + metrics visible)
- Add ERROR logs in `publish_consolidation()` when URL is None or HTTP
  fails.
- Add `/v1/health/consolidations` exposing `last_run_at`,
  `cost_ledger_total`, `consolidations_seeded` (via meili_loader report).
- Run `cortex-consolidator nightly --dry-run` manually; capture
  stdout/stderr for baseline.
- **Verify**: `curl /v1/health/consolidations` returns JSON with
  `consolidations_seeded > 0` or `error: "no runs yet"`.

### Phase 2 — Cron automation (gate: nightly batch produces consolidations)
- Build daemon `cortex-consolidator-worker` running `nightly` every 24h
  + responding to `SessionEnd` hook for 1:1 consolidation.
- Respects `CostBudget` cap (env `CORTEX_CONSOLIDATOR_BUDGET_CENTS`).
- Ingest URL fallback: if `CORTEX_INGESTION_URL` unset, write to
  `.cortex/consolidations.jsonl` + STDOUT.
- Orchestrator records cost ledger to syslog/Prometheus exporter.
- **Verify**: daemon runs, `cortex_consolidations` Meili index grows,
  health endpoint shows recent timestamps.

### Phase 3 — Semantic validation (gate: takeaway verification)
- Lightweight LLM-as-judge: Haiku scores each takeaway against
  `source_event_ids` (threshold ≥ 90% relevance).
- Reject consolidation if any takeaway < 80% (re-summarize once with
  Opus).
- Add `semantic_fidelity_score` (0-100) to `ConsolidationPayload`.
- **Verify**: fidelity IT passes with `ANTHROPIC_API_KEY=...`; rejected
  consolidations are not published.

### Phase 4 — Context integration (gate: consolidations improve search)
- Hybrid search: boost relevance score 1.5x for consolidations covering
  the query's temporal window.
- Bidirectional links: consolidation → source_session_ids; session lists
  consolidations covering it.
- Topic consolidations feed `cortex-topic-cards` downstream.
- **Verify**: golden-set query returns consolidation as top-3 hit;
  drill-down shows source sessions.

### Phase 5 — Rollout & monitoring (gate: production metrics OK)
- Deploy to staging; monitor cost ledger 7 days.
- Conservative `CostBudget::monthly_cents_cap` ($50/mo initially).
- Alert: cost > 1000 cents/run or `fidelity_score < 80` in > 10% of
  runs.
- **Verify**: 14 days production, < 2 escalations, dashboard shows
  consolidations with verified boost.

**Critical path**: Phase 1 → 2 → 4. Phases 3 and 5 are hardening/rollout.
