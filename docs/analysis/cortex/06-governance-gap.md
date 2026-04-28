# 06 — Governance gap (laws, violations, trust)

## What the architecture promises

[architecture.md §5.4](../../architecture.md) and [prd.md §G5](../../prd.md):

- **Laws DSL + sandboxed detectors** (Deno) — spec 13.
- **Governance engine** with blocking + observational modes, punishment ladder, per-(model, repo) trust score — spec 14.
- **Adapter integration** so critical laws reject offending tool calls at PreToolUse — spec 10 + 14.

These are the teeth of the platform. The whole "force consultation of Cortex before non-trivial code change" use case (US-02 in PRD) is gated on this.

## What exists today

| Component                            | State                                   | Notes                                                                |
|--------------------------------------|-----------------------------------------|----------------------------------------------------------------------|
| Spec 13 (Laws DSL + detector)        | 🟡 **Draft**                             | Spec exists; no implementation in `crates/`.                         |
| Spec 14 (Governance engine)          | 🟡 **Draft**                             | Same.                                                                 |
| Detector sandbox (Deno)              | ❌ Not built                             | No crate, no runtime wiring.                                          |
| Punishment ladder                    | ❌ Not built                             | No data model, no engine.                                             |
| Per-(model, repo) trust score        | ❌ Not built                             | `/v1/dashboard/trust` is a stub.                                      |
| `LawViolation` Nexus nodes (72)      | ✅ Present                               | Sourced from bootstrap-promoted `.claude/rules/*.md` and historical log envelopes — **not** from a live engine. |
| `Law` catalogue render in dashboard  | ✅ Derived                               | Commit `3f8bbe3` derives the law catalogue *from* `law_violation` envelopes, not from a registry. |
| Adapter PreToolUse hook contract     | ✅ Wired (spec 10)                       | Receives the call, but has nothing to evaluate against.               |

## What this means in practice

1. **Cortex is observational-only today.** It captures everything, summarizes, retrieves — but does not enforce anything. The `.claude/rules/*.md` files in this very repo are honored because Claude Code reads them itself, not because Cortex evaluates them.
2. **The dashboard's Laws view is misleading.** It shows a catalogue, but the catalogue is reverse-engineered from past violation envelopes, not from a registry of active laws. A maintainer reading that view would believe the system is enforcing rules; it is not.
3. **Trust score is a phantom field.** The route exists, the GUI references it, the data is empty.
4. **Phase 2 cannot close** until specs 13–14 are implemented. The roadmap's Phase 2 budget was 4 weeks; current Phase 2 work is GUI design parity (phase2a–h) — also valuable, but not the governance teeth phase 2 was supposed to deliver.

## Why this matters more than the indexing drifts

Indexing drifts (phase4a/b/c) degrade *recall*; the system still works, just less well. The governance gap means Cortex cannot do the **one thing** that makes it different from "yet another vector store of AI logs": automated, auditable enforcement of dev rules. Without that, the architecture's NG2 ("Cortex is not a coding agent — it informs the agents that code") still leaves Cortex with no teeth.

## Minimum viable governance (what would close the gap)

A pragmatic Phase 2 closure that lands governance without trying to ship the whole spec at once:

1. **Static law registry** under `.cortex/laws/*.yaml`, loaded at boot. No DSL, no Deno sandbox — just YAML rules with a `predicate: regex` or `predicate: starts_with` and a `severity`.
2. **`POST /v1/laws/evaluate`** endpoint: accepts a hypothetical tool call, returns matching laws + severities. Adapter calls this from PreToolUse and blocks on `severity: critical`.
3. **`LawViolation` write path** in `cortex-graph` for any non-blocking match — the data model is already there (72 nodes prove it).
4. **Trust score = simple ratio** initially: `1 - (violations_last_7d / total_tool_calls_last_7d)` per `(model, repo)`. Materialize daily.
5. **Defer the Deno sandbox + DSL** to a v2.

This gives the platform real enforcement teeth in 1-2 weeks instead of 4, at the cost of static-only rules. The DSL/Deno path can come later as a feature, not a blocker.

## Recommendation

Treat the governance gap as the highest-priority Phase-2 work *after* the indexing drifts are closed. The order matters: shipping enforcement against a single-repo index would advertise teeth that only bite within the one repo we happen to have indexed.

Sequence:

1. Close phase4a (Meili fan-out) so all 3 indexed repos serve all backends.
2. Close phase4d (consistency doctor) so we can prove the fix landed.
3. Close phase4b (17-repo orchestrator) so coverage is real.
4. Then ship MVP governance per the 5 steps above.
5. Then iterate on the DSL.

This sequencing keeps the platform's *visible* state honest at every step — we never ship enforcement without coverage.
