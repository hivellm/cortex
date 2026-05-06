# Proposal: phase16c_codex-adapter

Source: `docs/analysis/rework/minmax2.7/01-findings.md` F-010; `docs/specs/17-additional-adapters.md`.

## Why

OpenAI Codex / GitHub Copilot CLI is a third agent host the user evaluates. Codex's native plugin surface is thinner than OpenCode's; integration is via stdout/stderr scraping of the CLI plus the same HTTP listener pattern from Phase 16a/b.

## What Changes

- New crate `crates/cortex-adapter-codex/` — `impl EnvelopeProducer for CodexProducer`.
- Wrapper script `cortex-codex` that wraps `codex` CLI and pipes stdout through a hook formatter.
- Add `"codex"` to envelope `tool` enum.

## Impact

- Affected specs: `docs/specs/17-additional-adapters.md` § Codex.
- Affected code: `crates/cortex-adapter-codex/` (new), `scripts/cortex-codex.{sh,ps1}` (new), `crates/cortex-core/schemas/envelope.schema.json`.
- Breaking change: NO.
- User benefit: Codex sessions feed Cortex.
