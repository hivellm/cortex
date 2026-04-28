# Changelog

All notable changes to `cortex-classifier-worker` are documented
here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/)
and the project uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- `analysis.imported` (and `analysis`) bootstrap kinds map to
  `Kind::Analysis` so phase4e's audit / deep-analysis events flow
  through the worker into `cortex.events.enriched`.
- Static-fallback path now propagates the (empty) `entities` and
  `relations` slots so Sonnet-emitted typed nodes can flow through
  unchanged when the budgeted classifier promotes its output.

### Fixed
- `--max-tokens` flag dropped from the CLI invocation for Claude
  Code 2.x compatibility (the flag was renamed and broke the
  command line).

## [0.1.0] — initial scaffold

### Added
- Standalone crate that drains `cortex.events.raw` and
  `cortex.events.bootstrap`, classifies each envelope through a
  configurable stack (`Static`, `HaikuCli`, both behind cache +
  budget), and publishes `EnrichedEvent`s on `cortex.events.enriched`.
- ADR-002 path: separate crate from `cortex-classifier` to avoid the
  `classifier → embedder → classifier` dependency cycle.
- Bootstrap-kind ↔ canonical `Kind` mapping (`kind_from_bootstrap`)
  with case-insensitive matching.
- In-memory replay-dedup keyed on `event_id` for at-least-once
  delivery.
- Lazy Synap room creation on the first "Room not found" error so
  the worker tolerates startup-order races against bootstrap and
  live capture.

[Unreleased]: https://github.com/hivellm/cortex/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/hivellm/cortex/releases/tag/v0.1.0
