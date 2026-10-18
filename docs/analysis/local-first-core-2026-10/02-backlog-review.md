# Open backlog review

Date: 2026-10-17. Scope: all 18 older tasks in `.rulebook/tasks/`, plus inspection
of the two new phase36 tasks to protect concurrent work. The operator requested
review and explicitly authorized deleting old tasks that no longer make sense.

Architecture reference: [01-architecture.md](01-architecture.md) and
`.rulebook/tasks/phase36_local-first-core/design.md`. The approved target replaces
mandatory Docker, service workers and remote indexes with one local executable,
canonical files, SQLite indexes and simple graph links. This cleanup changes
task definitions and analysis records, not runtime data or legacy code.

## Historical source and removal semantics

Original revision: `2ba3d2f502f3f3955166ac272ae5d8b28c2aea05`.
All 15 removal candidates were checked against that revision: all 58 files,
including metadata, were tracked and unchanged; no extra files or reparse points
were present, and every resolved directory was inside `.rulebook/tasks/`.

Recover any original task document without changing the worktree:

```powershell
git show 2ba3d2f502f3f3955166ac272ae5d8b28c2aea05:.rulebook/tasks/phase29_graph-projection-unblock/tasks.md
```

Substitute any task ID/path from the table. References below to removed or
rewritten task files and line numbers refer to this original revision.

Removal means retirement of an active task definition, not erasure of delivered
work or a new assertion of acceptance. Nine removed tasks have every checkbox
marked, but their definitions no longer describe future work. Their original
text, code, tests, ADRs and existing archived tasks remain in Git/the repository.
Rulebook archival applies spec deltas and requires full quality gates; old deltas
include unresolved or contradicted requirements. This administrative cleanup uses
authorized task deletion rather than applying obsolete deltas or claiming those
gates have passed. No task is marked newly completed by this review.

## Disposition of every open task

Counts show the recorded checklist before this review, not independently proven
acceptance. Retained task IDs stay stable; their new scope starts unchecked.

| Task | Checked/total | Decision | Reason and retained value |
| --- | ---: | --- | --- |
| `phase26f_retrieval-nl-quality-and-observability-followups` | 5/9 | Remove | Remaining replay/reembedding/redeploy steps belong to the old service stack. Carry readable historical text and measured recall/latency into local import and evaluation; no destructive Vectorizer replay. |
| `phase27b_graph-community-detection` | 12/12 | Remove stale definition | Delivered legacy work; periodic Nexus communities are not required by the simple graph. Preserve ADR-027 and historical delivery evidence. |
| `phase27c_graphrag-community-summaries` | 11/11 | Remove stale definition | Haiku map/reduce and Nexus community processing are outside the approved core. Checked status did not establish every merge-plan integration; no automatic merge obligation transfers. |
| `phase27d_scip-precise-extraction` | 7/8 | Remove | Remaining rust-analyzer bootstrap/GraphPatch integration is not required. Keep parser, fixtures and ADR-037; any later SCIP import must be optional. |
| `phase27e_idf-graph-seed-selection` | 8/8 | Remove stale definition | Legacy integration delivered. Preserve the requirement that scope isolation includes statistics, caches and ranking, not only returned rows. |
| `phase28_docs-truth-reconciliation` | 10/10 | Remove stale definition | Documentation repair delivered; numbering/reference guards remain useful existing code. |
| `phase28_live-testing-bugfixes` | 9/9 | Remove stale definition | Fixes delivered with an upstream dependency caveat. Retain that caveat below; standalone dependencies need their own current audit. |
| `phase28_retrieval-eval-gate-live` | 20/20 | Adapt | Replace live service/unstable-ID evaluation with deterministic local fixtures, provenance, recall/MRR, latency, links and installed read-only smoke. Historical failed gates remain evidence below. |
| `phase29_graph-projection-unblock` | 14/14 | Remove stale definition | Old projection work delivered; Nexus scheduler assumptions do not transfer. Preserve evidence that populated graphs still failed useful questions. |
| `phase29_mcp-surface-doc-and-discovery` | 13/13 | Remove stale definition | Discovery work delivered. Carry equality of implemented, advertised and documented tools into standalone acceptance; no fixed legacy count of 41 tools. |
| `phase29b_hive-services-update-aug2026` | 11/11 | Remove stale definition | Old Synap/Nexus/Vectorizer updates are no longer core work. Preserve recovery/compatibility history; do not upgrade historical stores implicitly. |
| `phase30_live-e2e-smoke-and-doctor-wiring` | 15/15 | Remove stale definition | Service smoke already delivered. Carry functional read-only tool smoke and direct CLI/MCP checks with HTTP stopped into standalone acceptance. |
| `phase31_governance-engine-mvp` | 0/23 | Remove | Punitive laws, trust scores, asynchronous workers and nightly recomputation exceed the memory product. Keep explicit authority, scope and provenance safeguards in phase36. |
| `phase31_pre-thinking-audit-trail` | 0/7 | Adapt | Durable local evidence is needed to assess value. Distinguish prepared delivery, related observed actions, explicit evidence of use and unknown outcomes. |
| `phase32_embedding-model-eval` | 0/13 | Remove | Vectorizer/model/reembedding expansion is not needed for the initial lexical local index. Any future embedding investment must follow measured local retrieval gaps. |
| `phase32_feedback-loop-into-ranking` | 0/15 | Adapt | Keep attributable local feedback and an optional bounded ranking experiment, dependent on audit and evaluation. No automatic blame from missing citations. |
| `phase33_adapter-opencode-cursor` | 0/15 | Remove | Old adapter/Synap/watchers expansion lacks a current requirement. OpenCode already had a producer; its live acceptance was explicitly deprioritized. Existing integration code remains. |
| `phase34_daily-memory-recovery` | 0/22 | Remove duplicate | Its checklist explicitly says superseded; canonical `phase34_daily-agent-memory-recovery` is already archived. Never execute the old autostart/native-memory rewrite plan. |
| `phase36_local-first-core` | 1/11 | Preserve | Approved active replacement architecture and implementation. This review adds acceptance clarification to its design without changing checklist order. |
| `phase36_codex-adapter` | 0/13 | Preserve | New concurrent task, outside old-backlog cleanup. Its real Codex identity/payload evidence remains useful; standalone integration must respect those contracts. |

