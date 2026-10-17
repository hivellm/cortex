Review uses supplied excerpts only. Source locations below are inherited citations, not fresh verification. Where 04 or 05 supplies no source line, that limitation is explicit.

### F-031 — Stop known harm before restoring normal operation

**Evidence:** `05-execution-plan.md` puts capture recovery before disabling consolidation and repairing auto-memory. `04-claude-coordination.md` §4 F-C1 explicitly requires stopping harmful writes first; F-C2 identifies obsolete `/v1/laws/check` calls. Neither supplies a source line. Separately, `01-current-memory-loop.md` F-007 cites `packages/cortex-claude-plugin/hooks/cortex-session-start.sh:8` for asynchronous SessionStart packaging, yet 05 postpones that fix until after its live trace.

**Impact:** Known damage continues during recovery. Initial verification can exercise packaging already known to discard responses. Excluding governance development does not justify retaining its broken request.

**Recommendation:** Add containment as step zero: disable consolidation, back up affected files, reconstruct content and `MEMORY.md` links, and verify preservation. Do not blindly split nested consolidated headings. Coordinate with phase31 owner to disable/no-op only obsolete governance calls. Move synchronous SessionStart wiring before live-hook verification.

### F-032 — Success proves recall delivery, not “errors avoided”

**Evidence:** `05-execution-plan.md` success criteria assert lesson presence, supersession, restart survival, and budgets. They never assert changed behavior. `01-current-memory-loop.md` F-010 cites `crates/cortex-api/src/feedback.rs:148` and `crates/cortex-pre-thinking/src/implicit_feedback.rs:119`: storage and citation signals do not establish learning. F-009 identifies weak presence assertions at `crates/cortex-pre-thinking/tests/cross_session_continuity_it.rs:145` and `:154`.

**Impact:** An agent can receive correct advice, repeat known failure, and still pass every proposed success criterion. Rising capture counts remain operational diagnostics.

**Recommendation:** Extend seeded scenario with an independently checkable action or patch. Session B must follow recorded decision and avoid recorded failed approach; after correction, its next action must follow successor. Check resulting artifact or tool action, not model self-report. Keep exact context assertions as separate evidence explaining behavioral result.

### F-033 — “Live trace” omits durable capture and retry contracts

**Evidence:** `05-execution-plan.md` §1 requests a trace through ingestion, Synap, classifier, Meili, and vector, but specifies no capture-path repair. `01-current-memory-loop.md` F-001 contrasts archive-only `crates/cortex-workers/src/claude_archive/emitter.rs:181` with publication through `crates/cortex-workers/src/ingestion/router.rs:480`. `03-ai-memory-comparison.md` F-027 distinguishes retry identity from semantic deduplication, including unchanged-content handling at upstream `crates/ai-memory-wiki/src/wiki.rs:2969`.

**Impact:** One successful trace can hide lost captures, duplicate memories, or unrecoverable partial indexing. Requiring every infrastructure component also expands critical path beyond useful lesson recall.

**Recommendation:** Explicitly route supported lesson/decision captures through durable ingestion with stable retry identity, replay checkpoints, and observable downstream completion. Distinguish accepted capture, materialized active state, and completed indexing. Test interrupted publication and duplicate delivery. Require only indexes used by chosen retrieval contract; vector and classifier completion need not block structured lesson delivery.

### F-034 — “Structured state” needs a named authority and lifecycle

**Evidence:** `05-execution-plan.md` §3.3 never identifies authoritative state or how it changes. `01-current-memory-loop.md` F-005 cites existing decision status/supersession at `crates/cortex-core/src/events.rs:691`; its design section cites source event IDs at `events.rs:455`. F-004 identifies shared persistence capabilities at `crates/cortex-storage/src/metadata.rs:384`, alongside incorrect transient routing at `crates/cortex-api/src/http.rs:311`.

**Impact:** A second independently maintained lesson table can disagree with canonical decisions, resurrect superseded advice, or silently preserve stale source assumptions. Provenance fields alone cannot implement successor authority.

**Recommendation:** Specify:

`durably accepted canonical events → idempotent durable projection → scoped active-lessons lookup → bundle formatter`

Canonical typed decision events own acceptance and supersession. A rebuildable projection in shared file-backed metadata supplies authoritative **read state**. Reuse an existing durable materialization if suitable; excerpts do not establish that an active-lessons table already exists.

Projection must preserve origin, source references, scope, version, status, supersession, and applicable source validity. Archive search, Meili/vector results, Claude auto-memory files, and Jev judgments must not independently determine lifecycle authority. Preserve repository-instruction precedence.

### F-035 — Active plus matching repository does not establish applicability

