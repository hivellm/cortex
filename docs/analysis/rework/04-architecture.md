# 04 — Architecture review (high-level)

> **Audience**: project owner deciding whether to keep patching or
> commit to a structural rework. Frustration is real ("consolidation
> doesn't work, cleanup has to be brute force, the data results in
> nothing relevant"). This file gives a defensible answer.
>
> **Scope guard**: high-level only. Per-file findings are landing in
> parallel from other agents.
>
> **Position taken**: the frustration is **80% structural, 20% patch**.
> Recommended path: **medium rework** (1 design ADR + 2-3 sprints of
> structural consolidation), NOT a rewrite, NOT just more patches.
> Justification follows.

---

## Architecture today

Cortex at runtime is **a single Rust process (`cortex-api`) hosting
both the HTTP/MCP server and N in-proc daemons (classifier / embedder /
graph / fulltext / retention / consolidator / pruner / topic-cards /
claude-archive workers)**. They communicate through Synap streams
(`cortex.events.{raw,bootstrap,enriched,query_audit,...}`) and four
external backends: **Vectorizer** (vectors), **Nexus** (graph),
**Meilisearch** (BM25), **Synap** (event bus + durable log). Bootstrap
is a separate binary that injects historical envelopes into the same
bus. Consumer / cron / scheduling state lives in **SQLite** (ADR-008).
Electron+React frontend reads REST + SSE from the same `cortex-api`.
**There is no external supervisor**: if a worker dies inside the
process, the rest carries on and nobody notices until the operator
checks the dashboard — and the dashboard, until yesterday (2026-05-05),
hardcoded `next_run: "never"` for 7 of 9 sweeps.

---

## Problematic couplings

1. **`cortex-api` is god.** Hosts HTTP, MCP, dashboard, lanes, fusion,
   audit, retention daemon, cache, ACL, redaction, consumer offsets,
   loaders. 30+ modules in the same crate. Testing it in isolation is
   impossible; a bug in any subsystem (e.g., dashboard hardcode)
   compromises trust in all of it.
2. **"Worker" is not a concept.** `cortex-workers` groups 8 families
   (classifier / embedder / graph / fulltext / retention /
   consolidator / pruner / topic_cards / claude_archive), each with its
   own env-var conventions, bookkeeping, error recovery, and dashboard
   shape. The 2026-05-05 learning quotes verbatim: *"each sweep was
   implemented as a self-contained CLI with its own dashboard story,
   then bolted on to the cron scheduler later. No shared 'I am running
   as a sweep' wrapper exists"*. Same problem in the consolidator: 3
   grains (Session/Topic/DecisionTrace), 3 producers, 3 templates, 3
   cost-telemetry paths. When one of them stalls, the others mask it.
3. **Lanes and fusion know each other's internals.** `MeiliKeywordLane`
   stamps `extras["source"]="keyword"`; `orchestrator::derive_decisions`
   filters by `extras["decision_id"]` which **only the legacy
   `MemoryKeywordLane` stamped**; result: decisions overlay always
   empty in production (F-007 in
   `docs/analysis/relevance/01-findings.md`). The contract between
   lanes and the orchestrator is implicit, scattered across magic
   strings, and had no regression guard until phase6b.
4. **Bootstrap state file is a single-repo singleton.**
   `.cortex-bootstrap.state.json` is overwritten on every invocation.
   There is no "what's the org's coverage state?" — only "what was the
   last repo I walked?". 4 walked repos visible today; evidence others
   were walked-and-forgotten.
5. **The `Kind` schema grew by addition, never by reorganization.**
   Today: Turn / ToolCall / AgentCall / Artifact / Memory / Decision /
   Learning / LawViolation / Analysis / Consolidation / TopicCard / +
   internal family slugs. Each new feature adds a `Kind` and family;
   nothing is ever deprecated. Routing in
   `cortex-fulltext/src/routing.rs` has an explicit case per kind —
   implicit N² when adding lanes.
6. **Event identity is duplicated in 3 places.** `event_id` (own ULID)
   + Nexus internal id + Vectorizer collection-relative id + Meili
   `_id`. ADR-004 tried to align via "identity in Nexus reserved id
   slot" but only covered Nexus. Result: cross-backend rewrites
   (forget, dedup, retention) always require manual joins that fail
   silently when one backend lags.
7. **Dashboard reproduces daemon logic.** Instead of consuming a
   source of truth, the dashboard recomposes state from Meili-loaded
   fixtures + SQLite + REST calls to backends. Every new feature has
   to **build the feature + build the dashboard reader**, and they
   drift (the hardcoded retention `"never"` is the canonical example).
8. **Bootstrap, claude-archive, topic-cards, consolidator, and
   retention share the abstraction "a source that produces envelopes",
   but each one reimplemented it.** No common `EnvelopeProducer`
   trait. Result: every new "indexer" (Codex, Cursor, Gemini — spec 17)
   will cost replicating the same structure with no reuse benefit.

---

## Diagnosis — patches vs redesign

**Position**: the user's frustration is **structural**, not point-bug.
Evidence:

- The latest learning (2026-05-05, retention daemon) lists **6
  independent bugs that surfaced as a single observation** ("everything
  says never"). The author (you) already concluded in writing: *"each
  sweep was implemented as a self-contained CLI ... no shared 'I am
  running as a sweep' wrapper exists. Six independent gaps all live in
  the same gap: 'what does it mean to BE a sweep' was never codified."*
  This is not "a bug" — it's abstraction debt materializing.
- Consolidation has **the same shape**: 3 producers + 3 templates + 3
  cost-telemetry paths + 3 source modules, with no common
  `Consolidator` trait with a testable contract. When one grain
  (decision-trace) doesn't fire, the others mask it. The file
  `consolidator/mod.rs` is literally a skeleton with implicit TODOs
  ("§2.1 ships the trait surface ... full bodies land in §2.4..§2.10").
- "Cleanup has to be brute force" = pruning has no guaranteed inverse
  (re-hydration). Vectorizer SDK 3.2 has no per-vector `move/delete`
  (learning 2026-05-03), so the tier-transition pipeline is
  structurally blocked externally. **Patches don't fix it; the
  tiering redesign has to accept this**.
- "Data doesn't result in anything relevant" maps directly to
  F-001..F-009 (`docs/analysis/relevance/`): 5 of 9 findings are
  **inter-module contract defects** (scope default, lane projection,
  RRF score-blindness, intent table, query rewriting). Isolated
  patches already closed half (phase6b/c/d/e/g), but the recurrence
  shows that every new lane will introduce the same defect class.
- Recent patches (phase11p/q/r/s/t/u/v) accumulate at an unsustainable
  rate: **117 archived tasks**, ~30 in the last 7 days, and yet the
  operator opens the dashboard and everything still says "never".
  Patch velocity is high; **bug discovery** velocity is higher. That's
  the canonical signal that the abstraction lags the usage pressure.

**What is NOT structural** (the genuine 20% patch):

- Vectorizer SDK 3.0.3 silent-failure on `/upsert` — upstream bug;
  workaround is local.
- Meili settings v1→v4 — incremental, not load-bearing.
- Nexus UNWIND drop — already mitigated with `assert_write_landed`.

**Verdict**: 80% abstraction redesign / 20% upstream patch. **Not a
rewrite** — Synap+Vectorizer+Nexus+Meili+SQLite are the right choices.
The **plumbing between them** is what's ad-hoc.

---

## Rework recommendation

**Size: MEDIUM.** 1 design ADR + 2-3 sprints of structural
consolidation. Not "5 surgical PRs" (insufficient for couplings 1, 2,
6, 8) and not "subsystem rewrite" (Synap/Nexus/Vectorizer/Meili don't
need to change; only the glue).

### Attack sequence (verifiable gates between phases)

**PHASE A — Codify abstractions (1 sprint, ~10 days).** No new
features. Just extract contracts.

- A.1. **Trait `Sweep`** in `cortex-workers/src/sweep/mod.rs`: each
  retention/pruning becomes `impl Sweep`, exposes `name() / schedule()
  / run() -> SweepReport`, writes a row in `retention_sweeps` per
  invocation. Migrate the 7 sweeps that today only update `cron_jobs`.
  **Gate**: dashboard `/v1/dashboard/retention/sweeps` reads only
  `retention_sweeps`; IT proves each sweep produces exactly one row
  per execution, success or fail.
- A.2. **Trait `EnvelopeProducer`** in `cortex-core/src/producer.rs`:
  bootstrap, claude-archive, topic-cards-emit, consolidator-emit,
  future Codex/Cursor adapters. Contract: `produce(ctx) ->
  Stream<Envelope>` + `checkpoint(ctx) -> ProducerCheckpoint` (durable
  in SQLite).
  **Gate**: bootstrap and claude-archive migrated; checkpoint table
  accumulates (does not overwrite); IT proves resume after kill.
- A.3. **Trait `Lane` + projection contract**. Already partial in
  `crates/cortex-api/src/lane_contract.rs` (phase6b). Harden it:
  `Lane::project(hit) -> ProjectedHit { decision_id?, turn_id?,
  law_id?, symbol?, ... }` with typed structs, **not `extras:
  HashMap<String,Value>`**.
  **Gate**: all `extras.get("...")` calls in
  `orchestrator.rs::derive_*` removed; regression test covers empty
  overlay across at least 3 lanes.
- A.4. **Identity layer**. `EventIdentity { event_id, nexus_id?,
  vec_id?, meili_id? }` in `cortex-storage`, with `IdentityIndex`
  SQLite-backed. All cross-backend reads (forget, dedup, doctor,
  retention) move to this struct.
  **Gate**: `cortex doctor consistency` (already planned in phase4d)
  rewritten on top of `IdentityIndex`; one full run in <10s for 100k
  events.

**PHASE B — Rewrite ad-hoc subsystems atop the new traits (1 sprint,
~10 days).**

- B.1. **Consolidator** turned into a single `Consolidator` trait + 3
  `ConsolidationGrain` impls. Centralized cost telemetry. Re-run
  fidelity IT (already exists in phase11j).
- B.2. **Pruning daemon** turned into `Sweep` impls. Explicitly accept
  that Vectorizer SDK 3.2 has no per-vector move (record as a
  "blocking external dep" ADR) and implement **collection-level**
  pruning via re-encode-and-replace, not per-vector.
- B.3. **Dashboard as a pure reader**. All "what's the sweep /
  consolidation / coverage state" logic leaves the dashboard handler
  and lives in `Sweep::report() / Consolidator::report() /
  Coverage::report()`. Dashboard only renders.

**PHASE C — Bootstrap multi-repo + relevance closure (1 sprint, ~10
days).** Only execute if A and B close with green gates.

- C.1. Bootstrap orchestrator on top of accumulating
  `EnvelopeProducer::checkpoint`. Walk all 17 repos.
- C.2. Retrieval-quality harness (phase4 hardening / phase6e partially
  landed) run against the full corpus. Per-intent recall@k / MRR
  becomes a release gate.
- C.3. Codex/Cursor/Gemini adapter (spec 17) — now free, just `impl
  EnvelopeProducer`.

### Why not small (5 PRs)

5 surgical PRs solve F-001..F-009 and the 6 retention gaps. But they
don't solve **couplings 1, 2, 6, 8** — the next "feature" (live
governance, deep analysis, new adapter) will reintroduce the same
shape-bugs. The user opens the dashboard in 30 days and sees another
`"never"` in another column. Frustration doesn't drop; it climbs.

### Why not large (rewrite)

Synap (durable event bus), Vectorizer (vectors + tier-transition),
Nexus (graph), Meili (BM25), SQLite (state) **have no architectural
bug**. The stack choice is defensible. ADR-001 (bypass SDK) and
ADR-002 (separate classifier-worker crate) already prove the team
knows how to use ADRs to isolate external drift without changing
stacks. Rewrite costs 6 months, generates a new bug class, and loses
117 archived tasks of accumulated learning. Not worth it.

---

## Risks

| Path | Main risk | Probability | Cost if wrong |
|---|---|---|---|
| **Patch only (5 PRs)** | "Everything says never" returns in 60 days in another column; user gives up on the project. | High | Reputational: 6 months of reactive patches with no perceived gain. |
| **Medium (recommended)** | Phase A becomes "refactor with no visible feature" and the user feels stalled. **Mitigation**: each Phase A item delivers an observable gate (correct dashboard, green doctor, unified identity). | Medium | 2-3 weeks lost if the chosen abstraction is wrong — recoverable; trait can be revisited via superseding ADR. |
| **Large (rewrite)** | 6 months of work with nothing to show; new bugs; loses learnings captured in 18 ADRs and 117 tasks. | High cost, low payoff | Catastrophic for a solo / small team project. Do not recommend. |

**Specific medium-path risk worth flagging**: the temptation to touch
Phase B before Phase A closes. If the Consolidator is rewritten before
`Sweep`/`EnvelopeProducer`/`Lane` traits stabilize, the same mistake
repeats. **Phase A's gates are mandatory before any Phase B work.**

---

## Suggested ADRs

Title list to create via `rulebook_decision_create`. Each must carry
**explicit trade-off** (per AGENTS.override rule).

1. **ADR-009 — `Sweep` trait as the single contract for retention /
   pruning / digest jobs**
   - Trade-off: forces refactor of 7 existing sweeps; gains uniform
     observability and pure-read dashboard.
2. **ADR-010 — `EnvelopeProducer` trait as the single contract for
   bootstrap / claude-archive / future adapters**
   - Trade-off: bootstrap must migrate to accumulating checkpoints;
     gains resume-after-kill and free Codex/Cursor/Gemini adapters.
3. **ADR-011 — Lane projection contract uses typed `ProjectedHit`,
   not `extras: HashMap`**
   - Trade-off: API breaking between lanes and orchestrator; gains
     compiler-verifiable overlay correctness.
4. **ADR-012 — `EventIdentity` as cross-backend join key, materialized
   in SQLite `IdentityIndex`**
   - Trade-off: new table + extra write per ingest; gains forget /
     dedup / doctor / retention without ad-hoc joins.
5. **ADR-013 — Vectorizer pruning happens at collection level
   (re-encode-and-replace), not per-vector, until upstream SDK ships
   per-vector move/delete**
   - Trade-off: tier transition becomes coarse (whole collection moves
     at once); unblocks the daemon currently parked on SDK 3.2.
6. **ADR-014 — Dashboard handlers are pure readers; all "what's the
   state" logic lives in domain reports (`Sweep::report`,
   `Consolidator::report`, `Coverage::report`)**
   - Trade-off: dashboard becomes "dumb" (cannot infer state); makes
     hardcoded `"never"` impossible by construction.
7. **ADR-015 — `cortex-api` crate split: `cortex-api-http` (HTTP/MCP/
   dashboard) + `cortex-api-runtime` (lanes, fusion, audit, cache) +
   `cortex-api-daemons` (in-proc workers)**
   - Trade-off: 1-2 days of mechanical refactor; gains isolated
     subsystem testing so a bug in one doesn't poison trust in the
     others.
   - **Mark as reversible** — can collapse back to one crate if
     unnecessary.

---

## Review date

Revisit this document and the ADRs **2026-06-15** (6 weeks from today),
at the theoretical end of Phase B. If Phase A hasn't closed all 4
gates by then, the diagnosis was wrong; reopen the "medium vs large"
discussion with new evidence.
