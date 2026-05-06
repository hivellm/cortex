# 01 — Validation delta vs prior 4-doc analysis

> **Method**: each finding from the prior analysis (`01-consolidation`
> through `03-relevance`) was checked against the current code at the
> cited file/line. Verdict is one of: **STILL_OPEN** (evidence still
> matches), **PARTIALLY_FIXED** (some progress, gap remains),
> **CLOSED** (fix landed, evidence no longer matches), **UNKNOWN**
> (requires runtime verification beyond static read).

---

## Summary table

| Doc | Finding | Severity (prior) | Verdict (now) | Severity (now) |
|-----|---------|------------------|---------------|----------------|
| 01 | F1 — No consolidator daemon | P0 | **STILL_OPEN** | P0 |
| 01 | F2 — `publish_consolidation` drops envelopes when ingest_url=None | P0 | **STILL_OPEN** | P0 |
| 01 | F3 — Consolidations isolated index, no boost | P1 | **STILL_OPEN** | P1 |
| 01 | F4 — Fidelity IT lacks LLM-as-judge default | P1 | **STILL_OPEN** | P1 |
| 01 | F6 — Dashboard has no POST trigger | P2 | **STILL_OPEN** | P2 |
| 02 | F1 — Archive only purged via `/v1/admin/forget` per-event | P0 | **STILL_OPEN** | P0 |
| 02 | F2 — Pruner expired-tier doesn't cascade to Nexus + archive | P0 | **STILL_OPEN** | P0 |
| 02 | F3 — CAS vacuum returns `Ok(0)` silently above 50% | P0 | **PARTIALLY_FIXED** | P1 |
| 02 | F4 — `tool-call-digest` cron lacks `--purge-originals` default | P1 | **STILL_OPEN** | P1 |
| 02 | F6 — `is_live_partial_frame()` only in `admin_forget` | P1 | **STILL_OPEN** | P1 |
| 03 | F1 — `Service::query()` doesn't enforce `scope.repo` | CRITICAL | **CLOSED** | — |
| 03 | F2 — Meili indices empty for Rulebook + Vectorizer repos | HIGH | **UNKNOWN** | HIGH (needs runtime check) |
| 03 | F3 — Graph mapper only `IN_REPO` + `REMEMBERS` | HIGH | **STILL_OPEN** | HIGH |
| 03 | F11 — Meili settings v1 has no pt-BR analyzer | MEDIUM | **STILL_OPEN** | MEDIUM |

**Net**: 1 closed, 1 partial, 11 still open, 1 unknown. **Prior
analysis is ~14% stale and ~86% live.**

---

## Per-finding evidence

### 01 — Consolidation

#### F1 — No consolidator daemon (STILL_OPEN, P0)
- **Cited**: `crates/cortex-workers/src/bin/` (no consolidator daemon
  binary)
- **Verified**: `bin/` contains 10 binaries:
  - `classifier-worker.rs`, `cortex-claude-archive.rs`,
    `cortex-consolidator.rs`, `cortex-ingestion.rs`,
    `cortex-retention-sweep.rs`, `cortex-topic-cards.rs`,
    `embedder.rs`, `fulltext-indexer.rs`, `graph-backfill.rs`,
    `graph-writer.rs`
- `cortex-consolidator.rs` is CLI-only (`run_nightly()` is invoked
  imperatively, not subscribed to triggers).
- **Verdict**: still open. The structural fix (Phase A.1 — `Sweep`
  trait) and Phase 2 of doc 01 (build daemon) are both required.

#### F2 — `publish_consolidation` drops envelopes (STILL_OPEN, P0)
- **Cited**: `crates/cortex-workers/src/bin/cortex-consolidator.rs:78,
  248-261, 500-600`
- **Verified**: `resolve_ingest_url()` returns `None` for empty URL;
  `publish_consolidation()` early-returns `Ok(())` when URL is `None`.
  Default `127.0.0.1:17010` is unset in prod = silent drop. No fallback
  to file or stdout.
