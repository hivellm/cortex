# Authorized live validation

Date: 2026-10-17. Task `phase34_daily-agent-memory-recovery`, section 1.12.

This page records the debug-build installation and its authorized checks.
[Analysis23](23-release-runtime-validation.md) records a later sequential-delivery
failure and the validated release installation now active on this host.

## Authorization and scope

The operator explicitly authorized the three previously named summaries for
TypeSafe/Jev: ADR-039 daily/SQLite architecture, useful recall as project priority,
and prepared delivery versus model consumption. Two architecture/priority
summaries were also authorized for the native Claude recall test. The separate
authorization for selected Rust source reviews covers Claude, Kimi and Grok.
Native agent memories, `.env` and credentials are excluded from those payloads.
The operator also authorized preserving `.agents/` and `.codex/` outside commits.

Earlier approval rejections in analyses13/15/19 remain historical evidence; they
are no longer open permission requests. Authorization does not expand to the
subsequently captured Windows pipe lesson or unrelated Rulebook task content.

## Delayed classification exposed a retry defect

The first live attempt replayed the original three capture requests with their
original retry keys, first with `classify=false`, then with `classify=true`.
Every receipt retained its original ID, event, version and content hash. All
six stored records and Cortex revision 4 remained unchanged. However, all
classification attempts returned `reason=stale`, and no provider attempt was
reserved or sent. Artifact: ignored runtime
`authorized-jev-real-candidates.json`.

The original receipts refer to revisions 1, 2 and 3. A later independent lesson
had advanced the scope to revision 4 without changing any of those candidates.
The advice loader incorrectly required an old retry receipt's scope revision to
equal the current scope revision before considering the unchanged candidate.
This prevented later consent from classifying an otherwise eligible capture.

The correction reads the current scope revision and exact candidate in one
SQLite snapshot. Candidate version, event, content hash, lifecycle and stale
flag must still match the original receipt. Only the private advice receipt
copy receives the snapshot revision; the capture response remains immutable.
Cache hits and remote results still reject changes during evaluation, and
superseded candidates remain ineligible. No neighboring memory is sent.

Regressions cover later consent after an unrelated local capture, unchanged
capture receipts, candidate-only disclosure, refreshed cache keys after scope
changes, superseded retries, and an in-flight result invalidated by a concurrent
capture. Independent integration review found no additional P0-P2 defect in
this correction. The exact candidate subsequently passed all quality gates:
3,955 tests, zero failures, nine ignored; see analysis19.

## Installed hook preflight

Before replacing the older installed binaries, a native preflight returned an
unavailable-memory notice and an active-work block containing older Rulebook
tasks. A direct authenticated read then returned both pinned decisions in
562 ms while workspace compilation was active. These observations do not prove
idle latency or identify a single slow stage. The 550 ms memory HTTP bound and
600 ms total pre-thinking bound remain unchanged.

No Claude inference used that preflight output. The unrelated active-work
content exceeds the explicitly authorized two-summary payload. Any subsequent
external recall test must inspect its complete hook context and stay within
that authorization; changing a receipt stage cannot establish model use.

## Final installed verification

Four executables built from the validated candidate replaced the pinned API,
adapter, hook and MCP binaries on 2026-10-17. Every installed SHA-256 matched its
build output. Previous binaries and their manifest were backed up before the
launcher stopped only its recorded API/adapter processes. Private runtime config
remained byte-for-byte unchanged. The restarted API and adapter run in daily
mode; existing client-owned MCP processes were not terminated. Runtime artifacts
`final-installation.json` and `binary-manifest.json` retain exact paths and hashes.

The two authorized summaries were copied exactly into a separate repository
scope, with original event references and body hashes retained in
`authorized-recall-preparation.json`. This prevents unrelated Rulebook tasks or
the Windows pipe lesson from entering the demonstration. Original Cortex records
remain unchanged. These copies are test provenance, not evidence of automatic
migration or recovery of arbitrary historical memories.

Two failed preflights are retained. The first direct HTTP probe sent more than
32 distinct query terms and correctly received 400; direct reads now use a
bounded query, while the native hook still receives the exact original prompt
and applies the production query projection. The original temporary repository
was owned by `CodexSandboxOffline` and unreadable by the operator-owned daemon.
A new operator-owned temporary repository with the same explicit ID fixed that
fixture error without broadening ACLs or changing production code.

