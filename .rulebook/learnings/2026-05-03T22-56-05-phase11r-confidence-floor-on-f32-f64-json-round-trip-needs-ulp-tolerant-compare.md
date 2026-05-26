# phase11r — confidence floor on f32→f64 JSON round-trip needs ULP-tolerant compare
**Source**: manual
**Date**: 2026-05-03
**Related Task**: phase11r_topic_card_mcp_enrichment
**Tags**: phase11r, audit, f32, json, tests, topic-card, confidence-floor
During phase11r §4.1 the audit envelope for `cortex_topic_get` carries the `confidence: f32` value from the `TopicCardPayload` into a `serde_json::Value::Number`. Asserting `assert_eq!(envs[0]["result"]["confidence"], 0.82)` failed because `0.82_f32` widens to `0.81999999...` in f64 — JSON's number type is f64, so the wire shape carries the wider value.

The fix is to read the JSON number back as f64, cast to f32, and compare against the source f32 with `< f32::EPSILON`. This is a one-line change but the failure mode is non-obvious — the test passes locally with f64-clean values like 0.5 / 0.75 / 0.875 (representable exactly in both) and fails the moment a real-world tuned value (0.82, 0.6, 0.45) lands in the assertion.

Pattern that works:

```rust
let recorded = envs[0]["result"]["confidence"]
    .as_f64()
    .expect("confidence is a number");
assert!((recorded as f32 - 0.82_f32).abs() < f32::EPSILON);
```

Pattern that fails non-obviously:

```rust
assert_eq!(envs[0]["result"]["confidence"], 0.82);  // ← fails for non-binary-clean f32
```

This applies anywhere a typed f32 round-trips through JSON and back into a test assertion. The same caveat applies to embedding scores, similarity floors, and any other confidence/probability field captured in audit envelopes. Worth pinning as a pattern across the cortex-api audit suite.