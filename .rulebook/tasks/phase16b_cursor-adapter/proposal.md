# Proposal: phase16b_cursor-adapter

Source: `docs/analysis/rework/minmax2.7/01-findings.md` F-010 (HIGH); `docs/specs/17-additional-adapters.md`.

## Why

Cursor is the second largest agent host the user runs. Today Cortex captures nothing from Cursor sessions. With Phase 13b's `EnvelopeProducer` trait + Phase 16a's adapter pattern (HTTP listener + TS plugin), shipping the Cursor adapter is a 1-2 day port.

## What Changes

- New crate `crates/cortex-adapter-cursor/` — `impl EnvelopeProducer for CursorProducer`. Shares the HTTP listener with the OpenCode adapter (different bind port).
- Cursor doesn't have the same plugin API as OpenCode; integration is via the Cursor `aiagent.json` rules + a wrapper script that intercepts model invocations and POSTs hooks.
- `.cursor/{rules,commands}/` ports of the canonical agent + command files.
- Add `"cursor"` to envelope `tool` enum.

## Impact

- Affected specs: `docs/specs/17-additional-adapters.md` § Cursor (graduate from "not started" to "v1").
- Affected code: `crates/cortex-adapter-cursor/` (new), `.cursor/{rules,commands}/`, `crates/cortex-core/schemas/envelope.schema.json`.
- Breaking change: NO. Additive.
- User benefit: Cursor sessions feed Cortex; institutional memory survives the host switch.
