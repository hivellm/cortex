# Rework analysis — Consolidation, Cleanup, Relevance, Architecture

> **Triggered**: 2026-05-05 by user frustration. Translated verbatim:
> "consolidation doesn't work, memory cleanup has to be brute force,
> the data so far doesn't result in anything actually relevant."
>
> **Scope**: 4 parallel research agents (researcher × 3 + architect × 1)
> audited the 3 painful subsystems plus high-level architecture.
>
> **Conclusion**: 80% structural debt / 20% upstream patches. Recommended
> path is **medium rework** (1 design ADR + 2-3 sprints), NOT a rewrite.

---

## Documents

| # | File | Owner | Severity ceiling | Top finding |
|---|------|-------|------------------|-------------|
| 01 | [01-consolidation.md](./01-consolidation.md) | researcher | **P0** | No consolidator daemon in production; envelopes vanish into unconfigured ingest URL |
| 02 | [02-memory-cleanup.md](./02-memory-cleanup.md) | researcher | **P0** | Parquet archive only purgable via `/v1/admin/forget` per-event; no cron path. CAS vacuum fails silently behind 50% safeguard. |
| 03 | [03-relevance.md](./03-relevance.md) | researcher | **CRITICAL** | Queries without `scope.repo` fall through to `cortex-unknown-*` (zero hits). Meili indexes empty for 2 of 3 repos. Graph topologically flat (only 2 of ~12 edge types). |
| 04 | [04-architecture.md](./04-architecture.md) | architect | — | Frustration is structural. Patches alone won't fix; rewrite is overkill. Medium rework with abstract layer (Sweep / EnvelopeProducer / Lane / EventIdentity traits) recommended. |

---

## Cross-document synthesis

### Why nothing works (root causes shared across docs)

1. **No shared "I am running as a sweep" abstraction** — each
   retention/digest/consolidator was bolted on standalone, with its own
   cron, dashboard story, error path. Six retention gaps + the
   consolidator gap are **the same shape bug**: missing trait
   `Sweep` / `EnvelopeProducer`. (Doc 04 §A.1, §A.2)
2. **No daemon for consolidator** — only CLI exists. Triggers
   (`SessionEnd`, `NightlyTopic`, `DecisionLanded`) defined but unused
   in production. (Doc 01 Achado 1)
3. **Output silently dropped** — consolidator `publish_consolidation()`
   POSTs to `http://127.0.0.1:17010` default. Unset in prod = envelopes
   discarded. (Doc 01 Achado 2)
4. **Archive never purged by cron** — only `/v1/admin/forget` per-event.
   Operator falls back to `rm -rf`. (Doc 02 Achado 1)
5. **Scope routing drops queries** — MCP/HTTP callers don't pass
   `scope.repo`; fallback to `cortex-unknown-*` returns nothing.
   (Doc 03 Achado 1)
6. **Lane contract via stringly-typed `extras` hashmap** — overlays
   filter by `extras["decision_id"]` but live lanes didn't stamp it
   until phase6b. The same shape will recur on every new lane until
   `ProjectedHit` becomes typed. (Doc 04 §A.3 + Doc 03 Achado 6)
7. **Graph mapper incomplete** — only `IN_REPO` + `REMEMBERS` edges
   exist; spec defines ~12. Graph lane returns nothing useful.
   (Doc 03 Achado 3)
8. **Tests verify structure, not semantics** — fidelity IT validates
   takeaway length, not truth. (Doc 01 Achado 4)

### What's already closed (don't re-do)

- ✅ Vector lane payload deserialization (phase11d)
- ✅ Keyword projection chain (phase6g)
- ✅ Lane extras contract (phase6b)
- ✅ Query rewriting (phase6f)
- ✅ Intent selector + `Intent::Explain` (phase6d)
- ✅ Live-file partial zstd in `admin_forget` (commit `766a74b`)
- ✅ `already_digested` cascade for tool-call-digest (commit `694958a`)

---

## Recommended sequencing

