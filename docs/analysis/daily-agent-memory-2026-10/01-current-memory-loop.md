# Current memory loop and daily utility

Evidence date: 2026-10-16. Source inspection was read-only. Runtime observations
below were supplied by the coordinating agent from checks earlier today; they
are historical snapshots, not assertions about current process state. The
operator restarted the stack at approximately 05:53 UTC, as corrected by the
Claude session in `04-claude-coordination.md`; Claude did not restart it.
Recheck affected source and runtime evidence before implementation.

The smallest useful outcome is specific: a decision or verified correction from
one session changes the context and behavior of another agent in a later
session. Captured event counts, available containers, and a nonempty bundle do
not establish that outcome. Existing Cortex components provide much of the
required path, but capture, relevance, delivery, and feedback currently have
separate gaps.

## F-001 — Claude archive capture does not publish to live indexes

**Evidence:** `crates/cortex-workers/src/claude_archive/emitter.rs:181` implements
`ArchiveEmitter::emit` by writing compressed NDJSON. Its sink has no publisher;
the comment at `:16` describes Synap publishing as future work. In contrast,
`crates/cortex-workers/src/ingestion/router.rs:480` archives and then publishes
through the normal ingestion route. `crates/cortex-api/src/main.rs:359` selects
live Meili when its probe succeeds, while `:403` seeds archive data into the
separate `keyword_memory` lane.

**Impact:** A captured transcript can appear in an archive or fallback dashboard
without entering the active Meili/vector/graph retrieval path. Archive durability
must not be reported as successful live indexing. Recovery should converge
capture paths on durable ingestion with replay and an observable index receipt.

**Confidence:** High for source behavior. Actual indexed coverage needs a fresh
runtime check after the concurrent stack restart.

## F-002 — Periodic archive refresh repeats work over all history

**Evidence:** `crates/cortex-api/src/lanes/archive_loader.rs:75` recursively visits
the archive and collects hits; `:105` invokes that full load; `:113` seeds cloned
hit lists into three aliases. `crates/cortex-api/src/main.rs:417` starts the
periodic loop, which calls the same loader at `:424`. Earlier-today runtime
inventory, reported by the coordinator on **2026-10-16**, counted **68,949 archive
files, approximately 309 MB**. That inventory has not been repeated here.

**Impact:** Recurring decompression, parsing, allocation, and cloning grow with
total retained history rather than new evidence. This creates cost without
proving better recall. Prefer the existing ingestion stream for new events and
checkpointed replay for history. No runtime latency or CPU measurement was made
in this sub-investigation.

**Confidence:** High for repeated full scanning; medium for current archive size
because the measurement predates concurrent work.

## F-003 — Manual search can find captured lessons that daily injection misses

**Evidence:** `crates/cortex-api/src/search/strategies.rs:496` now includes
`code`, `docs`, and `misc` in `free_search`; its comment at `:499` identifies
`misc` as the destination for memory, knowledge, and learning. The earlier claim
that free search omits this family is therefore obsolete. However,
`crates/cortex-pre-thinking/src/intent_select.rs:15` documents
`pre_change_context` as the default, and that strategy's vector and keyword
families at `strategies.rs:117` and `:137` omit `misc`. The archive loader handles
turns, tools, agents, consolidations, and topic cards, then returns `None` for
remaining kinds at `crates/cortex-api/src/lanes/archive_loader.rs:371`.

**Impact:** A lesson can be captured and manually searchable while staying absent
from an ordinary prompt's context. Daily memory acceptance must exercise the
normal injection intent and exact expected lesson, not only manual free search.
Relevant active lessons need a retrieval path independent of phrasing heuristics.

**Confidence:** High.

## F-004 — Feedback storage exists, but the mounted route uses transient state

**Evidence:** `crates/cortex-storage/src/metadata.rs:384` supports feedback
upserts and `:1403` defines the SQLite feedback table.
`crates/cortex-api/src/feedback.rs:148` writes through its supplied store, but
`crates/cortex-api/src/http.rs:303` mounts feedback with a new
`MetadataStore::open_in_memory()` at `:311`. Production selects this router
builder at `crates/cortex-api/src/main.rs:964`; no replacement feedback wiring
was found. Earlier-today **2026-10-16** coordinator inspection found **zero rows
in local persistent feedback storage**. Zero rows alone would not prove this
wiring defect; the source establishes the separate transient store.

