## 1. Snippet projector
- [ ] 1.1 In `crates/cortex-api/src/lanes.rs`, change `LaneHit → Snippet` projection so `text` carries the resolved body (first 1 KiB)
- [ ] 1.2 When the inline body is empty, resolve via the CAS store handle on `DashboardState` (sha256 from `body_ref`)
- [ ] 1.3 Soft budget: 50 ms or 3 CAS hops per query; on overrun, fall back to the symbol+path string and stamp `extras.body_truncated_reason`

## 2. Snippet wire shape
- [ ] 2.1 In `crates/cortex-api/src/types.rs`, ensure `Snippet { text, symbol, path, kind, ... }` carries `symbol` separately so legacy callers that read `text == symbol` still see the symbol
- [ ] 2.2 Add `body_truncated: bool` so the dashboard / pre-thinking can render an ellipsis cue

## 3. Pre-thinking bundle
- [ ] 3.1 In `crates/cortex-pre-thinking/src/bundle.rs`, change the snippet bullet from `path:artifact — path` to `path:lineno — first 200 chars of text`
- [ ] 3.2 Honor the budget: every line is ≤ 240 chars and the bundle stops adding snippets once `budget_bytes` is reached

## 4. Tests
- [ ] 4.1 Unit test: known-good envelope round-trips through the projector and `snippet.text != snippet.path`
- [ ] 4.2 Integration test: pre-thinking bundle for a fixed prompt contains content from at least one source file (regex match against a known phrase)
- [ ] 4.3 Regression: replay the audit query "phase9k cron scheduler retention" and assert the snippet body contains "scheduler" or "cron" (not just "TodoWrite")

## 5. Spec / docs
- [ ] 5.1 Update `docs/specs/11-query-api.md` §"Snippet shape" with the body-capture rule
- [ ] 5.2 Update `docs/specs/12-pre-thinking-injection.md` §"Bundle layout"

## 6. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 6.1 Update or create documentation covering the implementation
- [ ] 6.2 Write tests covering the new behavior
- [ ] 6.3 Run tests and confirm they pass
