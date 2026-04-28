# Changelog

All notable changes to `cortex-bootstrap` are documented here. The
format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/)
and the project uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- `[cortex.analyses]` config block + `FileClass::Analysis` +
  `analysis.imported` event kind. Files matching
  `cortex.analyses.promote_patterns` (default
  `docs/analysis/**/*.md` and `docs/analyses/**/*.md`) reach the
  dashboard's Analysis surface as first-class entities. (phase4e)
- Default-discovery for `.rulebook/*` paths across every walked
  repo so sibling Hive repos without a `cortex.toml` still emit
  their decisions, knowledge, learnings, and handoffs.
- One Session per bootstrap run + Turn label fallback so historical
  commits get a useful display name in Nexus.
- Per-event publish-failure tolerance (5 % / 20-floor) so transient
  Synap "Room not found" misses do not abort the walk.

### Fixed
- `strip_prefix_ci` now respects UTF-8 character boundaries — a
  multi-byte character (em-dash, accented vowel) shorter than the
  prefix no longer panics the parser.
- Tooling-only fields stripped from JSON payloads sent verbatim to
  strict downstreams (Meili settings rejected unknown keys).

## [0.1.0] — initial scaffold

### Added
- Single-repo walker honouring `.gitignore` plus per-repo `cortex.toml`
  excludes and the 8 MB oversize gate.
- Synthetic event emitter for `artifact.code`, `artifact.doc`,
  `turn.historical`, `decision.imported`, `law.imported`, and
  `memory.imported`.
- Synap publisher with at-least-once semantics and lazy room
  creation on first 404.
- Resumable `.cortex-bootstrap.state.json` checkpoint.
- `--dry-run --estimate` sizing block.

[Unreleased]: https://github.com/hivellm/cortex/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/hivellm/cortex/releases/tag/v0.1.0
