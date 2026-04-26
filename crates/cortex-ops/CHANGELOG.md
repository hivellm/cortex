# Changelog

All notable changes to `cortex-ops` are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0] — initial scaffold

### Added
- `plan` subcommand serializing the bootstrap layout declared by
  `cortex-storage` (Vectorizer collections, Nexus Cypher, Meilisearch
  settings, Synap streams + KV namespaces) as JSON, with a `--slice`
  selector and `--pretty` flag.
- `doctor` subcommand that probes each backend (Vectorizer, Nexus,
  Meilisearch, Synap) and reports liveness with a non-zero exit code on
  any failure.
- Environment-variable defaults (`VECTORIZER_URL`, `NEXUS_URL`,
  `MEILI_URL`, `SYNAP_URL`) overridable via flags.

[Unreleased]: https://github.com/hivellm/cortex/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/hivellm/cortex/releases/tag/v0.1.0
