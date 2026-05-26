# Proposal: phase1_embedder

## Why

Semantic retrieval is non-negotiable for the "before-change context" user story (US-01). This task delivers the worker that chunks enriched events deterministically (symbol-level for code via Tree-sitter, section-level for docs, fixed-window fallback) and writes vectors to Vectorizer with stable chunk identity so re-runs are idempotent.

## What Changes

- `Chunker` trait + `CodeChunker` (Tree-sitter for Rust, TS, JS, Python, Go, Java, C, C++, Markdown, JSON, YAML, TOML), `DocChunker`, `FallbackChunker`.
- Deterministic `chunk_id` (`ulid_from_hash(event_id || ordinal || chunk_content_hash)`).
- `VectorizerClient` HTTP wrapper with retry, backoff, batched upsert (64 chunks).
- Per-`kind` collection routing + `ensure_collection` bootstrap at worker startup.
- Summary-substitution rule for payloads >4 KB.
- Worker binary consuming `cortex.events.enriched`, publishing `cortex.events.embedded`.

## Impact

- **Affected specs:** [`docs/specs/06-embedder.md`](../../../docs/specs/06-embedder.md); unblocks 09 + 11.
- **Affected code:** new `cortex-embedder/` crate, worker binary `cortex-embedder-worker`, Tree-sitter grammar deps under `cortex-embedder/Cargo.toml`.
- **Breaking change:** NO — greenfield.
- **User benefit:** semantic lane of the hybrid query API returns relevant code/doc chunks within P95 < 500 ms cold.

## Source

`docs/specs/06-embedder.md` · depends on specs 01 + 02 · PRD FR-6.
