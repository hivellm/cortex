# Changelog

All notable changes to `cortex-fulltext` are documented here. The
format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/)
and the project uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- `analyses` family — `Kind::Analysis` events route to a dedicated
  `cortex-{repo}-analyses` index instead of the catch-all `misc`
  bucket so the dashboard's Analysis view scopes to a single index.
  (phase4e)
- `agent_call` events route to `cortex-{repo}-turns` (alongside
  `turn`) and a `routed_total` metric tracks the dispatch.
- Artifact routing now reads `context_path` extension before falling
  back to `classifier.topics`; previously every artifact landed in
  `docs` regardless of whether it was source code or prose.
- Per-project index isolation — index names embed the owning repo
  slug as `cortex-{repo}-{family}`.

### Fixed
- Tooling-only fields stripped from the settings PATCH so Meili no
  longer rejects the upsert.
- `primaryKey=id` passed on every upsert to align with the
  content-hash-derived id contract.
- Source-attribution invariant enforced on every keyword lane hit so
  the dashboard knows where each result came from.

## [0.1.0] — initial scaffold

### Added
- Worker draining `cortex.events.enriched` into Meilisearch.
- Routing matrix mapping `Kind` → family (`code`, `docs`,
  `decisions`, `turns`, `governance`, `misc`).
- Baked-in `SETTINGS_V1` applied to every index on creation.
- Body builder honouring oversize / summary / raw-truncate fallbacks.

[Unreleased]: https://github.com/hivellm/cortex/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/hivellm/cortex/releases/tag/v0.1.0
