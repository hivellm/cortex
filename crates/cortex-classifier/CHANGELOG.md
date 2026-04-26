# Changelog

All notable changes to `cortex-classifier` are documented here. The format
follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the
project uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0] — initial scaffold

### Added
- `Classifier` trait + `ClassifierOutput` (topics, severity, PII risk,
  redaction suggestions, optional summary) (spec 05).
- `HaikuCliClassifier` driving Claude Haiku through the Claude Code CLI
  with the prompt template under `prompts/` and frozen topic vocabulary
  `TOPIC_VOCAB_V1`.
- `StaticClassifier` regex/heuristic fallback so the pipeline degrades
  gracefully when Haiku is unavailable.
- `CachedClassifier` + `InMemoryCache` content-hash cache layer.
- `BudgetedClassifier` + `BudgetTracker` for per-mode daily budgets and
  graceful degradation to the static fallback.
- `ClassifierStack` composer wiring the default decorator chain.
- `ClassifierSpend` + `PricingTable` for token-and-cost accounting.

[Unreleased]: https://github.com/hivellm/cortex/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/hivellm/cortex/releases/tag/v0.1.0
