## 1. Locate the bug
- [ ] 1.1 Trace the call site that produces `summary = "static summary: <N> chars"` (`crates/cortex-classifier/src/static_classifier.rs` or the `Classifier::classify` impl that wraps it)
- [ ] 1.2 Add a unit test that fails today: classify a known body and assert `output.summary` does NOT match the regex `^static summary: \d+ chars$`

## 2. Fix the StaticClassifier
- [ ] 2.1 Replace the placeholder line with `summary = None`
- [ ] 2.2 Verify no other test asserts a non-empty summary from the static path; remove or relax those that did

## 3. Fulltext worker fallback
- [ ] 3.1 In the Meili document builder, set `body = classifier.summary.as_deref().filter(|s| !s.is_empty()).unwrap_or(&envelope.text)`
- [ ] 3.2 Same fallback for the `summary` field exposed to clients
- [ ] 3.3 Unit test: a `ClassifierOutput { summary: None, .. }` builds a Meili doc with `body == envelope.text`

## 4. End-to-end probe
- [ ] 4.1 Drop existing Meili indexes (`cortex-code/docs/decisions/governance/misc/turns`)
- [ ] 4.2 Re-run `cortex-bootstrap` against the 17 Hive repos
- [ ] 4.3 Assert `POST /indexes/cortex-code/search { q: "HNSW recall benchmark", limit: 5 }` returns at least one hit whose `path` starts with `Vectorizer/`
- [ ] 4.4 Assert at least 3 of the 5 audit queries from the proposal return non-zero hits

## 5. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 5.1 Update or create documentation covering the implementation — extend spec-05 with a §Summary contract section saying static-mode emits `None`
- [ ] 5.2 Write tests covering the new behavior — classifier unit test + fulltext doc-builder unit test + e2e probe
- [ ] 5.3 Run tests and confirm they pass — `cargo test -p cortex-classifier -p cortex-fulltext`, `cargo clippy -- -D warnings`