**Evidence:** `04-claude-coordination.md` §5.3 makes active status and repository/global scope determine membership; similarity only orders. `05-execution-plan.md` adopts that design while requiring unrelated distractors absent. `01-current-memory-loop.md` F-008 cites query-independent fallback at `crates/cortex-api/src/lanes.rs:680` and `:688`.

**Impact:** Every active same-repository distractor remains eligible. Sorting five irrelevant lessons still injects irrelevant lessons. Fixed item limits also cannot guarantee an applicable correction survives when eligible records exceed budget.

**Recommendation:** Separate standing repository instructions from conditional lessons. Enforce lifecycle, scope, source validity, and explicit applicability conditions before budgeting. Conditional lessons without adequate applicability evidence should remain searchable without automatic injection. Similarity may order eligible lessons; it cannot establish authority.

Test same-repository active distractors, more-than-budget candidates, and lessons limited to another branch/file/tool. Treat item count and byte cap separately; never fill unused budget with unrelated advice.

### F-036 — Revision keys need atomic state changes and recovery invalidation

**Evidence:** `05-execution-plan.md` §3.4 specifies a corpus-revision key without commit semantics. `01-current-memory-loop.md` F-006 cites early cache return at `crates/cortex-adapter-claude-code/src/sync_paths.rs:313` and caching of bundle plus fail-open state at `:344`.

**Impact:** Incrementing revision before projection updates can cache old advice under new revision. Asynchronous indexes can still return predecessor after successful capture. Cached outage results can survive recovery without any corpus change.

**Recommendation:** Commit visible lifecycle/projection change and revision together. Read a coherent state/version when building context. Define when capture or supersession acknowledgement guarantees that next prompt sees committed state.

Invalidate or separately bound degraded cache entries on recovery. Record delivery evidence for every prompt, including cache hits, with exact lesson IDs and versions. Distinguish emitted hook output from independently observed client delivery; neither proves model compliance.

### F-037 — Another agent label is not another working client

**Evidence:** `04-claude-coordination.md` §6 proposes a “different agent label”; 05 inherits that test while excluding new adapters. `01-current-memory-loop.md` F-005 identifies synthetic session IDs and hard-coded origin at `crates/cortex-api/src/ingest/proxy.rs:151` and `:200`. `03-ai-memory-comparison.md` F-024 cites client-specific delivery behavior at upstream `crates/ai-memory-cli/src/commands/hook.rs:1200`, `:1287`, and `:1377`.

**Impact:** Relabeling Claude traffic can pass while Codex receives no context. MCP registration alone proves neither automatic injection nor reliable retrieval.

**Recommendation:** Name actual installed second harness and version. Exercise its real retrieval/injection contract. Derive provenance from trusted integration context; represent unavailable originating session information honestly.

If second client uses explicit MCP retrieval, describe and test that workflow accurately. If ordinary-prompt automatic injection remains required, an existing verified integration must satisfy it; otherwise revise adapter scope explicitly.

### F-038 — Outage warning belongs in `systemMessage`, not `additionalContext`

**Evidence:** `05-execution-plan.md` §1.3 says “tells the session once” without specifying recipient or channel. `01-current-memory-loop.md` F-007 cites context construction at `crates/cortex-adapter-claude-code/src/dispatcher.rs:281`, return at `:296`, and correct synchronous installer configuration at `crates/cortex-adapter-claude-code/src/install.rs:54`. Supplied excerpts do not reproduce Claude Code’s hook-output schema.

**Impact:** `hookSpecificOutput.additionalContext` adds model context; it does not reliably establish operator visibility. Successful plain stdout from SessionStart/UserPromptSubmit can also enter model context. Fire-and-forget prevents reliable returned warnings.

**Recommendation:** Use synchronous exit-0 stdout JSON with top-level `systemMessage`:

```json
{
  "systemMessage": "Cortex adapter unreachable. Hook capture and automatic context injection are unavailable; this session continues."
}
```

Reserve `hookSpecificOutput.additionalContext` for actual memory. When present, set matching `hookSpecificOutput.hookEventName` to `SessionStart` or `UserPromptSubmit`.

Do not use `continue: false`, `stopReason`, blocking decisions, or exit 2 for ordinary outages. `suppressOutput` controls transcript visibility; it is neither notification routing nor deduplication. Omit it until installed-client rendering is tested.

Keep an atomic hook-local warning marker keyed by actual session ID, shared across both events. Missing daemon cannot own this marker. Warn at SessionStart when already down, otherwise first affected UserPromptSubmit. Recovery must not reset once-per-session latch.

Require real-client verification: visible warning once, continued prompt execution, no repeated outage `additionalContext`, and correct behavior through installed Windows and packaged hooks.

### F-039 — Ingestion silence is not liveness; login autostart is not supervision

