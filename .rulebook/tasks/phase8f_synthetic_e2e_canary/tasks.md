## 1. Golden frame fixtures
- [ ] 1.1 Capture a real claude-code PostToolUse stdin (use the hook log forensic mechanism from phase8a) and store sanitized version in `crates/cortex-doctor/fixtures/post_tool_use_bash.json`
- [ ] 1.2 Capture UserPromptSubmit, Stop, PreToolUse, SubagentStop fixtures
- [ ] 1.3 Each fixture MUST include the pretty-printed format (newlines between fields) AND multi-line escaped strings — the regression vectors from 2026-04-28
- [ ] 1.4 Each fixture has a sibling `*_expected.json` describing the canonical envelope it should produce

## 2. cortex-doctor canary subcommand
- [ ] 2.1 Add `canary` subcommand to `cortex-doctor` (CLI parser)
- [ ] 2.2 Implement `connect_pipe()` that connects to the named pipe / unix socket with 3s timeout (matches the new hook timeout)
- [ ] 2.3 Implement `send_canary_frame(hook_kind: HookKind) -> Result<MarkerId>` that loads the fixture, replaces the tool_name with `Canary-<ulid>`, writes the frame, reads the response
- [ ] 2.4 Implement `wait_for_archive(marker_id, deadline)` that polls cortex-ingestion archive (or `/v1/dashboard/timeline/recent?kind=tool_call`) until the marker is found
- [ ] 2.5 CLI flags: `--hook=PostToolUse|UserPromptSubmit|all`, `--deadline-secs=10`, `--json` for machine output
- [ ] 2.6 Exit codes: 0 success, 2 timeout, 1 transport error
- [ ] 2.7 Unit tests: connecting to a fake pipe + verifying frame round-trip
- [ ] 2.8 Integration test gated by `CORTEX_E2E=1` env var so CI without a live daemon does not execute it (set by the canary CI job in phase8h)

## 3. cortex-api background canary runner
- [ ] 3.1 NEW `crates/cortex-api/src/health/canary.rs`
- [ ] 3.2 Spawn a `canary_runner` task at boot when `[canary].enabled = true`
- [ ] 3.3 Default config: `interval_secs = 300`, `deadline_secs = 10`, `hooks = ["PostToolUse"]`, `record_path = "~/.cortex/canary-history.jsonl"`
- [ ] 3.4 On each tick: invoke the same canary logic in-process (use cortex-doctor as a library dep) and append result to history file
- [ ] 3.5 On failure: emit law_violation envelope via the phase8e alert path (`law_id: "canary-<hook>"`, severity: critical)

## 4. CLI scripts
- [ ] 4.1 NEW `scripts/canary.bat` thin wrapper around `cortex-doctor canary --hook=PostToolUse`
- [ ] 4.2 Companion `scripts/canary.sh`
- [ ] 4.3 Document in `docs/runbooks/canary.md` how to run on demand and how to interpret history file

## 5. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 5.1 Update `docs/architecture.md` (Health monitoring section) + new `docs/runbooks/canary.md` + CHANGELOG entries on cortex-api + cortex-doctor
- [ ] 5.2 Tests: each fixture's expected envelope shape is asserted by an integration test that drives the dispatcher with the fixture and compares output; canary subcommand integration test boots a fake pipe and asserts the round-trip + archive landing
- [ ] 5.3 Run `cargo test -p cortex-doctor -p cortex-api` and confirm all pass
