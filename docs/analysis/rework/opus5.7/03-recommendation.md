# 03 — Recommendation: feature freeze + Phase A++ + 4-week checkpoint

> Concrete strategic call. Reads stand-alone if you skipped 01 and 02.

---

## The call

**Adopt the prior medium-rework recommendation, with three modifications:**

1. **Freeze new MCP tools, new lanes, and new dashboard surfaces until
   Phase A's gates close.** Tactical patches (the 7 in the prior
   README) are fine; new feature work is not. Specifically: defer
   `phase11v_mcp-fine-grained-backend-search` until ADR-011 +
   Phase A.3 land.
2. **Expand Phase A from 4 traits to 6 work items** — add `cortex-config`
   crate (A.5) and dashboard-reader migration (A.6, formerly B.3).
3. **Add a 2026-06-01 mid-checkpoint on A.1 + A.2.** If Sweep +
   EnvelopeProducer aren't green by then, the medium-vs-large discussion
   reopens 2 weeks earlier than the 2026-06-15 outer review.

**Add 2 ADRs** (beyond the prior 7): ADR-016 (schema evolution) and
ADR-017 (ingestion redaction).

**Total scope**: 6 weeks, 9 ADRs, 3 phases. Same horizon as the
prior recommendation; tighter sequencing.

---

## Concrete sequence

### Pre-Phase A (this week — 2026-05-05 → 2026-05-11)

**Tactical patches that don't block on the trait work.** Each is <2d,
none touches abstractions:

| # | Patch | Severity | Effort | Owner |
|---|-------|----------|--------|-------|
| 1 | Wire ERROR log + `.cortex/consolidations.jsonl` fallback in `publish_consolidation()` | P0 (data loss) | 0.5d | implementer |
| 2 | Flip cron default to `--purge-originals` for tool-call-digest | P1 | 0.5d | implementer |
| 3 | Extract `is_live_partial_frame()` to `cortex-storage/src/archive_purge.rs`, apply to digest purgers | P1 | 1d | implementer |
| 4 | Build `cortex-ops retention-archive-purge --before <RFC3339>` (no cron yet) | P0 | 2d | implementer |
| 5 | Run boot-time meili index audit: confirm cortex-rulebook-* / cortex-vectorizer-* doc counts | HIGH | 0.5d | researcher |
| 6 | Inventory the 117 archived tasks (categorize, mark dead-code candidates) | P1 (hygiene) | 1d | researcher |

**Constraint**: NO new MCP tools, NO new lanes, NO new dashboard
routes. The active `phase11v` task is paused.

**Gate to Phase A**: patches 1–4 merged + audit reports for 5 + 6
in `docs/analysis/rework/opus5.7/appendix/`.

---

### Phase A — Codify abstractions (2 weeks, 2026-05-12 → 2026-05-25)

| # | Trait / crate | Description | Gate |
|---|----|--------|------|
| A.1 | `Sweep` trait | Uniform contract for retention/digest/pruning. 7 sweeps migrate. Each writes `retention_sweeps` row per invocation. | Dashboard reads only `retention_sweeps`; IT proves each sweep produces exactly one row per execution. |
| A.2 | `EnvelopeProducer` trait | Bootstrap, claude-archive, topic-cards-emit, consolidator-emit. `produce(ctx) -> Stream<Envelope>` + `checkpoint(ctx) -> ProducerCheckpoint`. | Bootstrap + claude-archive migrated; checkpoint table accumulates (does not overwrite); resume-after-kill IT passes. |
| A.3 | `Lane` trait + typed `ProjectedHit` | Replace `extras: HashMap<String, Value>` with typed struct. Remove all `extras.get(...)` from `orchestrator.rs::derive_*`. | Compiler enforces overlay correctness; regression test covers empty overlay across at least 3 lanes. |
| A.4 | `EventIdentity` + `IdentityIndex` | `{event_id, nexus_id?, vec_id?, meili_id?}` in `cortex-storage`, SQLite-backed. Forget/dedup/doctor/retention move to it. | `cortex doctor consistency` rewritten; one full run < 10s for 100k events. |
| A.5 | `cortex-config` crate (NEW) | Single typed `Config` struct subsumes `cortex.toml` + 344 `CORTEX_*` env vars. Every new feature binds to `Config::*`, not `std::env::var`. | `cortex-ops doctor config-audit` reports 0 ad-hoc env-var reads in `cortex-api` + `cortex-workers`. |
| A.6 | Dashboard becomes pure reader (PROMOTED FROM B.3) | All "what's the state" logic moves to `Sweep::report() / Consolidator::report() / Coverage::report()`. Dashboard handler only renders. | Hardcoded `"never"` impossible by construction (no string literals in handler matching `never|n/a|unknown`). |