The successful preflight artifact is
`authorized-final-preflight-ecebc91a3de847d9b8f8770030167c1c.json`:

- Authenticated direct read returned exactly the two copied decisions in 412 ms.
- Missing authentication returned 401; an unrelated repository returned no memory.
- Filtered active tasks were empty. Complete hook contexts contained only the
  two authorized summaries, their provenance and the advisory-priority heading.
- SessionStart and UserPromptSubmit returned identical 1,234-byte context with
  SHA-256 `f85bffb66b8a9cc243794be7358ba9f310b1f2c915514825fd120128dab6ebf3`.
- Hook process wall times were 422 and 730 ms respectively. These include process,
  configuration and dispatch work and are not a claim that total hook wall time
  always meets the pipeline's 600 ms bound. Production pipeline/HTTP/shim bounds
  remain 600/550/1,500 ms; the serial regression suite passed unchanged.

### Native Claude consumed the saved decisions

The installed Claude CLI ran `claude-fable-5-1` in a fresh session using actual
user hooks. It advertised no model tools and no MCP servers, made no tool calls,
and received no expected IDs in its prompt. User hook settings remained unchanged.
The structured stream recorded the same SessionStart context hash as preflight;
the dispatcher log independently recorded SessionStart and UserPromptSubmit.

The model returned `adr039-daily-runtime` v1 and `daily-value-priority` v1 with
their exact operational requirements: daily/SQLite architecture, optional bulk
services, advisory-only Jev, useful recall as priority, old backlog only when
needed, and selected checklist order. Every host-side assertion passed. Duration
was 5.853 seconds; the CLI reported USD 0.04848. Evidence:
`%TEMP%/cortex-installed-claude-na30hqj1/report.json` plus its native stream.

This proves recall and model use of these two saved summaries through the
installed path. It does not prove universal compliance in future coding work.
The distinct corrected-action-after-restart scenario remains in analysis11.

### Jev classified the three authorized original candidates

For each original Cortex capture, the test replayed its original retry key with
`classify=false`, then `true`, then `true` again. False returned `not_requested`.
The first true request produced one available judgment; the second hit the cache.
All original event IDs, timestamps, hashes, versions and scope revisions matched
their original receipts. All stored records and scope revisions stayed unchanged.

| Memory | Suggested kind | Confidence | Provider latency | Input/output tokens |
| --- | --- | ---: | ---: | --- |
| `adr039-daily-runtime` | decision | 0.92 | 385 ms | 653 / 92 |
| `daily-value-priority` | decision | 0.27 | 281 ms | 655 / 92 |
| `prepared-is-not-consumed` | lesson | 0.97 | 264 ms | 663 / 92 |

All used pinned `jev-1.13.0`, candidate-only payloads and repository scope.
The low-confidence priority judgment remains advice; it grants no authority.
Cache responses reported no new usage. The three uncached calls reported
1,971 input and 276 output tokens; no unreported billed cost is inferred.
Artifact: `authorized-final-jev-e851096cbb8e49579f75ddc0f680c5e8.json`.

## Installed resource observation and preservation

A 60.008-second Windows counter sample after the model checks measured the
installed processes, with no collector HTTP requests during the interval.
Other clients produced six hook frames, so this was a light-activity observation,
not an idle benchmark. Existing unrelated workloads were left running.

| Process | End working set | Sampled peak | CPU seconds delta | CPU, one core |
| --- | ---: | ---: | ---: | ---: |
| API | 77.238 MiB | 77.477 MiB | 0.015625 | 0.0260% |
| Adapter | 20.129 MiB | 20.129 MiB | 0.015625 | 0.0260% |

Combined final working set was 97.367 MiB. This excludes client-owned MCPs,
Docker Desktop and unrelated services. API read/write byte deltas were
776,244/0; adapter deltas were 11,151/1,185. These counters include file, network
and device I/O, not disk-only traffic. Raw samples and activity counts are in
`final-installed-resource-observation.json`; earlier isolated idle measurements
remain in analysis10 and are not substituted for these installed values.

All 12 Cortex containers remain stopped and all six named volumes remain present.
After the final model run, all 535 native-memory hashes still match the original
baseline, with zero changed, added or missing files. SQLite integrity was checked
with a live WAL-aware read-only connection; immutable inspection correctly
refused the nonempty WAL. Automatic native-memory consolidation remains disabled.