Resulting active set: three adapted follow-ups and two preserved new tasks.

## Findings and requirements that must survive

### F-001 - Checked boxes overstated retrieval acceptance

Evidence: original `phase28_retrieval-eval-gate-live/tasks.md:23` limits scheduled
evaluation to classification and access control despite its retrieval-nightly
spec. At `:27`, measured reranker p95 was 102 ms disabled versus 2224 ms enabled:
**+2122 ms**, failing the old +250 ms limit. At `:28`, phantom-link rate was
**0.50**, failing the 1% gate; changing corpus contents also forced floor changes.

Impact: the reset task must lock reproducible fixtures/baselines, report failed
gates and missing measurements, and separate synthetic, installed and observed
agent-use evidence. New architecture does not itself prove improved quality.
Confidence: high for recorded historical results; no current runtime measurement
is inferred from those old values.

### F-002 - Populated graphs and healthy endpoints did not prove usefulness

Evidence: original `phase29_graph-projection-unblock/tasks.md:19` reports **0/6**
useful pre-change graph results and **0/5** decision trails after population.
At `:20`, property-less Decision anchors and mismatched Topic properties explain
unanswerable queries. `phase26f_retrieval-nl-quality-and-observability-followups/tasks.md:17`
records a prior live-validation claim without actual execution.

Impact: local graph endpoints must resolve to inspectable records; acceptance
must use expected answers and provenance, not edge counts or reachable ports.
Prepared delivery receipts must never be reported as observed model consumption.
Confidence: high for the documented failure mode.

### F-003 - Scope and attribution must survive simplification

Evidence: original `phase27e_idf-graph-seed-selection/tasks.md:14` records aggregate
COUNT probes and a shared cache crossing principals, missed by unit-only checks.
Original `phase32_feedback-loop-into-ranking/proposal.md` documents feedback with
no repository/lane attribution. Original `phase31_pre-thinking-audit-trail/specs/pre-thinking/spec.md`
requires a misleading binary utilized/not-utilized heuristic.

Impact: preserve exact repository isolation across search, traversal, counts,
caches and ranking; retain unknown attribution. Time proximity or an absent
citation cannot prove consumption or assign blame. Classifier judgments remain
advisory and cannot change accepted authority or supersession.
Confidence: high for the schema/spec gaps and required safeguards.

### F-004 - Some historical obligations are obsolete, not newly satisfied

Evidence: original `phase28_live-testing-bugfixes/tasks.md:6` records a deferred
upstream `rmcp` advisory; checked items do not establish a clean current audit.
Original graph/observability deltas also differ from delivered scheduler/soak
and smoke behavior. `phase34_daily-memory-recovery/tasks.md:1` explicitly says
superseded. Existing `.rulebook/archive/2026-06-22-phase16b_opencode-smoke-validation/tasks.md:1`
records operator-approved WON'T-DO live acceptance.

Impact: do not apply these old deltas as new requirements, claim an advisory is
currently unresolved without checking the current lockfile, or restart obsolete
backlog. Preserve historical sources and evaluate current standalone dependencies
under phase36 acceptance.
Confidence: high for historical task state; current dependency vulnerability
status was not assessed in this documentation cleanup.

## Verification scope

This review validates task structure, disposition coverage, original Git
recoverability and active references. It does not run the runtime quality suite,
mark implementations complete or archive tasks. Core work and concurrent adapter
changes remain separate. Runtime archives, collected events, databases, volumes,
native memories, source code and existing `.rulebook/archive/` entries are not
deletion targets.

The configured Cortex pre-thinking call for this audit returned an internal
timeout/fail-open result, not usable memory. A Rulebook knowledge read succeeded;
its learning-list call failed with `Cannot read properties of undefined (reading 'localeCompare')`.
Repository evidence and current operator instructions governed the decisions.

Final Rulebook validation: all five retained tasks are structurally valid.
Warnings correctly retain unchecked implementation/docs/tests work; the concurrent
Codex task also has no spec delta yet. These are pending-work warnings, not
implemented acceptance. Independent review approved disposition coverage,
prerequisites, historical evidence and retained safeguards. Whitespace validation
passed for the task/documentation changes.

One deletion call timed out after removing
`phase29_mcp-surface-doc-and-discovery`; filesystem inspection confirmed the task
was gone before continuing, and the generated index contains exactly the five
retained tasks. No blind retry or wider deletion was used.

The lesson was saved through Rulebook as
`checked-task-boxes-do-not-establish-runtime-usefulness-or-acceptance`.
The configured legacy Cortex capture tool rejected the actual repository ID
`Cortex` because it requires lowercase. No lowercase scope substitution was made;
this review does not claim the lesson reached the daily Cortex store.
