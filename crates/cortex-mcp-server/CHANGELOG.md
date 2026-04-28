# Changelog

All notable changes to `cortex-mcp-server` are documented here. The
format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/)
and the project uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0] — initial scaffold

### Added
- stdio JSON-RPC server speaking MCP for `cortexQuery`,
  `cortexPreThinking`, and `cortexStatus`.
- Identifier-safe tool names (no dots) + camelCase schema fields so
  MCP clients accept the descriptors without dropping fields.
- Pass-through routing to `cortex-api` and `cortex-pre-thinking` —
  the server owns no domain logic of its own.

[Unreleased]: https://github.com/hivellm/cortex/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/hivellm/cortex/releases/tag/v0.1.0
