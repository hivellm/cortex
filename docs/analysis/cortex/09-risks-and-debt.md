# 09 — Risks, drifts, and structural debt

Distinct from "what's broken right now" (covered in [02](02-pipeline-state.md) and [03](03-data-quality.md)) — this file is about **patterns of fragility**: things that aren't necessarily failing today but will fail again unless structurally addressed.

## R1 — Silent-success drifts in upstream services

The recurring failure mode across this codebase: **upstream returns 200 OK; data doesn't actually persist**. Captured instances:

- Vectorizer 3.0.x `/upsert` reports `total_failed=4-5/64`, `vector_count=0` — vectors *do* end up queryable in some cases but the response contract is misleading.
- Nexus 1.15 `UNWIND` writes return success; rows don't land.
- Vectorizer 3.0.x `/get_vector/{id}` returns synthetic 200 for any id.
- Meilisearch settings PATCH succeeds even with extra unknown fields *until* it doesn't, on a tooling-only field added later.

Each was caught by **post-write read-back** or **operator audit**, never by the SDK call itself. The architectural lesson: in Cortex, **never trust the upstream success signal alone**. Always verify a sample, especially on write paths.

**Mitigation:** the pattern is already in place for Nexus (`assert_write_landed`, commit `5bd0185`). Apply the same shape to Vectorizer upsert and Meili settings.

## R2 — Detection-by-accident

The 2026-04-27 audit was a 90-minute manual session: curl the four backends, decompress the zstd archive in Python, eyeball a per-`(repo, family)` table. That is how phase4a (Meili fan-out) and phase4c (graph edge poverty) were both surfaced. Both had been silently broken for **weeks** — the operator's pre-thinking bundles were thinner than they should have been; nobody noticed because the lanes degrade gracefully (in-memory fallback at boot, low-score returns instead of empty).

**Risk:** the next regression after phase4a/b/c will be discovered the same way unless `cortex doctor consistency` is built and wired into CI.

**Mitigation:** [phase4d](../../../.rulebook/tasks/phase4d_indexing_consistency_doctor/proposal.md) — already proposed, treat as P1 alongside phase4a.

## R3 — Asymmetric coverage masquerading as quality issues

When the keyword lane only knows about Cortex, queries about Rulebook fall to the vector lane only, and the BM25-as-embedding score floor is so low (top-1 = 0.136 on the audit probe) that bundles look "weak". The user-visible failure mode is "Cortex's pre-thinking is bad" when the actual fault is "Cortex's index is partial."

**Risk:** every retrieval-quality conversation gets contaminated by coverage gaps. The fix is to gate any retrieval-quality benchmark on the doctor reporting `parity=full` first.

**Mitigation:** sequence of phase4a → phase4d (verify) → phase4b (extend to 17) → only then evaluate retrieval quality.

## R4 — Bootstrap state file as single-repo singleton

[.cortex-bootstrap.state.json](../../../.cortex-bootstrap.state.json) is overwritten per `cortex-bootstrap` invocation. The current state shows 4 repos — earlier runs walked Vectorizer (we see vectors for it) but the checkpoint was lost. This means there is **no durable record** of which repos have been backfilled and how recent each is.

**Risk:** "is repo X up to date?" cannot be answered today without re-walking it. With 17 repos, this becomes prohibitive.

**Mitigation:** phase4b adds the orchestrator that maintains a multi-repo checkpoint. In the meantime, the state file should accumulate (merge) rather than overwrite.

## R5 — `cortex-classifier-worker` running as foundational infra without a supervisor

Per ADR 002, the worker is now its own crate. It must be running for any new event to flow from raw → enriched. There is no `systemd` unit, `Procfile`, or operator command in the repo to ensure it's up. Today the operator launches it manually next to the other workers.

**Risk:** worker dies → events queue silently in Synap → operator notices days later when bundles get stale. Synap is durable so data is not lost, but live capture appears to "stop working" until the worker is brought back.

**Mitigation:** add a `make start` target that brings up all four workers (classifier-worker / embedder / graph / fulltext) and a `make health` that asserts each one is consuming. Optional: a `cortex-supervisor` crate.

## R6 — Per-event Haiku classification has low ROI

