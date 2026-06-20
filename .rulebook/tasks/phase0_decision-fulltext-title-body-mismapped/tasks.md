## 1. Confirm builder + find the malformed-batch source
- [ ] 1.1 Verify the current decision doc builder in `crates/cortex-workers/src/fulltext/builders.rs` (+ routing) sets `title`/`decision_title`/`body` from the decision heading, not the id (the `01KQNYMYKH` doc proves it can)
- [ ] 1.2 Identify how the `01KQNYF4J*` batch got `title==id` + missing `decision_title` (older builder revision, or an ingest path that bypassed title extraction); fix that path if it still exists

## 2. Reindex + verify the mix is gone
- [ ] 2.1 Re-emit / reindex `cortex_decisions` through the current builder so the stale malformed batch is corrected (document the command)
- [ ] 2.2 Live: `cortex_keyword_search cortex_decisions q="" attributes=[id,title,decision_title]` shows every doc with a real title + decision_title (none where `title == id`)

## 3. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 3.1 Update spec 08 (decision doc schema) + CHANGELOG
- [ ] 3.2 Builder unit test: decision doc `title != id` and `body` is not a JSON-object string
- [ ] 3.3 Run `cargo check` + `clippy -D warnings` + `cargo test --workspace`
