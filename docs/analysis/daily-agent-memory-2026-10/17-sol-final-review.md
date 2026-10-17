# Sol independent final review — §1.12

Date: 2026-10-16. Route: Codex Sol reviewer. Scope: optional Jev advice,
durable advice admission/cache/accounting, incremental archive decoding/indexing,
API publication/forgetting, and Claude archive producer boundaries.

Review began read-only. Coordinator subsequently authorized narrowly owned fixes
in `claude_archive/emitter.rs`, `claude_archive/tail.rs` and the bootstrap binary.
No Cargo commands,
provider calls, runtime/config changes, commits, staging, or corpus rewrites were
performed by this reviewer. Coordinator owns combined type-check/lint/test gates.

## Findings and disposition

### P1 — producer wire records cannot enter the typed incremental index

`crates/cortex-workers/src/claude_archive/emitter.rs`, `envelope_to_value`
(currently line 281); consumer:
`crates/cortex-storage/src/archive_index.rs`, envelope decode inside `poll`.

Before correction, the producer emitted `stream: "Bootstrap"` and omitted required
`content_hash` and `context.platform`. `Envelope` accepts lowercase `bootstrap`
and requires both fields. Every such record was rejected as a malformed envelope;
the index still advanced its checkpoint. Existing producer tests decoded generic
JSON values and explicitly asserted the incompatible stream spelling, so they did
not exercise this boundary. This is a preexisting producer defect exposed by the
§1.8 end-to-end contract, not a newly introduced enum mismatch.

Fix applied: serialize the canonical `Envelope` type; compute the payload hash
with `cortex_core::content_hash`; propagate serialization/hash failures before
writing. Preserve source IDs and raw payload. Platform describes the import host
and is explicitly labeled `platform_origin: import_host`, with original source
platform recorded as unknown. Missing repository remains missing; no basename
heuristic broadens scope. Existing invalid archives remain untouched.

Regression coverage added: canonical typed decode/hash assertion; unknown repo
and absent outcome preservation; real `ArchiveEmitter` output consumed by
`ArchiveIndex`, then appended and reopened with both exact Turn identities and
payload text retained. Execution pending coordinator gates.

### P2 — failed tail emissions were permanently deduplicated

`crates/cortex-workers/src/claude_archive/tail.rs`, `TailWatcher::tick_once`
(currently lines 296–375).

Before correction, the file cursor advanced before reading/emitting and each event
ID entered `emitted_ids` before `emit` returned. A transient write/sync failure
therefore disappeared from future attempts: unchanged files were skipped; changed
files suppressed the failed ID. This is also preexisting behavior relevant to the
durable producer contract.

Fix applied: insert an ID only after acknowledged emission; advance the file
cursor only after its full mapped batch succeeds. Stop on the first failed emit,
surface degraded tick evidence, and retry the unchanged source next tick.
Previously successful IDs suppress duplicate prefix emission. No pending payload
queue was added; existing source reread behavior remains.

Regression coverage added:
`failed_emit_retries_unchanged_source_without_repeating_successful_prefix`
fails the second write, retries without touching the source, verifies exact
`u1/u2/u3` successful identities with no duplicate prefix, then verifies an idle
third tick. Execution pending coordinator gates.

### P2 — bootstrap resume could checkpoint past failed emissions

`crates/cortex-workers/src/bin/cortex-claude-archive.rs:329`: bootstrap logs an
`emit` failure but still advances `last_uuid`; lines 348–361 store the whole-file
checkpoint. `--resume` at lines 299–307 skips every file already represented in
that checkpoint. After a transient disk failure, a resumed bootstrap therefore
cannot recover rejected events from the skipped file.

This is preexisting explicit bootstrap behavior, separate from the tail watcher
and the incremental archive reader. Coordinator explicitly expanded ownership to
fix the same durable-boundary defect here. A failed emit now stops that file,
removes any prior whole-file checkpoint, preserves completed-file progress and
returns a visible error after final accounting/checkpoint persistence. Resume
retries the incomplete file; the logical archive index deduplicates successful
prefix events. Physical duplicate prefix frames remain possible, as with crash
recovery under the existing file-granularity checkpoint contract.

Regression coverage added:
`bootstrap_resume_recovers_failed_file_without_new_source_append` injects a
second-write I/O failure around the real archive sink, checks that no completed
file checkpoint was written, invokes production bootstrap with resume against
the untouched source, and checks exact unique `u1/u2/u3` archive projections.
Execution pending coordinator gates.

## Reviewed controls with no additional finding

- Advice production requests contain only the explicitly opted-in candidate;
  absent/false `classify` never reserves or calls the provider. State redaction,
  fixed endpoint/model/schema, redirect rejection, response byte limits, typed
  probabilities/options and advisory-only publication were inspected.
- Advice cache identity includes candidate identity/version, scope revision,
  sanitized state, model and schema. Admission uses an immediate SQLite
  transaction, a persisted daily limit and a short global lease. Finish checks
  current revision, source identity, record version/hash/status atomically before
  caching. Crashes retain their budget charge and unknown usage.
- Incremental decoding caps compressed/decoded bytes, line bytes, lines, window
  size and live readers. Safe frame anchors plus replay ordinals preserve partial
  lines. Projection and cursor writes share a transaction; failed commits discard
  speculative reader state. Cached active frames receive priority, avoiding the
  more-sources-than-readers replay stall.
