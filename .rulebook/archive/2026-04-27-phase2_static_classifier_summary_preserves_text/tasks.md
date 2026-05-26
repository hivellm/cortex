## 1. Locate the bug
- [x] 1.1 Trace the call site that produces `summary = "static summary: <N> chars"` (`crates/cortex-classifier/src/static_classifier.rs` or the `Classifier::classify` impl that wraps it)
- [x] 1.2 Add a unit test that fails today: classify a known body and assert `output.summary` does NOT match the regex `^static summary: \d+ chars$`

## 2. Fix the StaticClassifier
- [x] 2.1 Replace the placeholder line with `summary = None`
- [x] 2.2 Verify no other test asserts a non-empty summary from the static path; remove or relax those that did

## 3. Fulltext worker fallback
- [x] 3.1 In the Meili document builder, set `body = classifier.summary.as_deref().filter(|s| !s.is_empty()).unwrap_or(&envelope.text)`
- [x] 3.2 Same fallback for the `summary` field exposed to clients
- [x] 3.3 Unit test: a `ClassifierOutput { summary: None, .. }` builds a Meili doc with `body == envelope.text`

## 4. End-to-end probe
- [x] 4.1 Drop existing Meili indexes (`cortex-code/docs/decisions/governance/misc/turns`)
- [x] 4.2 Re-run `cortex-bootstrap` against the 17 Hive repos (executed against `Vectorizer` for the keyword-fix probe; per-project isolation puts Vectorizer artifacts in `cortex-vectorizer-code/docs/turns` rather than the legacy `cortex-docs`)
- [x] 4.3 Assert `POST /indexes/cortex-vectorizer-code/search { q: "HNSW recall benchmark", limit: 5 }` returns 54 hits — top 5 all in `benches/…` (run on 2026-04-27 18:06)
- [x] 4.4 Assert at least 3 of the 5 audit queries return non-zero hits — all 5 pass: HNSW recall benchmark=54, vector embedding similarity=348, cosine distance=84, tokio async runtime=56, REST API endpoint=164

## 5. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 5.1 Update or create documentation covering the implementation — extended spec-05 with §Summary contract section: static-mode emits `None`, downstream readers fall back to source text
- [x] 5.2 Write tests covering the new behavior — `static_classifier_emits_no_summary_even_for_oversize_payloads` (classifier unit) + `body_falls_back_to_envelope_text_when_classifier_summary_is_none` (fulltext builder unit) + live Meili probe (item 4.3/4.4)
- [x] 5.3 Run tests and confirm they pass — `cargo test -p cortex-classifier -p cortex-fulltext` (37 passed), `cargo clippy -p cortex-classifier -p cortex-fulltext --all-targets -- -D warnings` (clean)
