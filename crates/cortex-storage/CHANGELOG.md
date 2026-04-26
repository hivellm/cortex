# Changelog

All notable changes to `cortex-storage` are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0] — initial scaffold

### Added
- Vectorizer collection catalog (`collections`) with name, dimension, metric,
  tier filter, and bundled JSON schema per collection.
- Nexus graph layout (`graph`): label set + bootstrap Cypher (constraints
  and indexes for the Cortex schema).
- Meilisearch full-text layout (`fulltext`): index names + canonical
  `settings.v1.json`.
- Synap stream / topic / KV namespace declarations (`streams`).
- Parquet archive layout (`archive`) with partition + rotation helpers.
- SQLite metadata store (`metadata`) covering sessions, events, and runs.
- SQLite-backed CAS blob store (`cas`) with zstd compression.
- `names` module re-exporting every well-known constant for typed lookup.

[Unreleased]: https://github.com/hivellm/cortex/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/hivellm/cortex/releases/tag/v0.1.0