- Discovery avoids symlinks/reparse points and quarantine, has entry/depth caps,
  and performs source replacement/deletion cleanup. Snapshot rows/bytes are
  bounded. Logical deduplication, persistent suppression, publication locking,
  and archive revision cache invalidation were traced.

## Limits

This is source review, not evidence of successful Cargo gates or live external
classification. No claim of consensus with other reviewers is made. Concurrent
coordinator fixes to access classification/scope received the read-only cross
review below; combined Rust gates remain coordinator-owned.

Historical worker archives with the invalid wire shape are preserved, not
migrated. Producer records without authoritative repository identity stay
unscoped and cannot appear in scoped archive fallback. Raw ToolCall/AgentCall
payloads without a recorded outcome remain unchanged; their absence is not
converted into an invented success. The separately owned archive projector now
reads only its required display fields, allowing these partial observations to
remain queryable without adding a fabricated outcome.
The legacy Claude source watcher still rereads changed source JSONL and retains
its existing deduplication set; the new bounded zstd archive index must not be
described as a rewrite of that legacy source watcher. Daily profile disables
that archival producer path.

Owned-file `rustfmt --edition 2021` and `git diff --check` passed. New Rust tests
remain unexecuted by this reviewer until coordinator permits a Cargo slot.

## Offline non-Rust gates

Coordinator subsequently assigned the offline Python/Node/PowerShell gates.
No model, provider, installed native client, live hook, real client configuration
mutation, or runtime start was involved.

| Gate | Result | Retained log under `%TEMP%` |
| --- | --- | --- |
| AST parse of six `scripts/daily_*.py` files plus activation helper | 7 files passed | `cortex-phase34-sol-ast-gate.log` |
| `daily_memory_cross_model_probe.py --self-test` | Passed | `cortex-phase34-sol-cross-gate.log` |
| `daily_memory_live_harness.py --self-test` | Passed; synthetic journal retained | `cortex-phase34-sol-harness-gate.log` |
| `daily_memory_preservation_check.py --self-test` | Passed; synthetic SQLite/native-memory fixture retained | `cortex-phase34-sol-preservation-gate.log` |
| `daily_memory_installed_claude_probe.py --self-test` | Passed; no client/hook/model invoked | `cortex-phase34-sol-installed-gate.log` |
| `node --test packages/cortex-claude-plugin/scripts/install-mcp.test.js` | 2 passed, 0 failed/skipped | `cortex-phase34-sol-node-gate.log` |
| Windows PowerShell `bin/cortex-daily.ps1 -SelfTest` | Passed with process-scoped execution policy | `cortex-phase34-sol-powershell-process-policy-gate.log` |
| Activation helper offline guards and synthetic five-file plan | 7 scenarios passed; all seven hook events covered | `cortex-phase34-sol-activation-guards-v2-gate.log` |

The initial PowerShell command was blocked by host execution policy
(`UnauthorizedAccess`); it did not run the script. The successful rerun used
`powershell.exe -NoProfile -ExecutionPolicy Bypass -File bin/cortex-daily.ps1 -SelfTest`.
This sets policy only for that child process and does not write machine/user
execution policy. Initial failure is retained in
`cortex-phase34-sol-powershell-gate.log`.

The first synthetic activation plan accidentally used a prior pin different from
its synthetic binary directory. The helper correctly rejected it with
`Unexpected Cortex SessionStart command; review first`; no apply occurred.
Correcting the fixture to a supported bare prior hook command produced the
passing second run. Initial fixture error is retained in
`cortex-phase34-sol-activation-guards-gate.log`.

Activation scenarios cover hook pipe conversion to `//./pipe/...`, idempotence,
acceptance of the exact prior pinned command, rejection of unknown flags,
duplicate hooks and enabled duplicate plugin, plus a five-file synthetic plan.
The plan checks that native pipe spelling remains canonical in MCP environment,
unrelated MCP/TOML settings survive, and planning leaves fixture bytes unchanged.
The ad-hoc Python guard program ran from stdin with `-B`; no new source/test file
or bytecode cache was added.

## Cross review of coordinator archive repairs

Read-only inspection found no additional concrete P0–P2 issue in these repairs:

- `poll_and_publish` calls `require_projection_version("lane-hit-acl-v1")` before
  decoding or publishing a persisted snapshot. Version changes atomically remove
  old derived rows and reset cursors; permanent suppression and source bytes
  survive. Matching versions retain durable progress. Rebuild uses ordinary
  bounded polling. This avoids publishing older unlabeled projections after an
  upgrade.
- `envelope_to_hit` retains source `class_level` and `class_compartments` in the
  exact extras read by the authoritative ACL wedge. Exact `env.context.repo`
  survives; no cwd-derived repository fills an absent source identity.
- `scoped_archive_hits` still uses slug only to resolve the index alias, then
  requires exact repository identity before lexical matching. Distinct
  `foo-bar`/`foo_bar` sources cannot cross that filter.
- Archive-only call DTOs read display/input/output/duration fields without
  requiring an observed terminal outcome. The source payload is unchanged.
  The mapper-to-emitter-to-projection regression also checks classification
  propagation and preservation of missing repository identity.

Inspected regression coverage includes old unlabeled persisted rows, public,
level-restricted and compartment-restricted source events, a slug collision,
real issued API keys/principals, bounded upgrade/restart/suppression, and missing
call outcomes. These are coverage observations, not a claim those Rust tests
were executed by this reviewer.
