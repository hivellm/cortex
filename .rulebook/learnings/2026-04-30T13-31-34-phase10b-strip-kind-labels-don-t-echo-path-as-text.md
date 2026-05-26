# phase10b — strip kind labels + don't echo path as text
**Source**: manual
**Date**: 2026-04-30
**Related Task**: phase10b_snippet_body_capture
**Tags**: snippet, projection, meili, vectorizer, phase10b, body-truncated
Two distinct snippet projection bugs hit the same audit symptom (`Cortex/.../types.rs:artifact — \n   crates/cortex-api/src/types.rs`):

1. **Kind label leaked into `Snippet.symbol`**. Both meili_lane and vectorizer_lane stamp `LaneHit.symbol = doc.kind` (`artifact`/`turn`/`decision`/etc.) for back-compat with overlay derivation. Spec 11 says `Snippet.symbol` is the Tree-sitter symbol or H1 — kind labels do NOT belong there. Fix: filter `SYMBOL_KIND_LABELS` in `snippet_from_hit` before constructing the wire `Snippet`. Decisions get the curated `decision_title` extras (phase10a) as fallback.

2. **Path echoed as `Snippet.text`**. The fulltext-worker's `derive_title` for artifact events sets `title = path` (when no richer heading is available). When `body` was empty, the kind-aware projection chain stopped at `title` and surfaced the path as the snippet body. Fix: in the projection chain `find_map`, skip slots whose value equals `doc.path`. When the entire chain produces no path-distinct text, return empty `text` and stamp `extras.body_truncated = true` so the bundle renderer collapses to a header-only line with `(body not indexed inline)` cue.

Wire-shape change: added `Snippet.body_truncated: bool` (additive, skip-serialised, default false) so existing consumers stay backwards-compatible.

The proposal mentioned CAS resolution via a `body_ref` field, but `cortex-core` doesn't carry such a field today (only `content_hash` for canonical payload identity). The CAS plumbing was deferred — when the inline body is missing, the lane just flags `body_truncated` rather than re-fetching from CAS.