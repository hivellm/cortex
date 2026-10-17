# ai-memory comparison and selective reuse

## Review boundary

Reviewed `akitaonrails/ai-memory` at commit
`11ca7c5b3b65e7fc216c7eae5a9aac67d38c11ea`, committed
2026-10-16 04:40:22 UTC. Its workspace declares version `3.1.0`.
Evidence comes from source and documentation fetched through GitHub; no upstream
software was installed or executed, and no performance claims were measured here.
Links below are pinned to that commit. Upstream support claims are not proof that
the user's installed clients expose the same capabilities.

The recommendation is to retain Cortex and port selected contracts after one
recovery task proves its current capture-to-recall path. This comparison does not
authorize a second backend, a migration, or adoption of either project's entire
roadmap.

## F-021 - Capture must reach recall before consolidation becomes useful

**Evidence.** Upstream's [architecture, line 45](https://github.com/akitaonrails/ai-memory/blob/11ca7c5b3b65e7fc216c7eae5a9aac67d38c11ea/docs/ARCHITECTURE.md#L45)
describes capture, durable observations, rule-based session pages, handoffs, and
optional LLM consolidation as separate stages.
[hook_spool.rs:1](https://github.com/akitaonrails/ai-memory/blob/11ca7c5b3b65e7fc216c7eae5a9aac67d38c11ea/crates/ai-memory-cli/src/commands/hook_spool.rs#L1)
implements local capture spooling with boundary drains and threshold-triggered
catch-up. Its bounds at lines 33-78 include a seven-day age limit, a 10,000-file
cap, delivery-attempt limits, and maximum entry size. Retry durability therefore
has explicit limits; it is not an unlimited lossless queue.

Locally, `crates/cortex-workers/src/claude_archive/emitter.rs:16` explicitly
describes archive-only output and visibility after daemon restart;
`crates/cortex-workers/src/claude_archive/emitter.rs:182` writes the archive
encoder without publishing to live indexing.

**Impact.** Cortex's first useful recovery must demonstrate that a newly captured
decision becomes searchable while the service remains running. Additional
summarization cannot compensate for absent delivery to the active search path.
The reusable upstream idea is a durable, retryable capture-to-index contract with
visible delivery state, not another summary model.

**Confidence.** High for the inspected paths; the runtime audit determines which
paths the deployed services actually use.

## F-022 - Durable decisions need provenance and versioned corrections

**Evidence.** [page.rs:1](https://github.com/akitaonrails/ai-memory/blob/11ca7c5b3b65e7fc216c7eae5a9aac67d38c11ea/crates/ai-memory-core/src/page.rs#L1)
defines working, episodic, semantic, and procedural memory with version
supersession. Evidence sources are represented at line 84.
[wiki.rs:2969](https://github.com/akitaonrails/ai-memory/blob/11ca7c5b3b65e7fc216c7eae5a9aac67d38c11ea/crates/ai-memory-wiki/src/wiki.rs#L2969)
describes the write path that updates Markdown and its index, short-circuits
unchanged SHA-256 content, and supports conditional writes against a page version.
[programmatic-memory.md:108](https://github.com/akitaonrails/ai-memory/blob/11ca7c5b3b65e7fc216c7eae5a9aac67d38c11ea/docs/programmatic-memory.md#L108)
documents `expected_page_id` and `create_only` for concurrent writers.

The upstream [durable-page routing skill, line 19](https://github.com/akitaonrails/ai-memory/blob/11ca7c5b3b65e7fc216c7eae5a9aac67d38c11ea/crates/ai-memory-core/src/routing_skills/ai-memory-durable-pages/SKILL.md#L19)
distinguishes explicit permanent notes from ordinary session capture. Its
project-rule guidance at line 55 puts canonical repository instructions first.
This skill is inspected product evidence, not instructions adopted by this review.

**Impact.** Remembering a correction requires a current decision, its source, and
the obsolete decision it supersedes. A pile of transcripts can contain both
answers without identifying which one applies. Cortex can adopt this contract
using its existing event/storage model; copying the Markdown implementation is
not necessary. Candidate records should preserve repository scope, source
session/event, originating client, version, and supersession. Automatic memory
must not silently override repository rules.

**Confidence.** High for upstream behavior; Cortex schema changes remain proposals.

## F-023 - Handoff requires ownership and atomic acceptance

**Evidence.** [handoff.rs:60](https://github.com/akitaonrails/ai-memory/blob/11ca7c5b3b65e7fc216c7eae5a9aac67d38c11ea/crates/ai-memory-core/src/handoff.rs#L60)
defines source/target agent, session, working directory, owner, summary, open
questions, next steps, and touched files.
[ops.rs:4519](https://github.com/akitaonrails/ai-memory/blob/11ca7c5b3b65e7fc216c7eae5a9aac67d38c11ea/crates/ai-memory-store/src/ops.rs#L4519)
performs acceptance inside a transaction with scope and receiver checks. The
guard at line 4596 prevents a retried session start from consuming a second
handoff.

**Impact.** A search result about recent work is not equivalent to an assignment
to continue that work. If Cortex later adds a shared handoff, it should carry
repository/worktree/session ownership and an atomic claim, so concurrent Claude
and Codex sessions do not consume or overwrite each other's continuation state.
This is useful after the recovery task; it is not a prerequisite for proving
basic durable recall.

**Confidence.** High for source behavior; local handoff implementation is not
authorized by this comparison.

## F-024 - Client capabilities determine capture and context delivery

**Evidence.** The pinned [support matrix, line 19](https://github.com/akitaonrails/ai-memory/blob/11ca7c5b3b65e7fc216c7eae5a9aac67d38c11ea/docs/support-matrix.md#L19)
documents distinct integrations. Source confirms why a universal session-start
script would lose context: [hook.rs:1200](https://github.com/akitaonrails/ai-memory/blob/11ca7c5b3b65e7fc216c7eae5a9aac67d38c11ea/crates/ai-memory-cli/src/commands/hook.rs#L1200),
[hook.rs:1287](https://github.com/akitaonrails/ai-memory/blob/11ca7c5b3b65e7fc216c7eae5a9aac67d38c11ea/crates/ai-memory-cli/src/commands/hook.rs#L1287),
and [hook.rs:1377](https://github.com/akitaonrails/ai-memory/blob/11ca7c5b3b65e7fc216c7eae5a9aac67d38c11ea/crates/ai-memory-cli/src/commands/hook.rs#L1377).

| Surface | Upstream integration | Material caveat |
| --- | --- | --- |
| Claude Code | Lifecycle hooks, MCP, session-start context | Must verify installed hook/config loading. |
| Codex CLI | Lifecycle hooks and MCP | Upstream claims native `SessionEnd` from CLI 0.145.0; older versions require a finalization fallback. |
| Grok Build CLI | Hooks/MCP; first-tool context delivery | `SessionStart` and `UserPromptSubmit` stdout are discarded. A routing rule requests MCP handoffs through `use_tool`; `PostToolUse` delivers context. |
| Kimi Code | Hooks/MCP; prompt context delivery | `SessionStart` stdout is discarded; `UserPromptSubmit` delivers handoff. |
| Native Windows | Release executable and native hook paths | Upstream support tier remains experimental. |

**Impact.** Cortex should describe adapters by tested harness and version, not
model name. Grok Build CLI support does not establish Grok web/desktop support.
Registering MCP proves tool access, not automatic capture or injection. Existing
client configuration and real smoke tests must determine the first supported
route. Adding all four adapters before that route works would enlarge the
recovery task without proving daily usefulness.

**Confidence.** High for inspected upstream contracts; local client behavior
remains unverified until exercised.

## F-025 - Relevant current decisions should outrank repeated history

**Evidence.** [reader.rs:449](https://github.com/akitaonrails/ai-memory/blob/11ca7c5b3b65e7fc216c7eae5a9aac67d38c11ea/crates/ai-memory-store/src/reader.rs#L449)
applies bounded authority adjustments after retrieval. Rules/decisions add 0.15,
procedures/gotchas/preferences add 0.12, and session pages subtract 0.15; the
combined factor stays within 0.55-1.50. Historical, superseded, and test-fixture
tags reduce authority without removing evidence from search.
[retrieval_tuning.rs:23](https://github.com/akitaonrails/ai-memory/blob/11ca7c5b3b65e7fc216c7eae5a9aac67d38c11ea/crates/ai-memory-store/src/retrieval_tuning.rs#L23)
describes an observation-evidence stream that lets a matching observation vote
for its session page, preserving access to details omitted by summaries.
[brief_tiers.rs:1](https://github.com/akitaonrails/ai-memory/blob/11ca7c5b3b65e7fc216c7eae5a9aac67d38c11ea/crates/ai-memory-hooks/src/brief_tiers.rs#L1)
allocates brief coverage before expanding page bodies.

The published [3.1 benchmark](https://github.com/akitaonrails/ai-memory/blob/11ca7c5b3b65e7fc216c7eae5a9aac67d38c11ea/docs/benchmarks/longmemeval-s-3.1.0-release.md)
reports LongMemEval-S FTS hit@5 of 0.951 and local-embedding hit@5 of 0.940, with
p95 latency of 101 ms and 225 ms respectively. These are upstream measurements
on a different corpus and machine, not Cortex results. The benchmark identifies
its own measured code revision and notes later release changes were not rerun.

**Impact.** Start with representative Cortex questions about decisions, failed
approaches, and corrections. Measure current recall and irrelevant-context
volume before adding vectors or copying ranking constants. Bounded excerpts
with source links are useful even when ranking remains unchanged.

**Confidence.** High for source behavior; published performance has not been
independently reproduced and does not predict local gains.

## F-026 - Feedback must survive restart and refer to content actually read

**Evidence.** [ops.rs:3653](https://github.com/akitaonrails/ai-memory/blob/11ca7c5b3b65e7fc216c7eae5a9aac67d38c11ea/crates/ai-memory-store/src/ops.rs#L3653)
records feedback, updates salience, and appends audit data in one transaction.
The comment at line 3699 explains that feedback applies to a page version;
superseding it starts new salience instead of condemning the path forever.

Cortex already exposes SQLite-oriented feedback operations in
`crates/cortex-api/src/feedback.rs:150`. Any claim that feedback is in memory
must identify the deployed store wiring, not imply that no persistence API
exists in the repository.

**Impact.** A repeated-error memory should preserve the correction, source,
and affected version across restart. A thumbs-down counter alone cannot teach
another agent what to do differently. Reuse existing persistence where practical
and prove restart behavior before introducing a second feedback system.

**Confidence.** High for upstream transaction; local persistence depends on
runtime wiring and the recovery verification.

## F-027 - Retry deduplication and semantic merging solve different problems

**Evidence.** The architecture's keyed delivery path separates observation
commit from completion of downstream effects; incomplete retries resume work
instead of adding another observation. Unchanged page writes use the SHA-256
short circuit described in [wiki.rs:2969](https://github.com/akitaonrails/ai-memory/blob/11ca7c5b3b65e7fc216c7eae5a9aac67d38c11ea/crates/ai-memory-wiki/src/wiki.rs#L2969).
Semantic near-duplicate clustering is separate:
[cold_cluster.rs:1](https://github.com/akitaonrails/ai-memory/blob/11ca7c5b3b65e7fc216c7eae5a9aac67d38c11ea/crates/ai-memory-consolidate/src/cold_cluster.rs#L1)
uses embeddings and DBSCAN, explicitly restricting O(N squared) pairwise work
to a bounded cold set.

**Impact.** The recovery task needs stable retry identity and duplicate-safe
processing. It does not need semantic clustering. Repeated observations should
not inflate authority, but merging distinct decisions prematurely can erase
important exceptions. Any later semantic merge needs provenance, reversibility,
and recall regression checks.

**Confidence.** High. No upstream clustering workload was executed here.

## F-028 - A smaller deployment does not guarantee cheaper maintenance

**Evidence.** [Cargo.toml](https://github.com/akitaonrails/ai-memory/blob/11ca7c5b3b65e7fc216c7eae5a9aac67d38c11ea/Cargo.toml)
declares a Rust workspace with bundled SQLite, while
[writer.rs:840](https://github.com/akitaonrails/ai-memory/blob/11ca7c5b3b65e7fc216c7eae5a9aac67d38c11ea/crates/ai-memory-store/src/writer.rs#L840)
creates a writer channel bounded at 1024 commands. The deployment guide requires
[one server per data directory](https://github.com/akitaonrails/ai-memory/blob/11ca7c5b3b65e7fc216c7eae5a9aac67d38c11ea/docs/deploy.md#L291).
Its approximately 700 writes/second result is a published fast-local-disk
measurement, not a local capacity measurement.

Critically, [watcher.rs:1](https://github.com/akitaonrails/ai-memory/blob/11ca7c5b3b65e7fc216c7eae5a9aac67d38c11ea/crates/ai-memory-wiki/src/watcher.rs#L1)
combines debounced events with a full wiki reconciliation every 30 seconds.
Unchanged content short-circuits, but the tree is still walked. Filesystem
deletions are not reconciled automatically.

**Impact.** Single-server topology is attractive compared with Cortex's current
service footprint, but adding ai-memory alongside Cortex increases moving parts.
Do not copy its periodic full scan as a solution to Cortex's archive rescan cost.
Choose changes from measured idle CPU/RAM, indexing latency, and actual usage;
no RSS reduction is established by this review.

**Confidence.** High for topology and scan behavior; resource savings unmeasured.

## F-029 - Reuse is permitted, but upstream defaults should not become policy

**Evidence.** [LICENSE](https://github.com/akitaonrails/ai-memory/blob/11ca7c5b3b65e7fc216c7eae5a9aac67d38c11ea/LICENSE)
is MIT, copyright 2026 Fabio Akita, and requires retention of the copyright and
permission notice in copies or substantial portions.
[config.default.toml:227](https://github.com/akitaonrails/ai-memory/blob/11ca7c5b3b65e7fc216c7eae5a9aac67d38c11ea/crates/ai-memory-cli/templates/config.default.toml#L227)
documents provider-backed automatic improvement with
`require_approval = false`, scheduler `enabled = true`, and an executable
evaluation gate disabled by default.

**Impact.** Selective source reuse is possible with attribution. Default
auto-approval of model-generated proposals should not silently become Cortex's
authority model. The useful initial contract is explicit decisions and
corrections with evidence. Existing repository rules remain authoritative;
inferred lessons can be evaluated separately after basic recall works.

**Confidence.** High for the pinned license and configuration defaults. This is
an engineering reuse assessment, not a review of every upstream dependency.

## Options and recommendation

| Option | Benefit | Cost and limitation | Recommendation |
| --- | --- | --- | --- |
| Adopt ai-memory as primary backend | Existing CLI capture, handoffs, versioned pages, and a smaller server topology | Migration, schema and governance differences, local Windows/client validation, and replacement of Cortex responsibilities | Consider only if the user later prioritizes immediate CLI continuity over retaining Cortex's broader capabilities. |
| Run both as permanent peers | Can compare behavior with separate stores | Duplicate capture, competing sources of truth, extra retention/ranking and resource costs | Do not make this the default architecture. |
| Port selected contracts into Cortex | Retains existing integration and evidence; fixes the path the user already owns | Requires proving local runtime behavior and maintaining capability adapters | Preferred, beginning with one bounded recovery task. |
| Keep Cortex unchanged | No migration effort | Known gaps between capture, persistence, retrieval, and visible utility remain | Insufficient for the requested daily memory outcome. |

The first delivery should demonstrate one complete workflow using the current
Cortex stack: record a decision and failed approach, retrieve them live from a
second configured agent, record a correction, restart, and retrieve the corrected
version with evidence. Duplicate delivery must not create duplicate logical
memory, and another repository must not receive project-specific context.

These are acceptance observations for the single recovery task, not a new
multi-phase backlog. Reuse existing storage, event IDs, API, and MCP routes when
they can satisfy the contract. Add only the missing links shown by that test.
Typed handoffs, more client adapters, richer ranking, and semantic consolidation
remain candidates to consider only when the recovered workflow demonstrates a
specific remaining need. No upstream installation or additional implementation
task is implied by this comparison.
