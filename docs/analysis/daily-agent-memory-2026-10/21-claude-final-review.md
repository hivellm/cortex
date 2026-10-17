# Claude final review — §1.12

Date: 2026-10-17. Status: native v3 review completed; local triage completed.
Claude reported no proven P0/P1 defects and five possible P2 items. Local review
did not establish an actionable P0/P1/P2 defect from those items. This is a
two-file review, not whole-system approval or evidence of runtime tests.

The operator authorized selected Rust source snapshots for independent reviews,
excluding environment files, credentials and personal memory. This Claude packet
contains only the complete `memory_advice/mod.rs` and `provider.rs`
(440 lines), plus the coordinator's review instructions. Every numbered source
line and both full-file SHA-256 values matched the manifest before invocation.

The preserved v1/v2 packet included 800 and 440 lines respectively. Its SHA-256:
`8b929c6de56d3b90c0938a8260ce21255765ef642fed8ee956c3c9b3ea180b42`.
Manifest SHA-256:
`6f50aff88fa5073c301f648a265155ce052ea8bc571f42cc61738a2e1cb8d54b`.
Packet size: 60,003 bytes. Verification and raw run evidence are retained under
ignored `.cortex-state/daily-runtime/reviews/claude-*` paths.

The refreshed v3 packet contains the complete updated `mod.rs` (861 lines) and
unchanged `provider.rs` (440 lines). All numbered lines and full-file hashes were
verified again; source hashes were checked immediately before launch. The initial
packet, manifest and verification were preserved with the `claude-v1v2-` prefix.

| Refreshed v3 artifact | SHA-256 |
| --- | --- |
| Prompt, 61,549 bytes | `d094d6f9cef600d5aa9b9656ae7fb6d5dd7651e6d1a2912c502e211ce987bacc` |
| Manifest | `b0453b34bb73ef3bb7911856fb43ad446ff2260d65b35f8b84966c92f7fc1971` |
| `memory_advice/mod.rs` | `e751479330c0fceca88a768891144d4b08070dcf8e611d5cf62e1c7f940e43c6` |
| `memory_advice/provider.rs` | `08321d6271103668406e8a2f0f68488f4983fd9ca46f3d40961912cd1de18425` |

## Native execution boundary

Claude Code version: `2.1.291`. Requested model:
`claude-fable-5-1`. Native authentication was used without reading, extracting,
printing or placing provider keys in command arguments. Each invocation used a
new external temporary working directory, user settings only, maintained native
hooks, strict empty MCP configuration, `--tools ""`, a review-only system prompt,
`--permission-mode dontAsk`, `--no-session-persistence`, and verbose stream JSON.

The connected run's init record confirmed `claude-fable-5-1`, `tools: []` and
`mcp_servers: []`. Two native hook starts and responses were observed. No tool
calls or reviewer edits occurred. Runtime directories were outside the Cortex
repository, preventing project-local Cortex memory/rules from becoming the
review working-directory context.

## Preserved initial attempts

| Attempt | Boundary/result | Evidence |
| --- | --- | --- |
| v1 | Restricted sandbox: initialized native hooks/model but repeated `api_retry` with unknown transport error and no status. Stopped the uniquely identified owned Claude process after 112.213 s before retry. No assistant/result record. | `claude-run-v1.stdout.jsonl`, `.stderr.log`, `.meta.json` |
| v2 | Authorized execution outside the restricted sandbox: connected and streamed processing progress; hard timeout at 240.771 s, exit 1. No assistant/result record. | `claude-run-v2.stdout.jsonl`, `.stderr.log`, `.meta.json` |

Both attempts requested a maximum budget of US$0.50. Neither emitted a final
usage/cost result; usage and actual charge remain unknown. Processing progress is
not a code review or evidence of model approval. The v1 process was terminated
only after its unique review arguments were matched; a host `Stop-Process`
failure required `taskkill` for that confirmed process ID. The v2 timeout handler
terminated only its recorded owned process and descendants.

Coordinator independently identified a live idempotent reclassification problem:
a capture receipt's old scope revision can reject an unchanged candidate after
another capture advances the scope. The refreshed source takes a transactional
snapshot of the exact unchanged candidate at the current scope revision, updates
only the private advice receipt, and retains freshness checks after evaluation.
The original capture receipt remains unchanged. This was not a Claude finding.

The coordinator authorized v3 with `--effort low`, a US$1.00 maximum budget and
the same 240 s timeout. V3 runs outside the restricted sandbox using the same
native boundary. Its init record confirmed the requested model, no tools and no
MCP servers.

## Completed v3 evidence

Started at `2026-10-17T13:25:02Z`; exit code 0 after 95.589 s wall time. The
native result reported `subtype: success`, `terminal_reason: completed`,
`stop_reason: end_turn`, one turn, no permission denials and no spawned agents.
Reported native duration was 92.315 s (API duration 91.122 s).

