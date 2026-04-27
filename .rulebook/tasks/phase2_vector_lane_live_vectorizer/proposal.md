# Proposal: phase2_vector_lane_live_vectorizer

## Why

The vector lane is empty in production. `cortex-api/src/main.rs:40` reads `Arc::new(MemoryVectorLane::new())` — a defaulted in-memory map with no embeddings ever loaded. `debug.lanes.vector_ms = 0` on every query I probed today, and the snippets surfaced under `source = "vector"` are actually keyword-lane hits with positional RRF scores (`1/(60+rank)`).

The infrastructure to fix this exists:

- `cortex-embedder-worker` daemon is already running (PID 48984 verified on 2026-04-27).
- `cortex-embedder` crate already has `vectorizer_client.rs` and `routing.rs` modules.
- The user's auto-memory record `project_cortex_sdks.md` confirms the official Vectorizer Rust SDK exists and is used in production by other Hive components.

What's missing is a `VectorLane` impl in `cortex-api` that calls the Vectorizer SDK at query time (KNN against the same collections the embedder-worker writes to), plus the orchestrator-side wiring to swap `MemoryVectorLane` for the live one.

## What Changes

- New module `cortex-embedder::vector_lane` (or expand `vectorizer_client.rs`) implementing `cortex_api::VectorLane`:
  - Maps `VectorRequest { collection, query_vector, k, filter }` to the Vectorizer SDK's KNN call.
  - When the orchestrator passes a `query` text instead of a precomputed vector, the lane embeds it on-the-fly via the embedder service or via an in-process embedding cache.
- `cortex-api/src/main.rs` reads `VECTORIZER_URL` (or `CORTEX_VECTORIZER_URL`) + optional API key, probes the server, and binds the live lane in place of `MemoryVectorLane`. Fail-open to the memory lane on probe failure.
- The orchestrator's `VectorRequest` already carries `collection`; ensure the boot wiring populates it with the canonical collection alias from `cortex-storage::collections::COLLECTIONS`.
- Per the user's standing rule (auto-memory `feedback_dont_blame_hive_services.md`), the integration uses the official `vectorizer_sdk` crate without re-implementing client logic. Drift bugs that surface during integration land as anti-patterns on the SDK side, not as workarounds in cortex.

## Impact

- Affected specs: spec-06 (embedder), spec-11 (lane wiring).
- Affected code:
  - `crates/cortex-embedder/src/vectorizer_client.rs` (or new sibling `vector_lane.rs`)
  - `crates/cortex-api/src/main.rs` (boot wiring)
  - `crates/cortex-api/src/lanes.rs` (no trait change; only a new `impl VectorLane`)
  - tests against a wiremock or live Vectorizer
- Breaking change: NO (additive)
- User benefit: semantic recall — pre-thinking surfaces past turns that mean the same thing as the prompt, not just lexical matches. Pairs with the live keyword lane via RRF for the dual-lane retrieval the orchestrator was designed for.

## Source

2026-04-27 audit; `debug.lanes.vector_ms = 0` confirmed across 11 probes.
