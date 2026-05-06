# Proposal: phase15e_ingestion-redaction-policy-adr-017

Source: `docs/analysis/rework/opus5.7/02-blind-spots.md` §9 (privacy).

## Why

Cortex ingests Claude Code session data verbatim — including secrets accidentally pasted by the user, environment variable values, error messages with auth tokens, and full command-line invocations. There is no ingestion-time redaction. PII enforcement (`pii_enforce` sweep) runs over the storage layer, but secrets that hit the embedder before `pii_enforce` runs are already vectorised and indexed.

## What Changes

- New ADR-017 — "Ingestion-time redaction policy".
- New module `crates/cortex-core/src/redaction.rs` with `redact(envelope) -> Envelope` applied at the adapter boundary BEFORE the envelope crosses the network.
- Redaction patterns: AWS keys, GitHub PATs, Anthropic keys, `Bearer <token>`, `Authorization: ...`, `password=...`, generic high-entropy strings (≥40 chars, base64-ish).
- Redacted fields are replaced with `<REDACTED:kind:hash8>` so duplicate detection still works.
- Doctor `cortex-ops doctor redaction-coverage` samples 100 random envelopes from Synap and reports any unredacted candidates.

## Impact

- Affected specs: `docs/specs/04-event-schema.md` § Redaction; new ADR-017.
- Affected code: `crates/cortex-core/src/redaction.rs` (new), `crates/cortex-adapter-claude-code/src/dispatcher.rs` (call `redact()` before send), `crates/cortex-cli/src/bin/cortex-ops.rs`.
- Breaking change: NO at the wire format; payloads gain `<REDACTED:...>` placeholders.
- User benefit: secrets no longer leak into the embedding / index / archive layer.
