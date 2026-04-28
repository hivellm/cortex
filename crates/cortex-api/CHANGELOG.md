# Changelog

All notable changes to `cortex-api` are documented here. The format
follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and
the project uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- Sonnet-backed cross-event session analyzer (`src/analyzer.rs`)
  produces structured summaries for `/v1/dashboard/conversations/{id}`.
  Two modes: spawn the `claude` CLI when available, fall back to a
  direct Anthropic API call when `CORTEX_ANALYZER_API_KEY` is set
  (CI / server / Cursor-hosted CLI scenarios).
- Live `GraphLane` backed by Nexus.
- Live `VectorLane` backed by `vectorizer-sdk` 3.0.3.
- Live `MeiliKeywordLane` with source-attribution invariant.
- SSE timeline stream + reconnect ladder over Synap pub/sub.
- Per-project Decision filter on `/v1/dashboard/decisions`.
- `Conversations` and `Handoffs` views.

### Fixed
- Law catalogue derivation now reads `law_violation` envelopes
  rather than a hard-coded fallback.
- Canonical scope echo + slug-aware cache invalidation closed the
  cross-repo bleed where a query about repo X could hit a cache
  entry tagged for repo Y.

## [0.1.0] — initial scaffold

### Added
- HTTP service (Axum) exposing `/v1/query`, `/v1/status`, and the
  initial `/v1/dashboard/*` endpoints.
- Hybrid retrieval orchestrator with three lanes (vector, keyword,
  graph) and Reciprocal Rank Fusion.
- Result cache keyed on the semantic hash of `(intent, scope, query)`.
- MCP tool bindings re-exposing `/v1/query`.

[Unreleased]: https://github.com/hivellm/cortex/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/hivellm/cortex/releases/tag/v0.1.0
