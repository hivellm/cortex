# Proposal: phase1_classifier

## Why

Every enriched event (topics, severity, PII risk, redaction suggestions, summary) feeds embedder + graph writer + full-text indexer + query API. Without a working classifier, the corpus stays raw and retrieval is keyword-only. This task delivers the Haiku-backed worker that turns raw events into enriched ones, with a content-hash cache so bootstrap replays are free and a budget tracker so runaway cost degrades instead of explodes.

## What Changes

- `cortex-classifier` crate: `Classifier` trait + `HaikuCliClassifier`, `HaikuSdkClassifier`, `CachedClassifier`, `BudgetedClassifier`, `StaticClassifier`.
- Prompt template v1 + topic vocabulary YAML (hot-reloadable on SIGHUP).
- JSON output parser with schema validation + vocabulary enforcement.
- Content-hash cache in Synap KV (`cache:classify:v1:<hash>`).
- Budget tracker persisted in SQLite `classifier_spend`; 3-tier degradation ladder.
- Static fallback classifier (pure Rust rules).
- Worker binary consuming `cortex.events.raw` + `cortex.events.bootstrap`, publishing `cortex.events.enriched`.

## Impact

- **Affected specs:** [`docs/specs/05-classifier.md`](../../../docs/specs/05-classifier.md); unblocks 09.
- **Affected code:** new `cortex-classifier/` crate, new `cortex-workers/` crate with the worker binary, prompt + vocab files under `cortex-classifier/prompts/` and `cortex-classifier/topics.yaml`.
- **Breaking change:** NO — greenfield.
- **User benefit:** enables topic / severity / PII filtering across every retrieval intent.

## Source

`docs/specs/05-classifier.md` · depends on specs 01 + 04 · PRD FR-5.
