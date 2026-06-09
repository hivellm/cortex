# 24. Ingestion-time redaction policy

**Status**: proposed
**Date**: 2026-06-09
**Related Tasks**: phase15e_ingestion-redaction-policy-adr-017

## Context

Cortex captures raw tool-call payloads, prompt text, and environment snippets from Claude Code sessions. These envelopes flow into Synap, the embedder (vector index), and the archive (disk). Secrets embedded in session activity — AWS keys, GitHub PATs, Anthropic API keys, Bearer tokens, passwords — risk being persisted in plain text across multiple storage backends where they become very hard to purge retroactively.

## Decision

Apply redaction at ingestion time, in the adapter, before any envelope is published to Synap or the archive. Each secret match is replaced with `&lt;REDACTED:&lt;kind&gt;:&lt;hash8&gt;&gt;` where hash8 = first 8 hex chars of SHA256(matched_text). This preserves envelope shape and duplicate-detection semantics while keeping the actual secret out of every downstream store. Redaction adds &lt;1 ms per envelope (regex scan over payload bytes).

## Alternatives Considered

- Redact at the embedder — too late: secrets already in Synap stream and archive before the embedder sees them.
- Redact at the API layer on ingest — adds a round-trip and couples the ingestion service to secret patterns; adapter is the right choke point since it owns the envelope before publish.
- No redaction — unacceptable: a single session touching a .env file or an API call with an Authorization header persists the secret in perpetuity across vector + fulltext + graph indexes.
- Redact at the archive writer — still too late for the Synap stream and the real-time indexes.

## Consequences

Secrets never reach the embedder, fulltext index, graph, or archive. Duplicate-detection still works (placeholder is deterministic per secret value). A secret that recurs in multiple envelopes maps to the same placeholder — correlation is preserved. The operator cannot recover the original secret from Cortex (by design). Pattern coverage is conservative (false-negative risk on novel secret shapes) — the doctor command provides ongoing coverage monitoring. Redaction runs synchronously in the adapter hook path; the &lt;1 ms cost is within the 300 ms PreToolUse budget.
