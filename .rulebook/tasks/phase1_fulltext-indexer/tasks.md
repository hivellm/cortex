## 1. Crate scaffold
- [ ] 1.1 `cortex-fulltext` crate with `FulltextIndexer` trait + `Document` type
- [ ] 1.2 Worker binary `cortex-fulltext-worker` consuming `cortex.events.enriched` + `cortex.events.embedded`
- [ ] 1.3 Config via env (`CORTEX_FULLTEXT_*`)

## 2. Index settings
- [ ] 2.1 `cortex-fulltext/settings.v1.json` per index with searchable / filterable / sortable attrs, ranking rules, stop-words, synonyms, typo tolerance
- [ ] 2.2 `ensure_index` applies settings idempotently; fail-fast on incompatible pre-existing settings
- [ ] 2.3 Settings bump workflow: counter `fulltext.settings_bump`; no reindex required

## 3. Doc builders
- [ ] 3.1 One pure builder per event family (`tool_call`, `decision`, `turn`, `artifact`, `law_violation`, `law`, `memory`, `analysis_round`, `notification`)
- [ ] 3.2 Shared core fields (id, event_id, kind, content_hash, ts, repo, path, topics, severity, pii_risk, summary, title, body, language)
- [ ] 3.3 Per-kind `ext.*` sub-object

## 4. Identity + body rules
- [ ] 4.1 `doc_id = event_id` for live; `doc_id = "bootstrap:" + repo + ":" + path + ":" + content_hash` for bootstrap
- [ ] 4.2 Body selection: summary if raw >4 KB, else redacted raw; truncate body at 10 MB with `truncated=true`
- [ ] 4.3 Drop event with empty-body counter bump when redaction produces an empty body

## 5. Meili client + worker loop
- [ ] 5.1 HTTP client with retry + exp backoff; batch 1 000 docs per upsert
- [ ] 5.2 Fire-and-forget in live mode; task-await under `CORTEX_FULLTEXT_AWAIT_TASK=1` for bootstrap
- [ ] 5.3 Backpressure: pause consumer on sustained 503 (>30 s)
- [ ] 5.4 Publish `cortex.events.fulltext_indexed` on success

## 6. Observability
- [ ] 6.1 Counters + histograms per spec 08 §Observability
- [ ] 6.2 Empty-body counter + `fulltext.truncated` + `fulltext.task_failures` wired

## 7. Tail (mandatory)
- [ ] 7.1 Update `docs/specs/08-fulltext-indexer.md` status flag to 🟢 + index row
- [ ] 7.2 Integration tests: 10 000-event stream with per-kind routing; idempotent replay; bootstrap doc_id stability; typo tolerance (refator → refactor); synonyms (bug → defect); filterable query; sort by ts; 503 soak; schema-drift fail-fast
- [ ] 7.3 Run `cargo check && cargo clippy -- -D warnings && cargo test`; coverage ≥95%