The init, assistant and result/model-usage records identified
`claude-fable-5-1`. No reviewer tool calls or edits occurred. Native hooks remained
enabled; two hook starts and responses were observed. No transport retry was
observed in v3. Its final text is retained separately from processing-progress
events; no processing token stream is treated as review text.

Reported cost was **US$0.78545**. Reported usage: 2 input tokens, 24,494 cache
creation input tokens, 0 cache-read input tokens and 5,911 output tokens,
including 3,956 thinking tokens. These are the completed v3 result's figures;
they do not establish the unreported costs of v1/v2. The result reports list
cost, not an independently reconciled invoice.

Evidence retained under ignored `.cortex-state/daily-runtime/reviews/`:

- [Verified source manifest](../../../.cortex-state/daily-runtime/reviews/claude-manifest.json)
  and [packet verification](../../../.cortex-state/daily-runtime/reviews/claude-package-verification.json).
- [Native run metadata](../../../.cortex-state/daily-runtime/reviews/claude-run-v3.meta.json),
  [raw JSONL](../../../.cortex-state/daily-runtime/reviews/claude-run-v3.stdout.jsonl)
  and [stderr](../../../.cortex-state/daily-runtime/reviews/claude-run-v3.stderr.log).
- [Extracted final review text](../../../.cortex-state/daily-runtime/reviews/claude-run-v3.review.txt).

Ignored evidence links require this local workspace; they are not committed
review artifacts. The hashes, execution facts and findings are recorded here so
the result remains understandable without those files.

## Findings and local disposition

Line references below refer to the hashed v3 snapshot. Local triage used read-only
source inspection, including adjacent storage/runtime code omitted from the
external packet. No Rust changes or Cargo commands were made for this review.

| Claude item | Claim | Local disposition |
| --- | --- | --- |
| P2-1, `mod.rs:120-129` | A reservation can consume the allowance when the remaining network budget is zero. | Conservative reservation accounting is intentional: allowance is charged before network and retained for failures/crashes, without inventing provider usage. The proposed slow-SQLite scenario omitted `DailyMemoryState::run`'s 100 ms slot wait and 250 ms operation timeout (`daily_memory.rs:124-151`); two successful calls do not consume 1.65 s of storage waiting. A process descheduling or cancellation edge does not establish a contract violation. No actual provider bill is inferred from a reservation. No change. |
| P2-2, `mod.rs:106-110` | An undecodable cache entry repeatedly falls back without being invalidated. | A corrupt or externally changed row can fail closed repeatedly. Normal insertion follows typed validation, and storage also checks object shape/size before returning it. No valid producer path creating the alleged invalid row was identified. Corruption recovery is possible hardening, not a demonstrated regression. No change. |
| P2-3, `provider.rs:430` | A one-character configured key changes ordinary candidate text through literal replacement. | Literal replacement does have this effect for a deliberately invalid short credential. No evidence establishes that TypeSafe accepts such a credential or produces a successful judgment for it. Credential misconfiguration does not justify an invented minimum length that could reject future valid formats. Core committed content is untouched; remote failures fall back. No change. |
| P2-4, `provider.rs:407-415` | Secret values used as JSON object names escape sanitization. | Rejected for the supplied scenario. Candidate text is a string: after parsing and sanitizing nested JSON, `nested.to_string()` runs at line 427, then configured-key replacement and every extra-secret regex run across that entire string at lines 430-432. This includes serialized JSON names. Provider request object names are fixed, and arbitrary candidate objects are not inserted structurally into the request. The cited configured-key and `ts_...` examples therefore do not bypass these passes. No change. |
| P2-5, `provider.rs:376` | Probability normalization tolerance might reject rounded provider output. | Claude explicitly lacked evidence that Jev emits such output. Rejecting inconsistent distributions is a safe typed-answer fallback. Relaxing validation without a real provider-contract mismatch would weaken validation speculatively. No change. |

Claude's additional context questions were checked locally:

- **Cancellation/unfinished reservations:** `DailyMemoryState::run` uses
  `spawn_blocking`; started work can complete after its waiter times out.
  Storage intentionally retains unfinished charges and uses a three-second
  lease (`advice.rs:18` and `reserve_at`), so an abandoned attempt does not hold
  the lease indefinitely. Unknown usage remains unknown.
- **Moved usage:** `AdviceUsage` derives `Copy` (`advice.rs:43`); using it in the
  completion closure and response is valid.
- **Revision in cache key:** required by the task's design and bounded
  classification specification. Removing it because neighbors are currently
  empty would change the agreed cache invalidation contract.
- **Credential discovery:** the current-directory `.env` lookup belongs to the
  explicit production constructor. The review found no concrete launch-directory
  defect; no environment file was read or disclosed during this review.

## Coverage limits

Claude received only the complete two-file advice module, including its tests.
It could not inspect the ingest opt-in boundary, local executor or storage
implementation, and ran no tests. Its statements about durable commit, exact
repository isolation and bulk-service independence therefore cannot serve as
whole-system verification. The coordinator owns final rebuild, runtime
classification and task quality gates. Review success does not imply those gates
passed, nor does it replace Kimi/Grok's separate review evidence.
