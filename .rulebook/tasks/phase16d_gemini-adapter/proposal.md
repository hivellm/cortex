# Proposal: phase16d_gemini-adapter

Source: `docs/analysis/rework/minmax2.7/01-findings.md` F-010; `docs/specs/17-additional-adapters.md`.

## Why

Google Gemini CLI is the fourth listed agent host. With the trait + wrapper pattern proven in 16a/b/c, Gemini is a near-mechanical port.

## What Changes

- New crate `crates/cortex-adapter-gemini/` — `impl EnvelopeProducer for GeminiProducer`.
- Wrapper script `cortex-gemini` that wraps the `gemini` CLI.
- Add `"gemini"` to envelope `tool` enum.

## Impact

- Affected specs: `docs/specs/17-additional-adapters.md` § Gemini.
- Affected code: `crates/cortex-adapter-gemini/` (new), `scripts/cortex-gemini.{sh,ps1}` (new), `crates/cortex-core/schemas/envelope.schema.json`.
- Breaking change: NO.
- User benefit: Gemini sessions feed Cortex.