**Impact:** Feedback accepted by the current route disappears on restart and
does not populate the persistent database inspected by operations. Reuse the
existing file-backed metadata store and verify persistence through the actual
daemon route before adding new feedback infrastructure.

**Confidence:** High for wiring; the zero-row observation is a historical
snapshot and does not establish current counts.

## F-005 — Cross-agent capture discards origin and decision lifecycle

**Evidence:** `crates/cortex-mcp-server/src/tools.rs:890` defines capture input as
kind, body, repo, optional topic, and severity. The API counterpart at
`crates/cortex-api/src/ingest/proxy.rs:42` has the same limits. It creates a fresh
session ID at `:151` and stamps `tool: "claude-code"` at `:200`, irrespective of
the invoking MCP client. Accepted kinds are memory, knowledge, and learning at
`:113`; typed decisions already support status and supersession at
`crates/cortex-core/src/events.rs:691`.

**Impact:** Codex or Grok captures can be attributed to Claude and detached from
the originating turn. Free-form capture cannot express an accepted decision or
superseding correction as a lifecycle change. Preserve agent/session/turn,
source references, scope, source revision or hash, and explicit status using
existing event contracts where possible. Do not infer approval from model prose.

**Confidence:** High.

## F-006 — Context cache does not observe newly captured corrections

**Evidence:** `crates/cortex-adapter-claude-code/src/sync_paths.rs:41` describes a
cache keyed by prompt and cwd, with a 60-second TTL at `:44`. The path returns
cached output before querying at `:313`, and stores the bundle and fail-open
state at `:344`. The key has no corpus revision, decision version, or source
revision.

**Impact:** Repeating a prompt soon after recording a correction may return the
old context; an outage result may also remain cached briefly after recovery.
Add revision-aware invalidation or narrowly invalidate affected scope after a
successful capture/lifecycle change. A test must distinguish a new correction
from an identical repeated request.

**Confidence:** High for cache inputs and control flow; current user-visible
frequency has not been measured.

## F-007 — Packaged SessionStart hook can discard returned context

**Evidence:** `packages/cortex-claude-plugin/hooks/cortex-session-start.sh:8`
sets `SYNCHRONOUS=0`, selecting `--fire-forget` for the hook binary. Yet
`crates/cortex-adapter-claude-code/src/dispatcher.rs:281` builds SessionStart
active-work context and returns it at `:296`. The installer definition correctly
sets `fire_forget: false` at
`crates/cortex-adapter-claude-code/src/install.rs:54`.

**Impact:** Different installation paths provide different continuity behavior.
Testing only the Rust dispatcher or generated installer leaves the packaged
plugin path unverified. Verify the exact installed shim and returned
`hookSpecificOutput.additionalContext`, including Windows installation behavior.

**Confidence:** High for repository disagreement; current installed variant
requires inspection.

## F-008 — Fallback keyword lane does not perform relevant search

**Evidence:** `crates/cortex-api/src/lanes.rs:680` implements the memory keyword
lane by retrieving every seeded hit for an index at `:688`; it does not inspect
query text. `crates/cortex-api/src/main.rs:347` explicitly describes the prior
symptom of unrelated queries returning the same snapshot and falling back to
this implementation when live Meili is unavailable.

**Impact:** A nonempty fallback bundle can falsely suggest useful recall. Daily
memory needs deterministic, scope-aware relevant retrieval during degraded
operation, or an explicit degraded response that does not present unrelated
history as supporting evidence. Relevance tests must include distractors and
unrelated queries.

**Confidence:** High.

## F-009 — Existing continuity test does not prove the expected lesson arrives

**Evidence:**
`crates/cortex-pre-thinking/tests/cross_session_continuity_it.rs:46` returns
successfully when the API is unavailable; `:66` does likewise when the live
corpus lacks consolidations. The test uses a 20-second budget at `:117`. Its
comment at `:113` reports historical observations of approximately 700 ms and
production's 600 ms budget taking the fail-open path. Comments at `:130` explain
that the test intentionally does not assert which consolidation wins; assertions
at `:145` and `:154` check section and ID shape instead.

**Impact:** This is a wiring smoke test, not proof that the correct remembered
fact reaches a fresh session within the actual hook budget. Keep that purpose
explicit. Add a seeded integration gate with exact expected memory identifiers,
negative examples, real packaging, and configured production budgets. Required
integration jobs must fail on missing dependencies rather than silently pass.

**Confidence:** High for test semantics. The 700 ms figure is a historical source
comment, not a new measurement.

## F-010 — Activity and stored feedback do not establish behavioral learning