**Evidence:** `05-execution-plan.md` §1 proposes Startup-folder launch and unhealthy status after no ingested event. Its root-cause section names `scripts/setup/install-adapter-autostart.ps1` without a line. `04-claude-coordination.md` §5.2 expects only a few lesson/decision captures daily. `01-current-memory-loop.md` F-001 cites independent capture/publication behavior at `crates/cortex-workers/src/ingestion/router.rs:480`.

**Impact:** Healthy idle sessions become unhealthy; unrelated background ingestion can mask dead adapter. Startup-folder launch repairs login startup but does not recover a later crash. Reported slow health endpoint can itself defeat detection.

**Recommendation:** Separate adapter liveness, intentional inactivity, pending-capture age, and downstream progress. Assess ingestion stalling when work is outstanding, not merely when time passes. Keep health response bounded below configured healthcheck timeout.

Verify executable/config identity, current-session startup, single-instance behavior, and bounded crash recovery with backoff. Test adapter termination and restoration. No-admin launch mechanism alone does not close F-030.

### F-040 — Candidate-only Jev is better scoped, but changes 02’s contract

**Evidence:** `02-jev-classification.md` selected scope explicitly excludes memory-write gates and candidate extraction. F-014 defines 49 questions from topics at `crates/cortex-workers/src/classifier/prompt.rs:7` and per-event invocation at `crates/cortex-workers/src/classifier_worker/worker.rs:1061`. `05-execution-plan.md` §4 instead introduces durability, scope, and contradiction judgments.

**Impact:** All-events enrichment spends effort outside broken capture/delivery path. However, candidate judgments are a different feature, not a reduced implementation of existing overlay. Old field tests and thresholds cannot validate new policy.

**Recommendation:** **Agree with deferring all-events overlay.** Supplied evidence identifies missing plumbing and authority, not topic quality, as immediate blockers. Preserve overlay proposal for a measured future need.

Run deterministic useful-memory baseline before Jev work. Keep candidate judgments opt-in and justify them through measured benefit. Explicitly revise 02’s selected scope and acceptance for this task. Do not add injection-time inference to 600 ms path without separate evidence that it helps and fits.

### F-041 — Jev must not decide storage, global scope, or supersession

**Evidence:** `04-claude-coordination.md` §5.5 proposes scope classification and pairwise contradiction checks, allowing supersession after confirmation “or a typed decision event.” `01-current-memory-loop.md` F-005 cites typed lifecycle at `crates/cortex-core/src/events.rs:691`. `03-ai-memory-comparison.md` F-029 warns against adopting automatic approval defaults documented at upstream `config.default.toml:227`.

**Impact:** A typed event is a schema, not proof of authorization. A “global” judgment can spread repository-specific guidance elsewhere. A durability rejection can discard an explicit correction. Contradiction can reflect different conditions rather than replacement.

**Recommendation:** Persist captures before inference. Explicit accepted decisions must not depend on a Jev durability verdict. Inferred candidates can remain pending.

Default scope to originating repository; broader scope requires authorized declaration. Supersession requires validated authority and expected predecessor version. Jev may propose transitions only.

Keep contradiction distinct from supersession. Record compared lesson IDs/versions and comparison coverage. Negative results over top-k candidates do not establish corpus-wide absence of conflicts. Preserve valid exceptions and historical predecessors.

### F-042 — “F-016 applies” carries too little of Jev’s operational contract

**Evidence:** `05-execution-plan.md` §4 carries forward only egress rules explicitly. `02-jev-classification.md` F-017 identifies inadequate fallback at `crates/cortex-workers/src/classifier_worker/worker.rs:1304`; F-019 identifies incomplete cache identity at `crates/cortex-workers/src/classifier/cache.rs:106` and missing container secret delivery at `docker-compose.yml:182`; F-018 identifies per-output cent rounding at `crates/cortex-workers/src/classifier/stats.rs:22`.

**Impact:** Safe redaction alone does not prevent blocking capture, stale contradiction judgments, fabricated fallback verdicts, or misleading experiment cost.

**Recommendation:** Carry forward applicable requirements: pinned/resolved model, versioned questions, typed validation, bounded state/response sizes, attempts/deadline, complete-state sanitization, secret-safe configuration/logging, and honest usage accounting.

Candidate failure means “judgment unavailable”; static event enrichment cannot fabricate durability or contradiction verdicts. Preserve deterministic capture and retrieval.

Cache judgments against candidate context plus compared lesson versions or relevant state revision. Exclude transient failures from successful-inference cache. Evaluate reviewed English/Portuguese examples, uncertainty, conditional contradictions, embedded instructions, and provider failures. Bound live sample explicitly; report token-based cost.

### F-043 — Broken fallback is correctness work, not late footprint cleanup

