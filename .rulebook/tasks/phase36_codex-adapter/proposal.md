# Proposal: phase36_codex-adapter

## Why
Cortex has adapters for Claude Code and OpenCode, but Codex only has a thin plugin that pipes its hooks into the Claude path. It is unverified end to end, and every event is labelled `claude-code` because the hook frame carries no client identity. That bug also mislabels OpenCode. Codex CLI 0.162 has Claude-compatible hooks (SessionStart, UserPromptSubmit, PreToolUse, PostToolUse, Stop and SubagentStop, with `additionalContext` injection) and a plugin system that substitutes `${PLUGIN_ROOT}` and runs `commandWindows` under `cmd /C`. A first-class Codex adapter lets Codex sessions receive Cortex memory and be captured under their own identity.

## What Changes
Generalize the shared hook path so each frame names its client: `claude-code`, `codex` or `opencode`. Add a Codex payload mapping, fix and harden the Codex plugin (Windows launcher, events, version bump), and add spec 46, tests and docs. Then run an operator-gated live acceptance in Codex. The shared `cortex-adapter-claude` daemon and `cortex-hook` are reused, with no new daemon. The daily profile keeps capture disabled, so in daily mode the live value is memory injection on SessionStart and UserPromptSubmit.

## Impact
- Affected specs: none
- Affected code: none
- Breaking change: NO
- User benefit: none