**Evidence:** `crates/cortex-api/src/feedback.rs:148` stores feedback but does not
update retrieval policy or memory lifecycle. Repository search found
`detect_citation` defined at
`crates/cortex-pre-thinking/src/implicit_feedback.rs:119` and exercised by its
own tests, without an adapter producer calling it. The coordinator's earlier
**2026-10-16** observations recorded **12 containers stopped at approximately
02:35 America/Sao_Paulo**, a daemon log last recorded on **2026-10-09**, and a
nightly result last recorded on **2026-10-11 with zero candidates**. These records
are not current liveness evidence: the operator has since restarted the stack.
The Claude coordination reply attributes that restart to approximately 05:53 UTC.
Old logs may also be from an obsolete launch path.

**Impact:** Count-based health or an old nightly record cannot show that a
decision was recalled, obeyed, or corrected. Persist query-to-delivered-memory
correlation and explicit outcome evidence. Citation overlap can be a weak signal,
not a correctness verdict. Revalidate process/log paths before attributing
operational root cause, and measure one useful cross-session behavior first.

**Confidence:** High for inspected feedback control flow; medium for operational
interpretation because historical observations predate concurrent recovery.

## Minimal design choices for review

Reuse durable ingestion, existing event IDs, typed decision supersession,
consolidation lineage, and the current bundle formatter. Source event IDs already
exist in `crates/cortex-core/src/events.rs:455`; decision response links exist in
`crates/cortex-api/src/types.rs:318`; topic-card age/confidence handling exists in
`crates/cortex-pre-thinking/src/formatter.rs:226`. Their existence does not prove
that every capture is grounded or current; expose and test those properties in
the daily path.

| Choice | Useful result | Tradeoff |
| --- | --- | --- |
| Repair existing ingestion, retrieval, injection, and feedback wiring | Smallest path to a verifiable daily loop | Historical aliases and incomplete runtime wiring still require careful verification |
| Add a compact memory projection within existing metadata only where needed | Durable active lessons, source validity, supersession, and delivery evidence | Requires migration and reconciliation with canonical events |
| Introduce an independent memory service | Isolated experimentation | Duplicates authority, provenance, retrieval, and lifecycle before daily benefit is established |

Preferred starting point: repair existing loop and add only missing durable
state. Jev can make bounded typed judgments about candidate extraction,
applicability, relevance, or contradiction when an evaluated use case warrants
it. Deterministic code should retain identity, source validation, lifecycle
transitions, publication receipts, limits, and delivery. Provider failure must
preserve captured evidence and a usable basic retrieval path. Independent
ai-memory and model reviews belong in the companion analysis files; this file
does not claim those reviews have occurred.

## Behavioral acceptance criteria

These are proposed acceptance criteria for the single recovery task under
review, not implementation tasks or an authorization to execute old backlog.

- A source session records a verified correction through real capture. A fresh
  session using another agent asks an ordinary task prompt. The exact applicable
  correction appears in delivered context with its source and correct origin.
- An unrelated lesson and another repository's lesson stay absent. Correct
  results are checked by stable memory/event identity and content, not by the
  existence of a section or a positive score.
- A superseding decision becomes authoritative; the predecessor remains
  traceable as history. Changed source revision/hash or branch triggers an
  explicit stale/verification state rather than silently authoritative advice.
- Captures, lifecycle changes, feedback, and delivery evidence survive daemon
  restart. Replay and publication retry preserve one logical memory and recover
  interrupted indexing. Archive acceptance and index completion are distinct.
- A newly captured correction invalidates affected cached context. The next
  identical prompt receives the update within the agreed delivery contract.
- The actual packaged Claude hook delivers context. Codex and Grok have an
  explicit verified retrieval/injection contract; sharing MCP tools alone does
  not establish automatic injection.
- A seeded end-to-end job checks expected recall, distractor rejection, source
  traceability, and actual configured time/byte budgets. Missing required
  dependencies fail that job; optional live smoke tests can remain separate.
- Jev evaluation uses reviewed examples and provider-failure cases. Success
  measures correct applicable recall and avoided wrong advice, rather than
  classifier availability, citation counts, or additional infrastructure.

## Scope boundary

Latest user direction authorizes **one new recovery task first**, limited to
important work tied to daily utility. Obsolete backlog is not automatically
authorized for execution. This analysis creates no task, changes no code, and
does not prescribe executing later checklist sections ahead of earlier ones.
The coordinator owns cross-model decisions and task materialization after the
joint review.