**Evidence:** `05-execution-plan.md` leaves archive reload removal until §5, after lessons and Jev. `01-current-memory-loop.md` F-002 cites full scans and cloned aliases at `crates/cortex-api/src/lanes/archive_loader.rs:75`, `:105`, and `:113`; F-008 identifies irrelevant fallback results at `crates/cortex-api/src/lanes.rs:688`.

**Impact:** Earlier recall tests can be contaminated by arbitrary history. Redundant scanning also competes with production latency budget. Slowing another full-tree walk changes frequency, not scaling behavior.

**Recommendation:** Before recall acceptance, replace fallback with bounded relevant structured lookup or explicit degraded output that injects no unrelated history. Then remove full archive reload.

Measure footprint baseline early; make nonessential GPU reranking and bulk indexing optional before production-budget demonstration. Treat slower archive polling as interim tuning, with incremental/checkpointed processing justified by measured remaining cost. Avoid requiring infrastructure merely because current topology contains it.

### F-044 — Root-cause narrative overstates evidence and names wrong archive format

**Evidence:** `05-execution-plan.md` F-030 says archive tailer “only writes parquet” and missing adapter means “nothing reaches ingestion.” `01-current-memory-loop.md` F-001 identifies compressed NDJSON output at `crates/cortex-workers/src/claude_archive/emitter.rs:181`. F-005 identifies separate API capture input at `crates/cortex-api/src/ingest/proxy.rs:42`.

**Impact:** Wrong format can misdirect replay repair. Adapter failure establishes failure of adapter-dependent traffic, not absence of every ingestion producer. Binary dates establish age, not exact compiled revision.

**Recommendation:** Replace “parquet” with “compressed NDJSON” unless newer source demonstrates another deployed path. Narrow outage conclusion to adapter-dependent capture/injection; establish classifier and other-producer stoppage separately. Preserve supplied host observations while distinguishing observations from causal inference. Verify installed executable paths and build identities during reinstall.

## Recommended tasks.md changes

- Insert first section: **“Contain existing harm: disable `retention.memory_consolidate`; back up affected auto-memory files; reconstruct content and `MEMORY.md` links; verify preservation; coordinate removal/no-op of obsolete `/v1/laws/check` request.”**
- Move synchronous SessionStart fix from §2 into §1, before live-hook verification. Require installed global and packaged Windows paths.
- Replace §1 autostart item with: **“Install verified current binaries; launch adapter now and at login without admin; enforce single instance and bounded crash recovery; verify executable/config identity.”**
- Replace §1.3 with: **“Emit synchronous exit-0 `systemMessage` once per actual session when adapter is unreachable. Deduplicate locally across SessionStart/UserPromptSubmit. Keep outage text out of `additionalContext`. Verify operator-visible rendering and continued prompt execution.”**
- Add §1 health item: **“Distinguish adapter liveness, idle state, pending-capture age, and downstream completion. Bound health endpoint latency. Test adapter termination, missed work, and recovery.”**
- Replace all-components live trace with: **“Trace a real correction through durable ingestion, stable retry identity, active-state materialization, and required indexes. Distinguish capture acceptance from index completion; test duplicate delivery and interrupted publication/replay.”**
- Replace §3.3 with: **“Read active lessons from durable projection of canonical typed events in shared file-backed metadata. Validate lifecycle, scope, applicable source revision, and applicability before budgeting; retain history and repository-rule precedence.”**
- Add §3 lifecycle item: **“Implement authorized, version-checked acceptance and supersession. Preserve trusted client provenance and source references. Test stale source/branch conditions and concurrent updates.”**
- Replace §3.4 with: **“Commit projection changes and corpus revision atomically; build context from coherent state; invalidate recovered outage entries; verify next identical prompt sees acknowledged successor.”**
- Expand delivery evidence item to: **“Persist per-prompt lesson IDs/versions and delivery stage, including cache hits, through shared file-backed store. Verify feedback and evidence survive restart.”**
- Move fallback repair into §3. Move remaining footprint work before Jev; run deterministic production-budget baseline after lesson delivery and footprint changes.
- Replace §4 with: **“Opt-in advisory Jev judgments after durable candidate capture. No veto of explicit decisions, automatic global promotion, or unauthorized supersession. Carry forward applicable 02 transport, security, bounds, cache, configuration, accounting, and evaluation requirements; document changed scope.”**
- Expand acceptance: **“Use actual second installed harness/version; assert exact applicable correction and observable compliant action; reject cross-repository and same-repository distractors; verify supersession, source validity, retry recovery, restart persistence, and configured time/byte limits. Missing required dependencies fail.”**
- Keep final docs/full quality gate, but run seeded baseline before Jev and rerun after enabled changes. Correct archive-format and outage-scope claims in accompanying documentation.