Note in [analyzer.rs:9-12](../../../crates/cortex-api/src/analyzer.rs): "Per-event Haiku-grade classification was producing tags with no lift." In response, the team built the Sonnet cross-event analyzer. The Haiku path is still there (opt-in via `CORTEX_CLASSIFIER_MODE=cli`).

**Risk:** keeping a low-ROI path in the codebase invites confusion ("which mode is correct?") and incurs cost when accidentally enabled. The architecture's classification chapter ([architecture.md §5.2.1](../../architecture.md)) was written assuming Haiku-per-event; reality is Static-per-event + Sonnet-per-session.

**Mitigation:** update `architecture.md §5.2.1` to reflect the Static + Sonnet split as the **default** path, with Haiku CLI as an experimental knob. Reduce the surface mentally and in docs.

## R7 — Specs flagged 🟢 don't all match implementation reality

- Spec 16 (Dashboard) is 🟡 in the index but the GUI ships 9 working views.
- Spec 14 (Governance) is 🟡 but the dashboard renders Laws/Violations.
- Spec 11 (Query API) is 🟢 but the `cortex doctor consistency` evidence that the lanes return symmetric results does not exist.

**Risk:** "🟢 means tested in production" gets diluted. New contributors reading the index over-trust the labels.

**Mitigation:** add a fourth status emoji "🟠 partial" or annotate 🟢 specs with a "verified-by" line linking to the consistency-doctor evidence (once it exists).

## R8 — Adapter coverage is single-tool

Only Claude Code is wired. PRD G1 ("capture 100% of AI interactions across every supported tool") is not met for sessions where the user works in Cursor, Codex, or Gemini — those don't emit anything to Cortex.

**Risk:** institutional memory has a Claude-shaped hole. If 30% of work happens in another tool, 30% of decisions are lost.

**Mitigation:** spec 17 (Cursor / Codex / Gemini adapters) — Phase 3. Until then, document the limitation visibly so users know what's captured.

## R9 — Pre-thinking bundle quality is unmeasured

Spec 12 mentions a `query_id` carried through the bundle for "retrieval-quality analysis". Today no analysis runs against those query_ids; there is no `query_eval` task in [.rulebook/tasks/](../../../.rulebook/tasks/).

**Risk:** the model's reaction to bundles ("did it cite the right decision?", "did it avoid the law violation?") is not measured. The platform's job-to-be-done is invisible.

**Mitigation:** Phase-4 hardening should include a labeled query set + a CI job that scores bundles per intent. Until that exists, **we cannot prove Cortex is making models better** — only that it captures and indexes successfully.

## R10 — User-facing memory leakage between repos

The user expressed worry about cross-tool / cross-repo memory becoming stale or contradictory. The architecture's `Memory` entity is part of the graph, but the dashboard doesn't surface "this memory was written about repo X but is being recalled in repo Y". The Bootstrap promotes `.rulebook/memory/**/*.md` from each walked repo into Cortex as memory artifacts — fine — but cross-repo recall isn't filtered.

**Risk:** an answer to a Vectorizer question accidentally cites a Cortex-specific memory.

**Mitigation:** scope-aware retrieval (already partially in place — see [11-query-api scope](../../specs/11-query-api.md)) needs to default to "current repo" and require an explicit opt-in for cross-repo recall. Worth verifying behavior + adding a regression test.

## Risk register summary

| Risk | Severity | Trend | Mitigation owner |
|------|----------|-------|------------------|
| R1 — Silent-success drifts | High | Stable (3 known instances) | post-write verification, doctor |
| R2 — Detection-by-accident | High | Improving (phase4d proposed) | phase4d |
| R3 — Coverage masquerading | High | Improving (phase4a in queue) | phase4a + phase4d |
| R4 — Single-repo bootstrap state | Medium | Stable | phase4b |
| R5 — Worker supervisor gap | Medium | Stable | Makefile / supervisor |
| R6 — Haiku low ROI | Low | Stable | doc update |
| R7 — Spec status drift | Low | Stable | annotate 🟢 with evidence |
| R8 — Single-adapter coverage | Medium | Open | spec 17 |
| R9 — Pre-thinking quality unmeasured | High | Open | retrieval eval harness |
| R10 — Cross-repo memory bleed | Medium | Unknown | scope default + test |
