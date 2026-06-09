## §1. Bug #2 — Divergence checker: counter alignment + alert format

- [x] §1.1 Counter is in `events_archived_total` map in ingestion health extras (BTreeMap keyed by kind label)
- [x] §1.2 Already published correctly via `metrics.incr_events_archived_kind` in `ingestion/router.rs`; no code change required
- [x] §1.3 Removed synthetic `law_violation` POST from `silent_drop.rs::tick()`; replaced with `tracing::warn!` with structured fields. Webhook path kept.
- [x] §1.4 Verified via unit tests; counter alignment confirmed by reading router.rs publish path. Live container check on next redeploy.
- [x] §1.5 Verified via code: `post_envelope_to_ingestion` call removed; no "01ALERT…" envelopes can be generated. Live log check on next redeploy.

## §2. Bug #6 — Fulltext worker: fallback extraction

- [x] §2.1 `BodySource::Empty` is returned when both raw payload text and summary are absent — triggers `incr_events_dropped` in indexer
- [x] §2.2 Added `BodySource::Fallback` variant and `minimal: &str` param to `select_body`; `builders.rs` passes `"{kind} {event_id}"` as last-resort fallback
- [x] §2.3 `events_dropped_empty` counter fires only if minimal is also blank — impossible in practice since event_id is always non-empty
- [x] §2.4 Verified via updated unit tests (indexer test confirms `documents_upserted=1, dropped=0` for empty-payload artifact). Live metric check on next redeploy.

## §3. Bug #7 — Frames/envelopes ratio: exclude non-capture hooks

- [x] §3.1 Pair defined in `health.rs::build_divergence_pairs` — sums `frames_parsed_total` across all hook types
- [x] §3.2 Changed pair 1 in `health.rs` to filter out PreToolUse+UserPromptSubmit before summing `capture_frames`
- [x] §3.3 Ratio threshold change not required — divergence is delta_growth-based (>10 warn / >50 critical); removing non-capture hooks makes delta_growth ~0 at steady state
- [x] §3.4 Verified via code and unit tests. Live endpoint check on next redeploy.

## §4. Tail (mandatory)

- [x] §4.1 Updated `docs/analysis/cortex/12-live-audit-2026-06-09.md` — added "Fixes Applied (phase26b)" section documenting bugs #2, #6, #7
- [x] §4.2 Tests written and passing: fulltext fallback (`empty_payload_uses_minimal_fallback_not_skipped` in fulltext_builders.rs, `empty_payload_event_uses_minimal_fallback_and_is_upserted` in fulltext_indexer.rs); divergence pair filter covered by existing `build_divergence_pairs_*` tests in health.rs; 2 new body.rs unit tests for fallback + whitespace-minimal path
- [x] §4.3 `cargo test -p cortex-workers -p cortex-api` — all pass (zero failures)

## 99. Mandatory tail (rulebook v5.3.0)
- [x] 99.1 Update or create documentation covering the implementation. (done via §4.1 — analysis doc updated)
- [x] 99.2 Write tests covering the new behavior. (done via §4.2 — 4 new/updated tests)
- [x] 99.3 Run tests and confirm they pass. (done via §4.3 — zero failures)
