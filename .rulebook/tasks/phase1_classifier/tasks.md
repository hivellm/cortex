## 1. Crate scaffold
- [ ] 1.1 `cortex-classifier` crate with `Classifier` trait + `ClassifierOutput` type
- [ ] 1.2 `cortex-workers` crate with the worker binary `cortex-classifier-worker`
- [ ] 1.3 Config struct via env + optional TOML (`CORTEX_CLASSIFIER_*`)

## 2. Prompt + vocabulary
- [ ] 2.1 `cortex-classifier/prompts/classifier.v1.txt` with `{{TOPIC_VOCAB}}` + `{{EVENTS_JSON}}` placeholders
- [ ] 2.2 `cortex-classifier/topics.yaml` with the ~200-term controlled vocabulary (seed set + categories)
- [ ] 2.3 Hot-reload on SIGHUP; `prompt_version` propagated into `ClassifierOutput`

## 3. Haiku backends
- [ ] 3.1 `HaikuCliClassifier` invokes `claude -p ... --model claude-haiku-4-5 --output-format json`; file-backed prompt when argv too large
- [ ] 3.2 `HaikuSdkClassifier` uses Anthropic SDK with JSON response format
- [ ] 3.3 Shared `validate_and_match`: length check, id-match, enum enforcement, out-of-vocab drop, summary-on-oversize

## 4. Cache + budget + fallback
- [ ] 4.1 `CachedClassifier` wraps inner; Synap KV lookup + write (TTL 24h); per-event cache granularity
- [ ] 4.2 `BudgetTracker` persisted in SQLite; thresholds 0.8 / 0.9 / 1.0 with prompt-shrink + batch-raise + halt
- [ ] 4.3 `StaticClassifier` with rules table in `cortex-classifier/static-rules.yaml`

## 5. Worker loop
- [ ] 5.1 Consume `cortex.events.raw` + `cortex.events.bootstrap`; batch 32 events / 200 ms flush
- [ ] 5.2 Publish enriched event to `cortex.events.enriched`; cache + budget update on success
- [ ] 5.3 Failure routing: retries, dead-letter to `cortex.events.invalid` with cause

## 6. Observability
- [ ] 6.1 Counters + histograms per spec 05 §Observability
- [ ] 6.2 Span per batch with cache hits, tokens, cost, source
- [ ] 6.3 `cortex.classifier.budget.state` gauge (0 normal → 3 halt)

## 7. Tail (mandatory)
- [ ] 7.1 Update `docs/specs/05-classifier.md` status flag to 🟢 + index row
- [ ] 7.2 Integration tests: golden 32-event batch (both CLI + SDK); cache second-run replay; vocabulary enforcement; budget-halt rerouting; static fallback on unreachable Haiku
- [ ] 7.3 Run `cargo check && cargo clippy -- -D warnings && cargo test`; coverage ≥95%