- **Quick patch ($<0.5d)**: log ERROR + write to
  `.cortex/consolidations.jsonl` when URL unset. Doesn't fix
  architecture, prevents data loss.

#### F3 — Consolidations isolated index, no boost (STILL_OPEN, P1)
- **Cited**: `crates/cortex-workers/src/fulltext/builders.rs:249-270`,
  `crates/cortex-api/src/meili_loader.rs:48`, `:227-228`
- **Verified**: `builders.rs` routes `Kind::Consolidation` to a
  separate index. No reverse `consolidation → source_session_ids`
  link in the schema. Vectorizer pushes consolidations to dedicated
  collections (`cortex-{slug}-consolidations`).
- **Note**: this finding becomes load-bearing only AFTER F1 + F2 are
  fixed. Until consolidations exist in production, integrating them
  with retrieval is academic.

#### F4 — Fidelity IT (STILL_OPEN, P1)
- **Cited**: `crates/cortex-workers/tests/consolidator_consolidation_fidelity_it.rs:168-220`
- **Verified**: still 7 structural invariants. LLM-as-judge gated on
  `ANTHROPIC_API_KEY` (line 28-33). `CannedSummariser` returns
  hardcoded JSON.
- **Add**: this is a fidelity-debt smell that recurs across the
  codebase (see blind-spot §3). Not unique to consolidator.

#### F6 — Dashboard POST trigger (STILL_OPEN, P2)
- **Cited**: `crates/cortex-api/src/dashboard/consolidations.rs`,
  `gui/src/views/Consolidations.tsx`
- **Verified**: untracked file `dashboard/consolidations.rs` and
  untracked `views/Consolidations.tsx` exist in the working tree.
  Both add GET routes only. Active feature work, not yet committed.

---

### 02 — Memory cleanup

#### F1 — Archive only purged per-event (STILL_OPEN, P0)
- **Cited**: `crates/cortex-api/src/admin_forget.rs:164-181`,
  `crates/cortex-workers/src/pruner/purge.rs:122-168`
- **Verified**: `admin_forget.rs` has `LiveArchivePurger` only reachable
  via the HTTP endpoint. `pruner/engine.rs` does not call
  `archive.drop_event()` on the expired-tier transition.
- **Confirmed**: still no `cortex-ops retention-archive-purge` subcommand.

#### F2 — Pruner expired cascade (STILL_OPEN, P0)
- **Cited**: `crates/cortex-workers/src/pruner/engine.rs`,
  `mod.rs:160-165`
- **Verified**: `plan_demotion(Expired)` returns `vector_ids: []`. No
  call site routes to Nexus delete or archive drop on expired-tier.
- **This finding is the same shape as F1** — both are missing
  cron-cascade paths into the central forget/purge primitive.

#### F3 — CAS vacuum silent above 50% (PARTIALLY_FIXED, P1)
- **Cited**: `crates/cortex-workers/src/retention/cas_vacuum.rs:80-130`
- **Verified**: line 121-135 NOW returns
  `Err(VacuumError::SafeguardTripped)` instead of `Ok(0)`. **The error
  surface improved.**
- **Still open**: the operator-runtime fix (3-tier safeguard with
  WARN/ERROR/HARD-STOP, env override, separate cron) per doc 02 §Phase 4
  has NOT landed.
- **Severity downgrade**: P0 → P1 because the operator now sees the
  error rather than silent success. Phase 4's full fix is still needed
  but the bleeding stopped.

#### F4 — tool-call-digest opt-in (STILL_OPEN, P1)
- **Cited**: `crates/cortex-workers/src/retention/tool_call_digest.rs`,
  spec-19
- **Verified**: `purge_originals: false` default still in code. Cron
  scheduler config still passes the flag explicitly.
- **Quick patch (<1d)**: flip the cron default per doc 02 §Phase 2.
  No architectural dependency.

