# Changelog

All notable changes to `cortex-ingestion` are documented here. The format
follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the
project uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0] — initial scaffold

### Added
- Axum HTTP router with `/v1/events`, `/v1/events/batch`, `/healthz`, and
  `/metrics` endpoints (spec 04).
- Schema validation against `cortex-core` envelope/payload JSON Schemas
  on every accepted event, with structured error responses.
- Defense-in-depth redaction pass (`cortex_core::redact`) before any
  payload reaches disk.
- NDJSON + zstd archive writer (`NdJsonZstdArchive`) with per-file size
  rotation and atomic file roll-over.
- `Publisher` abstraction with `SynapPublisher` (production) and
  `MemoryPublisher` (tests) implementations.
- Live/bootstrap stream routing keyed off `envelope.source.mode`.
- Prometheus metrics (`Metrics`) exposing accept/reject counters, archive
  bytes, and Synap publish latencies.
- TOML + environment-variable configuration loader (`IngestionConfig`).

[Unreleased]: https://github.com/hivellm/cortex/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/hivellm/cortex/releases/tag/v0.1.0