### Phase A — Codify abstractions (1 sprint, ~10 days, NO new features)
Foundation before any feature work. Gate-blocked.

- A.1 Trait `Sweep` — uniform contract for retention/digest/pruning
- A.2 Trait `EnvelopeProducer` — uniform contract for bootstrap /
  claude-archive / future adapters
- A.3 Trait `Lane` + typed `ProjectedHit` (replace `extras: HashMap`)
- A.4 `EventIdentity { event_id, nexus_id?, vec_id?, meili_id? }` +
  SQLite `IdentityIndex`

**Gate to Phase B**: dashboard reads a single source of truth; doctor
runs cross-backend consistency in <10s for 100k events; overlays are
not stringly-typed; bootstrap survives kill.

### Phase B — Rewrite ad-hoc subsystems atop the new traits (1 sprint)
- B.1 Consolidator → 1 trait + 3 grain impls (Doc 01 Phases 1-3)
- B.2 Pruning → `Sweep` impls; collection-level pruning (Vectorizer SDK
  3.2 limitation accepted via ADR-013)
- B.3 Dashboard becomes pure reader (Doc 02 Phase 6)

### Phase C — Coverage + relevance closure (1 sprint, gate-blocked)
- C.1 Bootstrap multi-repo via accumulating `EnvelopeProducer::checkpoint`
- C.2 Golden-set retrieval harness (Doc 03 Phase 5) gates releases
- C.3 New adapters (Codex/Cursor/Gemini) — free as `impl EnvelopeProducer`

---

## Tactical patches that can land in parallel (don't wait for Phase A)

These are local fixes that don't depend on the trait work:

| Patch | Doc | Severity | Effort |
|-------|-----|----------|--------|
| Wire `x-cortex-repo` resolution into `Service::query()`; reject 422 when unresolved | 03 §Phase 1 | CRITICAL | 1-2d |
| Audit Meili index population for Rulebook + Vectorizer repos | 03 §Phase 2 | HIGH | 1-2d |
| Add ERROR logs in `publish_consolidation()` failure paths | 01 §Phase 1 | P0 | < 1d |
| Add `cortex-ops retention-archive-purge --before <date>` | 02 §Phase 1 | P0 | 2-3d |
| Default `tool-call-digest` cron to `--purge-originals` | 02 §Phase 2 | P1 | < 1d |
| Extract `is_live_partial_frame()` to shared module + apply to digest purgers | 02 §Phase 3 | P1 | 1d |
| CAS vacuum 3-tier safeguard (no silent Ok(0)) | 02 §Phase 4 | P0 | 1d |

---

## ADRs proposed (Doc 04)

1. ADR-009 — `Sweep` trait as the single contract for retention/digest
2. ADR-010 — `EnvelopeProducer` trait for bootstrap/archive/adapters
3. ADR-011 — Typed `ProjectedHit` replaces `extras: HashMap` lane contract
4. ADR-012 — `EventIdentity` as cross-backend join key + SQLite `IdentityIndex`
5. ADR-013 — Vectorizer pruning is collection-level until SDK 3.2 ships per-vector move
6. ADR-014 — Dashboard handlers are pure readers; state logic lives in domain reports
7. ADR-015 — `cortex-api` crate split (http / runtime / daemons) — reversible

Create via `rulebook_decision_create` with explicit trade-off (per
`AGENTS.override.md` Tier 0).

---

## Review schedule

Revisit **2026-06-15** (6 weeks from today). If Phase A's 4 gates
aren't all green by then, the diagnosis was wrong; reopen the
"medium vs large" discussion with new evidence.

---

## Agent attribution

| Doc | Agent type | Tokens | Duration |
|-----|-----------|--------|----------|
| 01-consolidation | researcher | 87,380 | 147s |
| 02-memory-cleanup | researcher | 94,532 | 164s |
| 03-relevance | researcher | 98,647 | 134s |
| 04-architecture | architect | 88,887 | 257s |

All four ran in parallel from a single dispatch turn.
