# Changelog

All notable changes to `cortex-graph` are documented here. The format
follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the
project uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- Crate scaffold for spec 07: `mapper`, `coalescer`, `cypher`, `schema`,
  `identity`, `nexus_client`, `patch`, `writer`, `worker`, `metrics`,
  `config`.
- `GraphConfig` + `GraphTransport` (Nexus RPC vs HTTP) loaded from
  environment.
- Deterministic node identity (`identity`) so MERGE upserts are
  idempotent across re-runs and bootstrap replays.
- Patch types and `PatchCoalescer` that fold duplicate node/edge upserts
  inside a micro-batch.
- Parameterized Cypher templates for constraints, indexes, and MERGE
  statements (`CypherTemplates`).
- `GraphWriter` glue plus the `cortex-graph-worker` binary skeleton
  (Synap consumer loop on `cortex.events.enriched`, Prometheus metrics).
- Re-export of `EnrichedEvent` from `cortex-embedder` so both workers
  stay in lockstep on the enriched-stream payload.

[Unreleased]: https://github.com/hivellm/cortex/compare/v0.1.0...HEAD
