# Changelog

All notable changes to `cortex-core` are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0] — initial scaffold

### Added
- Event envelope + payload Rust types mirroring the JSON Schemas under
  `schemas/` (spec 01).
- Canonical JSON encoder (`canonical_json`) used for content hashing.
- SHA-256 content hash helper (`content_hash`).
- ULID-based identifier types (`EventId`, `SessionId`) with strict parsing.
- Validator (`validate_event`, `validate_envelope`) backed by the bundled
  JSON Schemas (spec 01).
- Defense-in-depth redaction pass (`redact`) with the v1 pattern catalog
  covering common PII and secret shapes (spec 04).
- Frozen vocabularies (`vocab`) for kinds, severities, and PII-risk levels.
- `cortex-core` CLI with `validate`, `hash`, and `redact` subcommands for
  hooks and CI.

[Unreleased]: https://github.com/hivellm/cortex/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/hivellm/cortex/releases/tag/v0.1.0
