# Proposal: phase8f_synthetic_e2e_canary

## Why

phase8a–8e watch real traffic. But during quiet hours (no claude-code
activity), the pipeline could be silently broken for hours and no
divergence would fire because no events are flowing. The synthetic
canary closes that gap: every N seconds, fire a known fake
PostToolUse frame through the real hook path and assert it lands in
the archive within a deadline. If it doesn't, alert the same way
phase8e does.

This is also the regression test the user begged for: the JSON
truncation bug had been latent in the codebase for months — it only
surfaced when the model started pretty-printing PostToolUse stdin.
A canary that mimics that behavior would have caught the regression
the moment commit `5ad2973` (which changed payload field names) or
any later refactor broke the path.

The canary is also the most direct way to gate CI: the existing
unit/integration tests run against in-process fakes; the canary
boots the *whole* stack (adapter daemon, ingestion HTTP, archive
writer, lane refresh) and exercises it end-to-end.

## What Changes

1. NEW `cortex-doctor canary` subcommand that:
   - Connects to the named pipe `\\.\pipe\cortex-adapter-claude`
     (Windows) or `~/.cortex/adapter-claude.sock` (Unix).
   - Sends a synthetic PostToolUse frame with a unique marker
     `tool_name: "Canary-<ulid>"`, including:
     - **Pretty-printed** JSON (newlines between fields) — the
       exact failure mode that bit on 2026-04-28.
     - A multi-line `tool_response.stdout` (escaped `\n` as `\n` per
       JSON spec).
     - A realistic shape matching what claude-code actually sends.
   - Polls the cortex-ingestion archive (or `/v1/dashboard/timeline/recent`)
     until the canary envelope appears, with a 10-second deadline.
   - Returns `0` on success, `2` on timeout, `1` on transport errors.

2. NEW `cortex-api` background task `canary_runner` (default disabled,
   gated by `[canary] enabled = true` in cortex.toml) that runs the
   canary every `interval_secs` (default 300 = 5 min), records the
   result in `~/.cortex/canary-history.jsonl`, and emits a
   `law_violation` envelope on failure (same path as phase8e).

3. NEW canary frames for additional hooks: UserPromptSubmit, Stop,
   PreToolUse, SubagentStop. Each shaped after a real claude-code
   payload sample committed under `crates/cortex-doctor/fixtures/`.

4. `scripts/canary.bat` and `.sh` thin wrappers around
   `cortex-doctor canary` for ad-hoc invocation.

## Impact

- Affected specs: NEW `specs/canary/spec.md`.
- Affected code:
  - `crates/cortex-doctor/` extended with `canary` subcommand
    + `fixtures/` directory of golden frames
  - NEW `crates/cortex-api/src/health/canary.rs` (background runner)
  - `~/.cortex/cortex.toml` NEW `[canary]` section
  - NEW `scripts/canary.bat`
- Depends on: phase8e (uses the same alert envelope path).
- Breaking change: NO (additive).
- User benefit: a regression like the 2026-04-28 JSON truncation
  bug is detected in 10 seconds (or 5 minutes if running on the
  default schedule) instead of hours. CI gains an end-to-end smoke
  test that exercises the actual pipe + ingestion + archive path,
  not a mock.
