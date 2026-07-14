# Proposal: phase16a_opencode-adapter-via-envelope-producer

Source: `docs/analysis/opencode-adapter/`; supersedes `phase11w_opencode-adapter` (blocked).

## Why

The original `phase11w_opencode-adapter` was scoped before Phase A traits existed. It would have shipped the OpenCode adapter atop the current `cortex-adapter-claude-code` daemon's ad-hoc IPC + transport — re-introducing the same per-adapter scaffolding pattern that Phase 13b (`EnvelopeProducer` trait) was designed to eliminate.

This task re-scopes the OpenCode adapter as the FIRST consumer of the `EnvelopeProducer` trait. The TS plugin posts hook payloads to a uniform HTTP listener; the daemon-side handler is a thin shim that constructs `Envelope` and feeds the trait's emit stream.

## What Changes

- Re-implement the OpenCode adapter as `impl EnvelopeProducer for OpenCodeProducer`. The producer subscribes to OpenCode lifecycle events via the TS plugin and emits envelopes through the trait's stream.
- Reuse the HTTP listener from the original phase11w (default `127.0.0.1:17004`) but funnel into the trait, not into a per-adapter dispatcher.
- TS plugin (`packages/cortex-opencode-plugin/`) lands as designed in phase11w — the host-side surface is unchanged.
- `.opencode/{commands,agents}/` ports + `opencode.json` config land verbatim from phase11w §6 + §7.
- Pre-thinking injection via the path validated by the phase11w spike.

## Impact

- Affected specs: new `docs/specs/23-opencode-adapter.md` (was scoped in phase11w; refresh against the trait).
- Affected code: `crates/cortex-adapter-opencode/` (new crate replacing the per-adapter daemon), `packages/cortex-opencode-plugin/` (new TS package), `.opencode/`, `opencode.json`.
- Breaking change: NO. Claude Code path unchanged.
- User benefit: Cortex envelope capture inside OpenCode at parity with Claude Code; new adapter takes <1 day going forward.
