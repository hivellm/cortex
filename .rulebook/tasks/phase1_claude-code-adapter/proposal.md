# Proposal: phase1_claude-code-adapter

## Why

This is the capture surface that closes the loop: without an adapter, Cortex only sees bootstrap data. The Claude Code adapter is also the reference implementation for the multi-adapter layer (spec 17 copies it). It must capture every hook without breaking the session and must keep the pre-thinking + law-check synchronous paths under their hook budget.

## What Changes

- Hook shim scripts (`~/.claude/hooks/cortex-*.sh`) that pipe Claude Code's JSON stdin to the local daemon over UDS / named-pipe.
- `cortex-adapter-claude` daemon: IPC server, session/turn/tool-call correlation, in-process redactor, publisher, sync law-check + pre-thinking hooks, overflow WAL.
- Install / uninstall / status sub-commands on a shared `cortex-adapters` binary (patches `~/.claude/settings.json` idempotently).
- Envelope mapping per hook (spec 10 §Envelope mapping).
- Windows + Unix IPC parity.

## Impact

- **Affected specs:** [`docs/specs/10-claude-code-adapter.md`](../../../docs/specs/10-claude-code-adapter.md); unblocks 12, 14, 17.
- **Affected code:** new `cortex-adapters/common/` crate + `cortex-adapters/claude-code/` crate, hook shims under `cortex-adapters/claude-code/hooks/`, install framework under `cortex-adapters/common/install/`.
- **Breaking change:** NO — greenfield.
- **User benefit:** real Claude Code sessions become observable and pre-thinking context lands in the model's prompt.

## Source

`docs/specs/10-claude-code-adapter.md` · depends on spec 04 · PRD FR-10.
