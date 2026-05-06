# 02 — Blind spots in the prior 4-doc analysis

> Concerns the prior `01-consolidation` / `02-memory-cleanup` /
> `03-relevance` / `04-architecture` set didn't cover or under-weighted.
> Each entry: severity, evidence, and how it changes the rework call.

---

## 1. Config sprawl — 344 `CORTEX_*` references across 50+ files

**Severity**: P1 (operator pain) → P2 (architectural)
**Confidence**: High

### Evidence

`grep -r "CORTEX_[A-Z_]+" crates/` returns **344 matches across 50
files**. Top offenders:
- `crates/cortex-api/src/main.rs` — 82 references
- `crates/cortex-api/src/http.rs` — 32 references
- `crates/cortex-api/src/config_audit.rs` — 26 references

Spot-check of the surface:
- `CORTEX_HOME`, `CORTEX_INGESTION_URL`, `CORTEX_RRF_ALPHA`,
  `CORTEX_QUERY_REWRITER`, `CORTEX_ALLOW_UNKNOWN_SCOPE`,
  `CORTEX_DIGEST_DRY_RUN`, `CORTEX_CAS_VACUUM_FORCE`,
  `CORTEX_CONSOLIDATOR_BUDGET_CENTS`, `CORTEX_GRAPH_CYPHER_ENABLED`,
  `CORTEX_NEXUS_EXTERNAL_ID_IT`, …

Many overlap (3 different vars affect retention behavior, 2 affect
scope resolution).

### Why prior analysis missed it

The 4 docs are organized by SUBSYSTEM (consolidation / cleanup /
relevance / architecture). Config sprawl is a CROSS-CUTTING concern
that doesn't surface inside any single subsystem audit.

### How it changes the call

Add to Phase A: **A.5 — `cortex-config` crate with a single typed
`Config` struct** that subsumes `cortex.toml` + env vars. Bind every
new feature to `Config::*` rather than `std::env::var(...)`.

This is independent of A.1–A.4 and can run in parallel — but only
**inside Phase A**, not after. Otherwise every Phase B subsystem
rewrite re-introduces ad-hoc env-var reads.

### Quick smoke today

`cortex-ops doctor config-audit` already exists
(`crates/cortex-api/src/config_audit.rs:26` references).
Run it, see how many of the 344 references are documented vs ad-hoc.
That number is the real backlog for A.5.

---

## 2. The `phase11v` active task entrenches the lane monolith

**Severity**: P0 (process)
**Confidence**: High

### Evidence

`phase11v_mcp-fine-grained-backend-search/proposal.md` (excerpted):

> **Affected code:** `crates/cortex-api/src/search_proxy.rs` (new),
> `crates/cortex-api/src/http.rs` (3 new routes),
> `crates/cortex-mcp-server/src/tools.rs` (3 new tool impls + registry
> bump 7 -> 10), `crates/cortex-mcp-server/src/lib.rs` (re-exports).

The 3 new MCP tools (`cortex_vector_search`, `cortex_keyword_search`,
`cortex_graph_query`) are direct backend proxies that bypass fusion.
Their natural home is `impl Lane` against the **typed
`ProjectedHit`** from ADR-011.

**But ADR-011 hasn't been written yet.** Doc 04 §"Suggested ADRs" lists
it as ADR-011 (proposed, not landed). So phase11v ships against the
current `extras: HashMap<String, Value>` contract, then has to be
migrated when ADR-011 lands.

### Why prior analysis missed it

Doc 04 was written about the SHAPE of the problem; it didn't audit
the ACTIVE backlog. The active task list is in `.rulebook/STATE.md`
and `.rulebook/tasks/`, separate documents.

### How it changes the call

**Defer phase11v** until ADR-011 + Phase A.3 land. Or scope phase11v
to be **the first consumer of the new Lane trait** — the trait gets
designed driven by the 3-search use case, then phase11v ships atop it.

