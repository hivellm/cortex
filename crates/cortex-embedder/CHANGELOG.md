# Changelog

All notable changes to `cortex-embedder` are documented here. The format
follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the
project uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0] — initial scaffold and worker (spec 06)

### Added
- `Chunker` trait + three implementations: `CodeChunker` (Tree-sitter for
  Rust, TS/JS, Python, Go, Java, C/C++), `DocChunker` (Markdown / RST /
  plaintext), and `FallbackChunker` (sliding window).
- Deterministic `dedup_key` per chunk (`identity::dedup_key`) for
  idempotent re-runs against Vectorizer.
- Routing layer (`routing::collection_for`) selecting the destination
  Vectorizer collection from `cortex-storage`.
- `VectorizerClient` trait with `LiveVectorizerClient` (HTTP via
  `vectorizer-sdk`) and `MemoryVectorizerClient` (tests), plus
  `with_retry` exponential-backoff helper.
- `Embedder` trait + `VectorizerEmbedder` wiring chunker → routing →
  client.
- `cortex-embedder-worker` binary: Synap consumer loop on
  `cortex.events.enriched`, batched upserts, Prometheus metrics, graceful
  shutdown.
- Configuration via `EmbedderConfig` (env vars + TOML).

[Unreleased]: https://github.com/hivellm/cortex/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/hivellm/cortex/releases/tag/v0.1.0