#### F6 — `is_live_partial_frame` only in admin_forget (STILL_OPEN, P1)
- **Cited**: commit `766a74b`, `crates/cortex-api/src/admin_forget.rs:218-322`
- **Verified**: function still only present in `admin_forget.rs`
  (lines 225-248, 287-289). Grep across the workspace shows no other
  call sites. `turn_digest.rs` and `tool_call_digest.rs` have no
  equivalent.
- **Quick patch (1d)**: extract to
  `crates/cortex-storage/src/archive_purge.rs`, apply to all 3 purgers.

---

### 03 — Relevance

#### F1 — Service::query() scope routing (CLOSED) ✅
- **Cited**: `crates/cortex-api/src/strategies.rs:25-34`,
  `crates/cortex-api/src/service.rs`
- **Verified**: `service.rs:87-100` now has `resolve_scope()` properly
  wired. `ENV_ALLOW_UNKNOWN_SCOPE` deprecation hatch present
  (lines 39-44). HTTP 422 mapping at lines 94-95.
- **This was the highest-severity finding ("CRITICAL") in the prior
  analysis.** It's closed. Any remaining "no relevant snippets"
  symptoms now have a different root cause (likely F2 — indexing
  coverage).

#### F2 — Meili indices empty per repo (UNKNOWN, HIGH)
- **Cited**: `crates/cortex-workers/src/fulltext/routing.rs`,
  `crates/cortex-api/src/meili_loader.rs`
- **Cannot verify statically.** This requires a runtime check
  (`curl /v1/health/meili` or `cortex meili-search --index
  cortex-rulebook-code --query schema`).
- **Recommend**: doc 03 §Phase 2's verification commands should be
  run as a 30-min smoke before any further relevance work.

#### F3 — Graph mapper edge types (STILL_OPEN, HIGH)
- **Cited**: `crates/cortex-api/src/nexus_graph_lane.rs:61-100`
- **Verified**: templates show `TOUCHED` + `SUPERSEDES` only. The
  chunker's `symbol` field is referenced (line 187) but the mapper
  still emits only 2 of the ~12 spec'd edge types.
- **This is the highest-leverage open relevance finding.** The graph
  lane is currently a no-op for most queries; populating it 6× by
  emitting `CALLS` / `IMPORTS` / `DEFINES` / `RETURNS` is a step-change
  in retrieval quality.

#### F11 — pt-BR analyzer (STILL_OPEN, MEDIUM)
- **Cited**: `crates/cortex-workers/src/fulltext/settings/settings.v1.json`
- **Verified**: still `settings.v1.json`, English-only stopwords/synonyms,
  no pt-BR locale.
- **Note**: this is a 2-line config change blocked only on a settings
  version bump (v1 → v7 per doc 03). Schema-versioning is itself a
  blind spot — see [02-blind-spots.md §4](./02-blind-spots.md#4-schema-versioning-with-no-migration-path).

---

## Active task (the strategic miss)

`.rulebook/STATE.md` lists the active task as
`phase11v_mcp-fine-grained-backend-search` (0/23 items, not started).
The proposal adds 3 new MCP tools backed by 3 new endpoints in
`cortex-api` — directly into the god crate the prior architecture
analysis (Doc 04) flags for split.

**This is feature work being prioritized over the abstraction extraction
the analysis says must come first.** Detail in
[03-recommendation.md](./03-recommendation.md).

---

## What's drifting in the working tree right now

`git status` (snapshot at conversation start):

```
M crates/cortex-api/src/dashboard.rs
M crates/cortex-api/src/meili_loader.rs
M gui/src/App.tsx
M gui/src/lib/api.ts
M gui/src/shell/Sidebar.tsx
?? crates/cortex-api/src/dashboard/consolidations.rs
?? gui/src/views/Consolidations.tsx
```

Pattern: a "consolidations dashboard view" feature is being built
across backend + frontend simultaneously, **without an active task
covering it** (`phase11v` is MCP search, not consolidations UI). This
is the operational signature of the 117-archived-tasks pattern —
features land outside task tracking, abstraction debt accumulates
silently.

**Not a critique of the work itself** — but a data point that the
discipline gap is live, not historical.
