# Proposal: phase1_classifier_worker

## Why

Cortex's spec 09 bootstrap CLI publishes synthetic envelopes onto
`cortex.events.bootstrap`, and `cortex-ingestion` writes live envelopes
onto `cortex.events.raw`. Specs 06/07/08 (embedder/graph/fulltext)
all consume from `cortex.events.enriched` — but **no worker bridges
the two halves**. Spec 05 (classifier) shipped the library
(`StaticClassifier` / `CachedClassifier` / `BudgetedClassifier` /
`HaikuCliClassifier`) but explicitly deferred the worker binary
("Worker binary wiring + Synap-backed cache arrive with the ingestion
consumer in a follow-up pass").

Consequence: bootstrap and live capture both publish events that
nothing consumes. Vectorizer / Nexus / Meilisearch stay empty, and
`/v1/query` falls back to the archive-loader stopgap rather than
the canonical pipeline. Until this gap is closed, "Cortex maps and
indexes the project" cannot be demonstrated end-to-end.

## What Changes

A new `cortex-classifier-worker` binary inside the existing
`cortex-classifier` crate. It:

- Connects to Synap and consumes from both `cortex.events.raw` and
  `cortex.events.bootstrap` in parallel.
- For each consumed envelope, builds an `EnrichmentInput` (mapping
  the bootstrap-event shape and the canonical envelope shape both
  onto a common `Kind`).
- Runs the input through a configurable classifier stack:
  - default: `StaticClassifier` behind cache + budget tracker
    (offline, deterministic, zero LLM cost),
  - opt-in via `CORTEX_CLASSIFIER_MODE=cli`: `HaikuCliClassifier`.
- Composes an `EnrichedEvent` (the same struct
  `cortex-embedder::EnrichedEvent` that graph and fulltext consume)
  and publishes to `cortex.events.enriched`.
- Acks the source message after a successful publish.
- Tracks already-classified `event_id`s in-memory for at-least-once
  delivery — same pattern as `cortex-embedder` and `cortex-graph`.

Configuration via `CORTEX_CLASSIFIER_*` env vars (synap URL, mode,
batch size, parallelism, daily budget cents). Documented in the
crate README.

Integration tests cover: bootstrap envelope -> enriched envelope,
canonical envelope -> enriched envelope, dedup on replay, static
fallback when budget halts.

## Impact

- Affected specs: 05 (classifier worker wiring graduates from
  "follow-up pass" to implemented).
- Affected code:
  - `crates/cortex-classifier/Cargo.toml` (add `[[bin]]` +
    runtime deps).
  - `crates/cortex-classifier/src/worker.rs` (new — Synap
    consumer/publisher abstractions + `Worker` loop).
  - `crates/cortex-classifier/src/main.rs` (new — binary entrypoint,
    env-driven config, ctrl-c handler).
  - `crates/cortex-classifier/src/config.rs` (new — env parsing).
  - `crates/cortex-classifier/README.md` (new — docs).
  - Workspace-level `cortex.toml` so `cortex-bootstrap .` can run
    against the Cortex repo itself with sensible exclude/decision/
    memory rules.
- Breaking change: NO. New binary; existing library API unchanged.
- User benefit: closes the loop so bootstrap + live capture actually
  reach Vectorizer/Nexus/Meilisearch, unblocking the whole
  "Cortex indexes the project" capability.

Source: docs/specs/05-classifier.md (worker binary wiring deferral).
