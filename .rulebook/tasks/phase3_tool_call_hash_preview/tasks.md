## 1. Server — surface hash + preview on `TimelineEvent`
- [x] 1.1 Add `content_hash: Option<String>` and `preview: Option<String>` (plus `preview_truncated: bool`) to `TimelineEvent` in `crates/cortex-api/src/dashboard.rs`, both `#[serde(skip_serializing_if = "Option::is_none")]`
- [x] 1.2 Map `LaneHit.content_hash` → `TimelineEvent.content_hash` and `LaneHit.text` → `TimelineEvent.preview` (8 KiB cap; set `preview_truncated` when clipped) in `timeline_recent`
- [x] 1.3 Mirror the same mapping in the SSE handler so live rows carry the new fields without a polling-only workaround
- [x] 1.4 Add a `cortex doctor` probe (`tool_call_hash_coverage`) asserting ≥ 99% of archive-sourced `tool_call` rows in the last 24 h carry a non-null `content_hash`

## 2. GUI — render hash + preview in the Inspector
- [x] 2.1 Extend `TimelineEvent` in `gui/src/lib/api.ts` with the three new optional fields
- [x] 2.2 Add a `Content` section to `Inspector` (`gui/src/views/Timeline.tsx`) that renders `preview` for `kind === "tool_call"` with a copy button and a `(truncated — open full)` link when `preview_truncated`
- [x] 2.3 Add a `content_hash` row to the Inspector's Envelope `dl`, short form (`sha256:abc1234…`) with copy-to-clipboard
- [x] 2.4 Wire a click handler on the hash row that sets a `content_hash` filter on the timeline (new `Filters.content_hash` field; query param `content_hash=<full hex>`); cleared by the existing "clear filters" button

## 3. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 3.1 Update or create documentation covering the implementation — `docs/specs/16-dashboard-and-gui.md` (or its equivalent) gains the two new optional fields, the 8 KiB cap, and the `content_hash` filter contract
- [x] 3.2 Write tests covering the new behavior — `cortex-api` unit covers the mapper preserves `content_hash` and clips `preview` at exactly 8 KiB with `preview_truncated=true`; GUI Vitest covers the Inspector renders the Content section only for `tool_call` rows
- [x] 3.3 Run tests and confirm they pass — `cargo test -p cortex-api` + `pnpm --filter ./gui test` both green with zero warnings
