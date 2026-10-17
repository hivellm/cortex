# Execution plan for daily memory recovery

Date: 2026-10-16. Rulebook task:
`phase34_daily-agent-memory-recovery`. This plan was finalized under **§1.1**.
Current implementation results and actual assignment changes are recorded in
`08-implementation-evidence.md`; the task checklist owns completion status.

## Authority and joint decision

`05-joint-decision.md` records the selected architecture and actual model
participation. The operator authorized this one recovery task before older
backlog. Only work necessary to useful daily memory is selected. The authoritative
execution sequence is the actual task's `tasks.md`, reproduced below in order.

Claude's separately authored `05-execution-plan.md` is retained as review input.
Its alternate task ID, sequence, and suggested OS autostart are not the execution
contract. No OS scheduling or Startup-folder workaround is introduced. Lifecycle
and any recurring work remain application-managed.

The operator, not Claude, restarted Docker at approximately 05:53 UTC today,
according to `04-claude-coordination.md`. Historical resource and failure
observations require remeasurement. Fable, Kimi, and Grok returned substantive
reviews through local 9router; response artifacts were inspected in temporary
`cortex-recovery-model-review` files for `cc-claude-fable-5-1`, `kimi-kimi-k3`, and
`xai-grok-4.6`. Their recommendations are design input, not runtime tests,
authorization, or proof that corresponding agent harnesses are installed.

## Selected implementation boundary

Existing Cortex SQLite gains a small durable decision/lesson projection with
FTS lookup, provenance, idempotency, scope revisions, and explicit supersession.
Reuse existing typed decision status/supersession semantics. A committed explicit
write becomes readable through this local path immediately, independent of GPU,
Synap, Meili, Vectorizer, bulk workers, and Jev. Full-stack indexes may mirror it.

Scope/lifecycle/source checks determine eligibility before ranking. Context has
at most five records and a 1,536-byte daily-memory section including citations.
The production pre-thinking deadline remains 600 ms end to end; optional stack
work cannot consume the time needed for the local section. Delivery records and
feedback persist. A prepared response is not proof a model consumed it.

Jev is optional, pinned to tested `jev-1.13.0`, and classifies only explicit
memory candidates. It may advise durability, kind, scope, and contradiction. It
cannot discard an explicit write, broaden scope, promote rules, author summaries,
or supersede records. Raw events get zero Jev calls in this recovery path.

Stop automatic rewriting of native memory first; preserve files and raw history.
No bulk native-memory repair, deletion, corpus migration, second memory service,
all-event classifier expansion, or unrelated backlog is implied.

## Ordered ownership and acceptance

Assignments below are prospective responsibilities. Model availability must be
recorded at execution; a model's design suggestion does not mean it has accepted
an implementation assignment. Codex coordinator assigns disjoint sub-work only
within the current first unchecked item. Claude edits use Opus 5.5 when available;
Codex edits use Sol; Astra and Fable handle architecture/review. If a named model
is unavailable, record the actual substitute rather than claiming participation.

