# Changelog

All notable changes to `cortex-adapter-claude-code` are documented
here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/)
and the project uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- `Stop` hook captures `assistantMessage` and emits a `Turn` envelope
  closing the user-prompt → assistant-reply asymmetry that previously
  left replies unrecorded.
- Pre-thinking sync paths now flow through
  [`cortex-pre-thinking`](../cortex-pre-thinking/) instead of a
  bespoke HTTP client — single source of truth for bundle assembly.

### Fixed
- MCP-style hook descriptors normalised to identifier-safe names + a
  camelCase schema so Claude Code accepts them without dropping fields.

## [0.1.0] — initial scaffold

### Added
- Local hook daemon (`cortex-adapter-claude`) wired to
  `UserPromptSubmit`, `PreToolUse`, `PostToolUse`, `SubagentStop`,
  `Stop`, `Notification` hook entry points.
- Envelope publication on `cortex.events.raw` via Synap.
- Pre-thinking round-trip to `cortex-api /v1/query` with byte-budget
  enforcement and per-section caps.
- Recent-file TTL cache so the scope-derivation heuristics do not
  shell out to git on every prompt.

[Unreleased]: https://github.com/hivellm/cortex/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/hivellm/cortex/releases/tag/v0.1.0
