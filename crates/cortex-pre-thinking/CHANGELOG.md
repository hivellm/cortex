# Changelog

All notable changes to `cortex-pre-thinking` are documented here.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/)
and the project uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed
- Adapter sync paths now flow through this crate instead of a
  bespoke HTTP client in `cortex-adapter-claude-code`. Single source
  of truth for bundle assembly across every adapter.

## [0.1.0] — initial scaffold

### Added
- Deterministic Markdown bundle formatter for adapter-side
  pre-thinking injection.
- Byte-budget enforcement (default 32 KB) with per-section fairness
  caps (decisions, similar turns, snippets, laws).
- Scope-derivation heuristics turning user prompt + cwd + recent
  files into `cortex-api` query scope.
- `query_id` tracing on every assembled bundle for retrieval-quality
  analysis.

[Unreleased]: https://github.com/hivellm/cortex/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/hivellm/cortex/releases/tag/v0.1.0