| Exact task item | Accountable owner / model route | Acceptance and evidence |
| --- | --- | --- |
| §1.1 Finalize evidence-based analysis, joint model decision, ownership and behavioral acceptance for this one-task priority override. | Codex coordinator / Astra; Claude cortex-73, Fable 5.1, Kimi K3, Grok 4.6 review input | Current analysis, joint decision, this mapping, task design and Given/When/Then spec agree; exact task validated before code; historical observations distinguished from current checks. |
| §1.2 Disable automatic native-agent memory rewriting safely, preserve existing files, and test scheduler defaults and existing-install behavior. | Claude implementation owner / Opus 5.5; Codex / Astra review | Fresh and existing SQLite scheduler installations disable `retention.memory_consolidate`; default reseeding cannot restore it; pre/post file hashes prove native files unchanged. |
| §1.3 Implement compact durable decision/lesson storage in existing SQLite with provenance, repository isolation, idempotent writes, revision counter, FTS lookup and explicit supersession. | Codex storage owner / Sol; Astra review | Transactional capture/read; restart durability; stable retry returns same identity without revision bump; conflicting retry and stale expected revision fail; scope and successor tests pass; SQLite FTS capability checked. |
| §1.4 Expose durable memory capture/read and persistent feedback through API/MCP with honest caller attribution, delivery receipts and compatibility for existing capture clients. | Claude API/MCP owner / Opus 5.5; Codex / Sol contract review | Existing capture clients still work and unknown origin stays unknown; explicit write visible before workers; API/MCP receipts distinguish local commit, optional index mirror and prepared delivery; feedback survives restart. |
| §1.5 Integrate bounded Jev classifications for explicit memory candidates only, with redacted state, pinned model, typed validation, deterministic fallback, cache and measured budget. | Codex Jev owner / Sol; Astra and Claude review | Missing key/provider outage preserves core; explicit transient-labeled write remains durable; zero raw-event calls; redacted request capture, typed response/error/cache tests, actual usage and bounded call counts recorded. |
| §1.6 Inject relevant active memories into ordinary pre-thinking with strict byte/time bounds, source citations, revision-aware cache and no superseded or cross-project leakage. | Codex retrieval owner / Sol; Claude / Opus integration review | Exact seeded lesson arrives on ordinary prompt; repo distractors and superseded/stale sources excluded; successor appears within old cache TTL; section at most five records/1,536 bytes; production deadline exercised. |
| §1.7 Repair packaged Claude lifecycle delivery and provide verified MCP-based contracts for Codex, Grok and Kimi; test real installed harness paths without claiming unsupported hooks. | Claude harness owner / Opus 5.5; Codex coordinator / Astra verifies capabilities | Installed binary/version/config inventory; packaged Claude hook with adapter running emits actual context; each available client performs real MCP read/capture; absent clients reported unsupported/unverified, never replaced by model-provider calls or fake adapter stubs. |
| §1.8 Eliminate recurring whole-archive decompression from daily-memory operation and make newly captured archive events visible through a bounded incremental path when archival mode is enabled. | Claude archive owner / Opus 5.5; Codex / Astra review | Daily profile performs zero recurring archive decompression; archival mode reads only changed/new bounded units with durable checkpoints, retry deduplication and corruption visibility; append/restart tests prove new evidence appears without complete rewalk. |
| §1.9 Provide and validate a lightweight daily-memory runtime profile without mandatory GPU or bulk indexers, preserving full-stack data and explicit capability status. | Claude runtime owner / Opus 5.5; Codex / Sol measurements | Capture/read/feedback succeed with GPU and bulk services absent; status labels optional unavailable features honestly; profile changes preserve volumes/files; same-host idle CPU/RSS/IO and process counts compared with stated intervals. |
| §1.10 Run seeded cross-session and cross-model acceptance for exact recalled decisions, corrections, restart durability, distractor rejection, outage behavior and measured resource/latency budgets. | Codex acceptance owner / Sol; Astra evaluates; Claude operates real harness | Seeded fixture and actual tool/hook outputs prove exact identities, origin, successor, restart and outages; required jobs fail on absent dependencies; 600 ms/1,536-byte gates and resource measurements recorded; real client matrix discloses unavailable paths. |
| §1.11 Activate validated daily-memory integration locally, verify operator workflow, and reconcile operational documentation with observed behavior. | Codex coordinator / Astra with Claude / Opus local integration | Current build installed through authorized reversible path; adapter/app lifecycle verified, one visible prompt-to-memory demonstration captured; no OS scheduler; documentation matches actual capabilities and observed measurements. |
| §1.12 Complete independent code review with Sol/Astra, Claude, Kimi and Grok; fix findings and run full required quality gates before committing and archiving. | Codex coordinator / Astra; independent Sol, Claude, Kimi K3, Grok 4.6 reviewers | Actual patch reviews saved with model/route and limitations; findings resolved; type-check, lint and relevant tests per commit; full required suite before archive/push/PR; hooks respected; no unverified consensus claim. |
| §2.1 Update or create documentation covering the implementation | Claude docs owner / Opus 5.5; Codex / Sol verification | User workflow, profiles, provenance, scope, failure status, installed client matrix and CHANGELOG reflect final behavior; earlier docs reconcile observations. |
| §2.2 Write tests covering the new behavior | Codex test owner / Sol; Astra review | Audit tests written alongside §1 work; close any uncovered behavioral scenarios from spec without duplicating implementation-only tests. |
| §2.3 Run tests and confirm they pass | Codex coordinator / Astra; Claude runs applicable host checks | Required final gates green with commands/results, integration dependency status and honest client evidence retained; no silent skips counted as acceptance. |

Quality checks accompany each change; the tail audits completeness rather than
postponing tests until implementation ends. Commits cite exact sections and
remaining items when partial. No section can start ahead of an earlier unchecked
item except the documented project-law exemptions. A missing optional provider is
tested as outage behavior; it does not block core work. An unavailable required
installed harness is recorded through Rulebook's blocker contract, not claimed
as tested.

## Release gate and limits

Seed source session A with a correction in repo R, another repo S memory, an
unrelated distractor, and a predecessor/successor pair. Fresh session B reads
ordinary task context and must receive the exact applicable active ID plus source.
Explicit supersession must change the next identical prompt; restart must retain
that result and feedback. Jev and bulk stack outages must leave this result
available locally. Actual hook/tool output and stored delivery records must agree.

Do not equate MCP response preparation with model comprehension. Actual harness
tests establish delivery, and a paired task scenario establishes that the agent
chooses the corrected action. Model compliance beyond measured fixtures remains
a limitation, not an unconditional guarantee.

This plan adds no independent tasks and marks no checklist items. The design in
the existing task and its `specs/daily-memory/spec.md` define the implementation
contract; model recommendations that conflict with the joint decision are not
adopted implicitly.
