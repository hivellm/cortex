## 1. SSE health stream backend
- [ ] 1.1 NEW `crates/cortex-api/src/health/stream.rs`
- [ ] 1.2 Handler `GET /v1/health/stream` returns an SSE stream emitting one `health` event every 5 s with the full `HealthSnapshot { aggregator, freshness, divergence, versions, config, canary_recent }`
- [ ] 1.3 Reuse the same per-subscriber pattern as `/v1/dashboard/timeline/stream` (heartbeat every 15 s, reconnect-safe)
- [ ] 1.4 Wire route in `dashboard.rs`
- [ ] 1.5 Cap snapshot bytes at 64 KB; truncate `findings[]` if larger and add `truncated: true`

## 2. API clients in GUI
- [ ] 2.1 Add typed clients in `gui/src/lib/api.ts`: `health.overview()`, `health.freshness()`, `health.divergence()`, `health.versions()`, `health.config()`, `health.canaryHistory()`
- [ ] 2.2 Add `useHealthStream()` hook wrapping `useSSE` for the new SSE endpoint
- [ ] 2.3 TypeScript types matching the Rust shapes

## 3. Health view
- [ ] 3.1 NEW `gui/src/views/Health.tsx` with the 5-section layout: overall banner, subsystems grid, freshness table, divergence table, version drift, config audit, canary history
- [ ] 3.2 Subsystem card component (`gui/src/components/SubsystemCard.tsx`): state pill, version, uptime, sparkline
- [ ] 3.3 Freshness table component sorts by gap_seconds desc; colour-coded (>60s yellow, >300s red)
- [ ] 3.4 Reuse the existing Inspector aside for click-through expansion
- [ ] 3.5 Empty/loading states identical to the existing views

## 4. Sidebar + topbar integration
- [ ] 4.1 Add "Health" entry to `gui/src/shell/Sidebar.tsx` with a badge showing the count of subsystems whose state != ok
- [ ] 4.2 Add a tiny health pill to the topbar (green/yellow/red dot) visible from every view; click jumps to /health
- [ ] 4.3 Both the sidebar badge and topbar pill subscribe to `useHealthStream` so updates are real-time without polling

## 5. Routing
- [ ] 5.1 Add `/health` route in `gui/src/App.tsx`
- [ ] 5.2 SessionState filters do not apply to Health view (it's stack-wide)

## 6. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 6.1 Update `docs/architecture.md` (GUI section) and `gui/README.md`; add `gui/CHANGELOG.md` entry; add CHANGELOG entry on cortex-api
- [ ] 6.2 Tests: unit test for `aggregate()` rendering of state pill; SSE stream integration test (boots cortex-api with a stub aggregator); React Testing Library tests for Health.tsx (loading / loaded / error states); Playwright test that opens /health and asserts each section renders
- [ ] 6.3 Run `npm test` (gui) and `cargo test -p cortex-api` and confirm all pass with ≥95% coverage on new GUI files