**Sequencing inside Phase A**:
- A.1 + A.5 + A.6 land together (one sprint, 1 week). Dashboard
  correctness ships in lockstep with the trait, not as a follow-up.
- A.2 + A.4 land together (one sprint, 1 week). EnvelopeProducer
  needs `EventIdentity` to checkpoint usefully.
- A.3 lands last in the phase but **is the gate for unfreezing
  feature work** — it's what `phase11v` will build atop.

**2026-06-01 mid-checkpoint**: A.1 + A.2 must be green. If not, halt
Phase A and reopen the medium-vs-large discussion.

---

### Phase B — Rewrite ad-hoc subsystems atop the new traits (2 weeks, 2026-05-26 → 2026-06-08)

| # | Subsystem | Description | Gate |
|---|----|--------|------|
| B.1 | Consolidator → `Consolidator` trait + 3 grain impls | Centralized cost telemetry. Daemon (not just CLI) wired to `Trigger::SessionEnd / NightlyTopic / DecisionLanded`. Output via `EnvelopeProducer`. | Daemon binary in `crates/cortex-workers/src/bin/`. `cortex_consolidations` Meili index grows nightly. Health endpoint shows recent timestamps. |
| B.2 | Pruning → `Sweep` impls | Collection-level pruning (re-encode-and-replace, per ADR-013). Expired-tier cascades to all backends via `IdentityIndex`. | Consolidation `age > 365` after pruner run: event_id absent in Nexus + Meili + Vectorizer + archive. |
| B.4 | Golden-set harnesses (NEW, generalized from prior B.4) | Three CSVs + `cortex-eval` subcommands: retrieval, consolidation, classification. CI gate blocks regression > 5% on each. | `cortex-eval --suite retrieval` MRR@10 ≥ 0.60 / recall@5 ≥ 0.50; equivalents for consolidation + classification. |
| B.5 | GUI/backend contract test (NEW) | `gui/src/lib/api.ts` generated from Rust route signatures (or contract test diffs them). | CI fails when Rust route signature drifts from TS types. |

(B.3 absorbed into Phase A as A.6.)

**Phase B unblocks `phase11v`**: 3 MCP tools shipped as `impl Lane`,
not as ad-hoc proxies.

---

### Phase C — Coverage + relevance closure (2 weeks, 2026-06-09 → 2026-06-22)

Same as the prior recommendation:

- C.1: Bootstrap multi-repo via accumulating `EnvelopeProducer::checkpoint`
- C.2: Golden-set retrieval harness gates releases (already in B.4)
- C.3: New adapters (Codex/Cursor/Gemini) — free as `impl EnvelopeProducer`
- C.4: Graph mapper edge expansion (`CALLS`, `IMPORTS`, `DEFINES`,
  `RETURNS`, etc.) — landed against the populated graph schema

**2026-06-15 outer review**: full Phase A + B retrospective. Decide
whether to extend Phase C, declare done, or open the next rework
horizon.

---

## ADRs to land (9 total)

7 from the prior analysis + 2 new:

| ADR | Title | Phase |
|-----|-------|-------|
| 009 | `Sweep` trait as single contract for retention/digest/pruning | A.1 |
| 010 | `EnvelopeProducer` trait | A.2 |
| 011 | Typed `ProjectedHit` replaces `extras: HashMap` | A.3 |
| 012 | `EventIdentity` cross-backend join key + SQLite `IdentityIndex` | A.4 |
| 013 | Vectorizer pruning is collection-level until SDK 3.2 | B.2 |
| 014 | Dashboard handlers pure readers; state in domain reports | A.6 |
| 015 | `cortex-api` crate split (api-http / runtime / daemons) — reversible | (post-C, optional) |
| **016** | **Schema-evolution policy** (NEW — see blind-spot §4) | A.5 |
| **017** | **Ingestion-time redaction policy** (NEW — see blind-spot §9) | parallel |

Each must carry **explicit trade-off** per `AGENTS.override.md` Tier 0.

---

## What changes if I'm wrong about the freeze

**Risk**: feature freeze blocks user-facing momentum for 2 weeks.

**Mitigation**: the 6 tactical patches in pre-Phase A all ship visible
fixes (consolidation envelopes no longer lost, archive purge shipped,
peak-hour digest errors gone). The user-visible cadence stays positive
during pre-Phase A.

