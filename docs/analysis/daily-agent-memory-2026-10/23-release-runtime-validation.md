# Release runtime validation

Date: 2026-10-17. Task `phase34_daily-agent-memory-recovery`, section 1.12.
This follow-up supersedes the installed debug-runtime measurements in analysis22.
It preserves those observations and failures rather than treating a single model
success as sufficient runtime acceptance.

## F-045 - Development binaries exhausted the daily delivery budget

Evidence: `Cargo.toml:82`, `Cargo.toml:86`,
`crates/cortex-api/src/storage/api_keys.rs:332`,
`crates/cortex-pre-thinking/src/daily_memory.rs:117`, and
`crates/cortex-pre-thinking/src/pipeline.rs:217`.
Impact: healthy services could intermittently omit accepted decisions from
ordinary hooks. Confidence: high for the observed build-profile effect; the
measurements are local samples, not a universal latency guarantee.

After the debug installation and successful native-model test, an additional
check in the original repository failed to deliver both pinned memories. Direct
authenticated reads returned both records in 405-428 ms. Alternating original
and isolated repository hooks then failed 4 of 12 deliveries. Ten consecutive
SessionStart-only calls passed, while UserPromptSubmit also exercises optional
query work. Scope, ownership, repository ID and accepted record state were
verified; both scopes exhibited the failure.

The development profile uses `opt-level=0`. Every authenticated request still
performs Argon2 verification. Its blocking worker can continue after a client
timeout, competing with subsequent work. Release uses `opt-level=3`, thin LTO
and one codegen unit. The same source, persisted PHC parameters, authentication,
scope checks and deadlines were retained. No authentication cache, weaker hash,
larger timeout or relaxed assertion was introduced.

Retained local artifacts: `final-original-repository-hooks.json`,
`final-original-http-diagnosis.json`, `final-alternating-scope-hooks.json`, and
`final-memory-only-sequence.json`, under ignored `.cortex-state/daily-runtime/`.

## Exact build and installation

Four release executables were built from the same isolated selected-source
candidate that passed all Rust gates in analysis19:
`%TEMP%/cortex-phase34-commit-candidate-9y3vwevg`. Build completed in 286.758 seconds.
Log: `%TEMP%/cortex-phase34-final-release-build.log`. Build metadata from this
extracted candidate does not provide a Git SHA; the following artifact hashes
and candidate comparison establish identity instead.

| Executable | Installed SHA-256 |
| --- | --- |
| `cortex-api.exe` | `5594639f7c8524a1a3ec18cf86ee4df4d17e406bfd30f89595e7c8944650dbd9` |
| `cortex-adapter-claude.exe` | `e74c54a3494296d05b83d5ba071d91305e4c185fab48eb3fed4f09968361b22d` |
| `cortex-hook.exe` | `2bc9fa2aadfb9d8800794e0bd56d0ba1fe1c924089cb5f019c7f68a711463aa6` |
| `cortex-mcp-server.exe` | `9588d13981b41ab1049a215746c489a6470bac8039dad951d73bae6e7b6abe1a` |

Pins remain under `.cortex-state/daily-runtime/bin/`. Prior files/manifests were
backed up. Only launcher-owned API/adapter processes were stopped. Final rollout
verified config bytes unchanged and every installed hash equal to its source.
An installation helper initially waited on inherited Windows pipe handles after
PowerShell had exited; its owned process was stopped and helper output redirected
to files. Repeating the guarded rollout completed normally. This was separate
from hook delivery and required no production Rust change.

The launcher now defaults to `target/release`; operations examples explicitly
build/select release. PowerShell 5.1 and 7 parse/ownership/quoting/port-conflict
self-tests pass. Existing client-owned MCP processes remain untouched and need
reconnection to load their newly configured pin. Artifact:
`final-release-installation.json`; `binary-manifest.json` records current pins.

## Delivery and observed model use

Forty consecutive hook calls alternated the original project and authorized
isolated repository, with no delay between calls. All returned both expected
decision IDs, exit 0 and no unavailable-memory notice.

| Hook | Samples | Minimum | Median | p95 | Maximum |
| --- | ---: | ---: | ---: | ---: | ---: |
| SessionStart | 20 | 32 ms | 35 ms | 44 ms | 143 ms |
| UserPromptSubmit | 20 | 47 ms | 51 ms | 82 ms | 95 ms |

These are hook process wall times. The HTTP/pipeline/shim limits remain
550/600/1,500 ms. Artifact: `final-release-sequence.json`.

Authorized isolated preflight again confirmed authentication, empty unrelated
scope and absent unrelated active-work content. HTTP read took 120 ms; hooks
took 43/55 ms. Both delivered the same authorized 1,234-byte context as analysis22.
Artifact: `authorized-final-preflight-c6d12cd02ee545cabb0346a7533f867d.json`.

Claude/Fable then consumed that context through actual installed hooks in a fresh
session. It returned both exact IDs/versions and required decision details,
without model tools, MCP tools or expected IDs in its prompt. Duration was
5.752 seconds; reported cost USD 0.01109325. User hook settings stayed unchanged.
Artifact: `%TEMP%/cortex-installed-claude-8os7jwb1/report.json`. This proves recall
of these decisions; it does not prove compliance with all future tasks.

The three authorized original Jev retries also passed after release restart.
`classify=false` remained `not_requested`; both true attempts for each candidate
hit persisted advice cache. No new inference was needed. All original receipts,
records and revisions stayed unchanged. Artifact:
`authorized-final-jev-40b016c0e2fe41cabad4a868c3010435.json`.

## Installed resource observation and preservation

The final installed pair was observed for 60.005 seconds without collector HTTP
requests during the interval. Other clients generated 23 hook frames, so this is
a light-activity observation, not idle measurement. No unrelated process was
stopped to improve the numbers.

| Process | End working set | Sampled peak | CPU seconds delta | CPU, one core |
| --- | ---: | ---: | ---: | ---: |
| API | 19.004 MiB | 19.004 MiB | 0.078125 | 0.1302% |
| Adapter | 13.027 MiB | 13.027 MiB | 0.015625 | 0.0260% |

Combined working set was 32.031 MiB. This excludes client-owned MCPs, Docker
Desktop and unrelated services. API read/write byte deltas were 776,244/0;
adapter deltas were 41,299/4,081. Counters include file/network/device I/O.
Artifact: `final-release-resource-observation.json`.

All 535 native-memory files still match the original baseline: zero changed,
added or missing. SQLite WAL-aware read-only integrity check returned `ok`,
with eight records and `retention.memory_consolidate` disabled. Artifacts:
`preservation-after-release-models.json` and `database-after-release-models.json`.
The latest Docker inventory retains all six volumes and 12 stopped Cortex
containers. Automatic OS startup was not added; use the pinned launcher commands
in the operations guide after reboot.

After these observations, the verified build-profile lesson was saved locally as
`daily-runtime-release-build`, version 1, event `01M5552D1SGMMEVNCWRJQ3M2Z6`,
Cortex revision 5. A matching local query returned it. Classification remained
`not_requested`; this additional lesson was not sent to any external model.
The database consequently contains nine records. Artifact:
`local-release-lesson-capture.json`. Rulebook knowledge also records F-045.

A newly started pinned release MCP process also passed the actual stdio
initialize/list/read exchange against the authenticated installed API: 12 tools,
both decisions and the new release lesson returned, 155 ms process wall time.
Artifact: `final-release-mcp-stdio.json`. This verifies transport and retrieval,
not a model's consumption; existing client sessions still need reconnection.