The second option is better — abstractions designed in isolation
("imagine 3 lanes that…") are weaker than abstractions designed against
a real consumer ("3 lanes for vector / keyword / graph search must
support…"). But it requires sequencing discipline: A.3 trait first,
phase11v as the trait's `impl` second.

---

## 3. Test fidelity debt is system-wide, not just in the consolidator

**Severity**: P1 (quality)
**Confidence**: Medium-High

### Evidence

Doc 01 §F4 calls out that `consolidator_consolidation_fidelity_it.rs`
validates length, not truth. That same pattern recurs across the
codebase:

- `crates/cortex-workers/tests/embedder_it_*.rs` — verifies vectors
  exist, not that they're semantically meaningful
- `crates/cortex-api/tests/relevance_eval_it.rs` — likely (per
  filename) a structural relevance check
- `crates/cortex-api/tests/lane_contract.rs` (referenced by phase6b)
  — verifies extras keys are stamped, not that the value is correct
- The fidelity tests gated on `ANTHROPIC_API_KEY` are skipped in
  default CI (Doc 01 line 78)

The whole codebase has **structural test coverage but not semantic
test coverage**. The retrieval-quality harness (Doc 03 Phase 5,
`docs/evals/queries.csv` + `cortex-eval` crate) is the proposed
remedy — but only for retrieval. Consolidation, classification, and
graph mapping don't have golden-set equivalents.

### Why prior analysis missed it

Doc 01 caught the consolidator fidelity gap. Doc 03 caught the
relevance fidelity gap. **No doc connected these into "the codebase
has zero golden-set semantic tests".**

### How it changes the call

Add to Phase B: **B.4 — Golden-set harness applies to ALL three
LLM-driven subsystems**:
1. Retrieval (Doc 03 Phase 5 — already proposed)
2. Consolidation (Doc 01 Phase 3 — already proposed but limited to
   takeaway scoring)
3. Classification (NOT in prior analysis — `crates/cortex-workers/src/classifier/`
   has structural tests only)

Each gets `docs/evals/{retrieval,consolidation,classification}.jsonl`
with labeled truth + `cortex-eval` subcommands gated in CI.

This is a multi-week investment but it's the only way to detect
quality regressions automatically. Today, regressions are detected
by user frustration (the trigger for this entire analysis).

---

## 4. Schema versioning with no migration path

**Severity**: P1
**Confidence**: High

### Evidence

- `crates/cortex-workers/src/fulltext/settings/settings.v1.json` —
  schema is versioned, but how does v1 → v2 → v7 happen at runtime?
- `Kind` enum (Doc 04 §coupling 5: "grew by addition, never by
  reorganization")
- `ConsolidationPayload` struct — adding `semantic_fidelity_score`
  (Doc 01 Phase 3) is a breaking change for any consolidation already
  on disk; no migration tooling
- Parquet archive partitions — no schema-evolution policy

### Why prior analysis missed it

Each doc focused on its subsystem. Schema evolution is the
cross-cutting concern that bites when you try to roll out a fix:
"add `archive_purge` cron" is easy; "add `archive_purge` cron that
handles the 3 schema versions of envelopes already on disk" is hard.

### How it changes the call

Add to ADR list:
- **ADR-016 — Schema-evolution policy: every persisted struct has a
  version tag, every reader supports the current + 1 prior version,
  every writer writes only the current version.**

Trade-off: 1 extra serde annotation per struct, gain
forward-compatibility during the rework itself.

---

## 5. GUI ⇄ backend contract drift

**Severity**: P2
**Confidence**: High

### Evidence

`git status` snapshot:
- `M gui/src/lib/api.ts`
- `M gui/src/App.tsx`
- `M gui/src/shell/Sidebar.tsx`
- `M crates/cortex-api/src/dashboard.rs`
- `M crates/cortex-api/src/meili_loader.rs`
- `?? crates/cortex-api/src/dashboard/consolidations.rs`
- `?? gui/src/views/Consolidations.tsx`

**5 modified files spanning the GUI/backend boundary, plus 2 new
untracked files.** Without an active task tracking this work, contract
drift is a near certainty:
- TypeScript types in `api.ts` may not match Rust types in
  `dashboard/consolidations.rs`
- `Sidebar.tsx` likely references the new Consolidations view; if
  the view ships in GUI but the backend route lands in a separate PR,
  GUI breaks at runtime

There's no `openapi.json` generated from the Rust handlers, no
contract test that diffs `api.ts` against the Rust route signatures.

### Why prior analysis missed it

Doc 04 §coupling 7: "Dashboard reproduces daemon logic" hints at
the symptom. The drift mechanism (no shared schema, no contract
test) is one layer deeper.

### How it changes the call

Add to Phase B: **B.5 — Generate `gui/src/lib/api.ts` from the
Rust route signatures** (or at least add a contract test that
diffs them). This is a small lift (`schemars` + `ts-rs` or similar)
that prevents an entire class of cross-stack bugs.

Alternatively, accept the drift and add a CI check that GUI builds
against the latest backend OpenAPI before merge.

---

## 6. The "117 archived tasks" debt is not yet inventoried

**Severity**: P1 (planning)
**Confidence**: Medium

### Evidence

Doc 04 cites "117 archived tasks" and "30 in the last 7 days" as
evidence that patch velocity exceeds bug discovery. But the docs
don't:

1. Categorize the 117 — how many are bugfixes, refactors, features?
2. Identify dead code from abandoned phases — do all 117 have
   shipping artifacts in the current codebase, or are some
   half-merged?
3. Surface which features were SHIPPED vs PARTIALLY_SHIPPED.

### Why prior analysis missed it

Time-bounded — the 4 docs were a 4-agent parallel dispatch (147s to
257s each per the README attribution table). Inventorying 117 tasks
would have been a separate research session.

### How it changes the call

Add a one-shot inventory task: **"List all archived tasks since
phase8, categorize each as `feature_shipped` /
`feature_partial` / `bugfix` / `refactor` / `infra` / `dead_code`,
mark dead-code candidates for removal."**

This is a researcher-tier task (haiku, ~1h). Output: a single CSV
the team can use to plan abandonment / cleanup. Not on the critical
path of Phase A but useful as parallel hygiene.

---

## 7. Test inventory: what subsystem has zero integration tests?

**Severity**: P1
**Confidence**: Medium (would need runtime grep to confirm)

### Evidence (partial — full audit would require a separate pass)

`crates/cortex-api/tests/` has integration tests for:
- `dashboard_auth_it.rs`, `decision_lookup_it.rs`,
  `governance_global_index_it.rs`, `http.rs`, `law_check_it.rs`,
  `meili_filter_grammar_it.rs`, `relevance_eval_it.rs`,
  `vectorizer_lane.rs`

`crates/cortex-mcp-server/tests/` has `forget_it.rs` only.
`crates/cortex-workers/tests/` has multiple but not one per binary.

What's missing (likely — needs confirmation):
- E2E test for the consolidator binary (only fidelity IT exists)
- E2E test for the bootstrap walker resume-after-kill path (Doc 04
  §coupling 4 implies this is broken)
- E2E test for retention sweeps that exercises the cron path (the
  "everything says never" bug shipped because of this gap)

### How it changes the call

This is essentially Doc 04's Phase A.1 gate restated: "IT proves each
sweep produces exactly one row per execution, success or fail." But
the same gate needs to apply to consolidator (Phase B.1) and
bootstrap (Phase C.1). Make the gate pattern explicit in the rework
docs.

---

## 8. The pre-thinking pipeline is over-engineered for what it ships

**Severity**: P2 (debatable)
**Confidence**: Low-Medium (this is opinion; reasonable people will
disagree)

### Evidence

Per `docs/analysis/prethinking/findings.md` (the pre-thinking analysis
already in the repo):

- F-001: 5-stage pipeline (scope → intent → query → format → clip)
- F-002: 55-keyword rule table for 6 intents
- F-006: 6-step trim ladder
- F-009: deterministic byte-identical Markdown output
- F-013: 3 named graph edge classes formatted into 3 sub-blocks

The implementation is sophisticated and correct, but **the actual
output the LLM gets is dominated by the failure modes the prior 4
docs identify**:
- Scope unresolved → empty bundle (closed in 03-F1)
- Meili indices empty → no snippets (still open, 03-F2)
- Graph topologically flat → no graph hits (still open, 03-F3)
- Consolidations isolated → no consolidations in bundle (still
  open, 01-F3)

So the pre-thinking pipeline is doing impeccable work formatting
empty data. The user-visible quality of the bundle is bottlenecked
on the upstream subsystems, not on the formatter or the trim ladder.

### Why prior analysis missed it

Doc 04 framed the diagnosis at the abstraction layer; it didn't
look down into pre-thinking specifically. The prethinking findings
doc was written in a separate analysis pass and isn't cross-linked
from the rework set.

### How it changes the call

**No change to the rework plan**, but a calibration note: when
Phase A + B close and consolidations / graph / Meili are populated,
the pre-thinking output will improve dramatically *without any
pre-thinking changes*. Don't tune the formatter before the upstream
data exists. The temptation to "improve the bundle" while the data
is still empty is real and counter-productive.

---

## 9. Security/privacy track is under-instrumented

**Severity**: P1
**Confidence**: Medium

### Evidence

Recent commit `8debc3b chore(security): redact secrets in graph
dumps + ignore *.cypher` proves there IS a security track. But:
- No security ADR exists in `.rulebook/decisions/`
- No `cortex-redact` crate or shared redaction module surfaced
- The `8debc3b` fix is surgical (graph dumps); ingestion-time
  redaction policy is undocumented

What about:
- Tool calls that contain secrets (env-var dumps, `.env` reads)?
  These get classified, embedded, indexed, archived.
- Memory captures that contain user PII (email, phone, addresses)?
- Decision documents that contain stakeholder names?

### Why prior analysis missed it

The prior 4 docs are about why retrieval doesn't work. Security is
a separate axis.

### How it changes the call

Add to ADR list:
- **ADR-017 — Ingestion-time redaction policy**: every envelope
  passes through a redaction stage before persistence. Detector list:
  AWS keys, GitHub tokens, Anthropic keys, generic secrets (entropy +
  pattern), email addresses (PII), phone numbers (PII). Redacted
  fields are replaced with `[REDACTED:secret-aws-key]` etc.
- Existing `8debc3b` fix becomes one consumer of the shared module.

This is parallel to Phase A; it doesn't depend on the trait work.

---

## 10. The dashboard is a known liability — but not a Phase A target

**Severity**: P0 (user trust), but P2 (architectural)
**Confidence**: High

### Evidence

Doc 04 §"Worker is not a concept" cites the 2026-05-05 retention
daemon learning verbatim: "everything says never". That's a dashboard
bug. But the prior analysis defers the dashboard fix to Phase B.3
("Dashboard becomes pure reader").

### Why this matters

The user-visible signal of the rework is the dashboard. If Phase A
(traits) lands but the dashboard still hardcodes "never" for some
column, the user's perception is "still broken". Phase B.3 needs to
ship in lockstep with Phase A.1 (Sweep trait), not as a follow-up.

### How it changes the call

Reorder Doc 04 §"Phase B" — promote B.3 (dashboard becomes pure
reader) to PARALLEL with A.1. Specifically: when A.1's `Sweep` trait
+ `retention_sweeps` table land, B.3's dashboard handler that reads
that table also lands in the same PR.

This is a small reordering but it changes the user-perceived velocity
of the rework. "Phase A done, dashboard correct" reads very differently
from "Phase A done, dashboard fix in 3 weeks".

---

## Severity recap

| # | Concern | New severity | On critical path? |
|---|---------|--------------|-------------------|
| 1 | Config sprawl (344 envs) | P1 | Yes (add as A.5) |
| 2 | phase11v entrenches lane monolith | P0 (process) | Yes (defer or restructure) |
| 3 | Semantic test fidelity (system-wide) | P1 | Add as B.4 |
| 4 | Schema versioning with no migration | P1 | Add as ADR-016 |
| 5 | GUI ⇄ backend contract drift | P2 | Add as B.5 |
| 6 | 117 archived tasks not inventoried | P1 (planning) | Parallel hygiene |
| 7 | Test inventory gaps | P1 | Subsumed by Phase A gates |
| 8 | Pre-thinking over-engineered for current data | — | Calibration note only |
| 9 | Security/privacy track undocumented | P1 | Add as ADR-017, parallel |
| 10 | Dashboard as user-trust signal | P0 (user-perceived) | Promote B.3 → A.1 sibling |

**Net additions to the rework plan**: 2 ADRs (016, 017), 1 phase
addition (A.5), 1 phase rebalance (B.3 → parallel with A.1), 2
hygiene tasks (B.4 generalization, 117-task inventory).