**Inside Phase A** (2 weeks), the dashboard becoming correct (A.6) is
ITSELF the visible win. "Everything says never" → "everything says the
right time" is the largest user-trust delivery in the entire 6-week
plan. Don't underweight it.

**If the user prioritizes phase11v (the 3 MCP tools) over the freeze**:
acceptable, but require phase11v to land **after** A.3 ships, as the
trait's first consumer. Don't ship phase11v atop the current
`extras: HashMap` contract — that locks in a forced migration.

---

## Verifiable milestones

| Date | Milestone | Verification |
|------|-----------|--------------|
| 2026-05-11 | Pre-Phase A patches done | `git log --grep="^(fix|feat)\(" --since=2026-05-05` shows 6+ commits |
| 2026-05-18 | A.1 + A.5 + A.6 green | Dashboard `/v1/dashboard/retention/sweeps` reads `retention_sweeps`; `cortex-ops doctor config-audit` reports 0 ad-hoc envs |
| 2026-05-25 | A.2 + A.3 + A.4 green | `cortex doctor consistency` < 10s; phase11v unfrozen |
| **2026-06-01** | **Mid-checkpoint** | A.1 + A.2 green. If not: HALT, re-evaluate |
| 2026-06-08 | Phase B done | Consolidator daemon shipping nightly; pruning cascades; golden-set harnesses gating CI |
| 2026-06-15 | Phase A + B retrospective | All 9 ADRs accepted; metrics dashboard shows recall@5 ≥ 0.50 |
| 2026-06-22 | Phase C done | 17 repos in bootstrap; Codex/Cursor adapters in flight |

---

## Failure modes to watch

1. **Phase A "becomes a refactor with no visible feature"**. Mitigation:
   A.6 (dashboard becomes pure reader) is the visible win. Ship it
   parallel to A.1, not after.
2. **A.3 `Lane` trait gets designed in isolation**. Mitigation: design
   it driven by `phase11v` requirements (3 search tools); land
   phase11v as the trait's first consumer immediately after A.3 closes.
3. **Tactical patches keep landing during Phase A**. Mitigation: the
   feature-freeze. CI rejects PRs that touch `cortex-mcp-server/src/tools.rs`
   between 2026-05-12 and 2026-05-25 unless they cite an A.* item.
4. **117-task inventory finds dead code, no one cleans it up**.
   Mitigation: add a Phase C item C.0: "Delete dead code from
   inventory." Half-day effort, large clarity payoff.
5. **Schema evolution (ADR-016) gets deferred to "later"**. Mitigation:
   ADR-016 lands inside Phase A.5 — it's the rationale for `cortex-config`
   choosing serde-versioned structs over ad-hoc TOML.

---

## What to do this week

If the user agrees with this plan:

1. **Today**: pause `phase11v_mcp-fine-grained-backend-search`. Mark
   the active task as `paused` with a one-line reason ("waiting on
   ADR-011 + Phase A.3").
2. **Today**: create the 7 ADRs as `proposed` via
   `rulebook_decision_create`. Trade-offs explicit per Tier 0.
3. **This week**: ship pre-Phase A patches 1–6.
4. **2026-05-12**: kick off Phase A. A.1 + A.5 + A.6 in week 1.
5. **2026-06-01**: mid-checkpoint.
6. **2026-06-15**: Phase A + B retro.

If the user disagrees with the freeze: ship `phase11v` atop the
current contracts, accept the migration cost when ADR-011 lands, and
expect the recommendation cycle to repeat in 60 days.

---

## What this analysis ASKED to deliver vs what it delivers

The user prompt was:

> ola minmax quero sua analise sobre o cortex e o que podemos melhorar
> salve sua analise em /docs/analysis/rework/opus5.7

(Note: user wrote "minmax" — addressing me; I'm Claude Opus 4.7. Path
saved as requested at `docs/analysis/rework/opus5.7/`.)

Delivered:
- Independent validation of 11 prior findings (1 closed, 1 partial,
  9 still open) → [01-validation-delta.md](./01-validation-delta.md)
- 10 blind spots beyond the prior 4 docs →
  [02-blind-spots.md](./02-blind-spots.md)
- Concrete sequenced plan with 9 ADRs, 6-week horizon, mid-checkpoint
  → this document

NOT delivered (out of scope, deliberate):
- Code-level patches for any finding
- ADR drafts (only titles + trade-off summaries)
- Performance benchmarking
- User research on whether the rework horizon is acceptable
