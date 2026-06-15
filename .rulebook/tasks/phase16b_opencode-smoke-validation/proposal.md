# Proposal: phase16b_opencode-smoke-validation

## Why
phase16a shipped the `cortex-adapter-opencode` Rust crate and the TS plugin but §4.4 / §6.1–§6.3 of that task require a live operator-run OpenCode session to verify. Those items were blocked during phase16a because the implementation environment cannot launch the OpenCode TUI. This follow-up task carries them forward so they can be executed when a live session is available, and also closes phase11w_opencode-adapter once parity is confirmed.

## What Changes
- Operator runs a live OpenCode session (prompt + 2 tool calls + 1 subagent) with the cortex-opencode-plugin active.
- Verify ≥4 envelopes with `tool: "opencode"` land in Synap `cortex.events.raw`.
- Verify the pre-thinking bundle reached the model (audit envelope).
- Spot-check 3 commands + 3 agents inside the OpenCode session.
- Archive task `phase11w_opencode-adapter` once parity is confirmed.

## Impact
- Affected specs: `docs/specs/20-opencode-adapter.md`
- Affected code: none (validation only)
- Breaking change: NO
- User benefit: Confirms the OpenCode adapter is production-ready end-to-end.
