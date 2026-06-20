## 1. Diagnose + fix the decision doc builder
- [ ] 1.1 Locate the decision Meili document builder in `crates/cortex-workers/src/fulltext/builders.rs` (+ routing); confirm where `title`/`body` are set
- [ ] 1.2 Map `title` to the real decision title (heading / `title` field), not the document id
- [ ] 1.3 Map `body` to the decision markdown/text, not the JSON-serialized payload (fix the double-encoding)

## 2. Reindex + verify
- [ ] 2.1 Re-emit / reindex `cortex_decisions` so existing docs are corrected (document the step)
- [ ] 2.2 Live: `POST /v1/decisions/search {"query":"vectorizer","limit":2}` returns hits whose `title` is the real title and `body` is clean text

## 3. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 3.1 Update spec 08 (decision doc schema) + CHANGELOG
- [ ] 3.2 Builder unit test: decision doc `title != id` and `body` is not a JSON-object string
- [ ] 3.3 Run `cargo check` + `clippy -D warnings` + `cargo test --workspace`
