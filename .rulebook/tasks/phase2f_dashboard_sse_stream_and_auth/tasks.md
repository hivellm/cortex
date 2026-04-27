## 1. Backend — SSE timeline stream
- [ ] 1.1 Add `/v1/dashboard/timeline/stream` route in `crates/cortex-api/src/dashboard.rs` returning `Sse<impl Stream<Item = ...>>`
- [ ] 1.2 Use `tokio::sync::broadcast` channel sized at 256 to fan envelopes from the lane to subscribers; `MemoryKeywordLane` pushes new envelopes via a sender side stored in `DashboardState`
- [ ] 1.3 Each event encoded as `axum::response::sse::Event::default().id(envelope_id).event("timeline").json_data(payload)`
- [ ] 1.4 Heartbeat every 15 s via `tokio::time::interval`; encoded as `event: heartbeat data: {}`
- [ ] 1.5 Honour `Last-Event-ID` header: replay any in-memory envelopes newer than that id before joining the live broadcast
- [ ] 1.6 Honour `?repo`, `?session_id`, `?kind` query params using the same filter helpers as `/timeline/recent`
- [ ] 1.7 Backpressure: when subscribers > 100, drop receivers whose channel `len()` > 64 (record `cortex.dashboard.sse.dropped` counter)

## 2. Backend — Auth middleware
- [ ] 2.1 Create `crates/cortex-api/src/auth.rs` exporting `require_api_key` Axum middleware
- [ ] 2.2 Apply via `Router::layer(from_fn(require_api_key))` to the dashboard sub-router (do not apply to `/v1/status` to keep liveness probes anonymous)
- [ ] 2.3 401 body matches existing error shape: `{ "reason": "missing_or_invalid_api_key" }`
- [ ] 2.4 Constant-time comparison via `subtle::ConstantTimeEq` to avoid timing leaks
- [ ] 2.5 Hash keys at rest with Argon2id; SQLite table `api_keys (id, scope, hash, created_at, last_used_at)` migrated via `sqlx`

## 3. Backend — Admin sub-command
- [ ] 3.1 Add `cortex-api admin issue-api-key --scope dashboard [--label <name>]` sub-command using `clap`
- [ ] 3.2 Generate a 32-byte random key via `rand::rngs::OsRng`, encode as `cortex_dash_<base32>`
- [ ] 3.3 Print the cleartext key to stdout exactly once; persist only the Argon2id hash
- [ ] 3.4 Add `cortex-api admin list-api-keys` and `cortex-api admin revoke-api-key <id>` for symmetry

## 4. Frontend — useSSE hook
- [ ] 4.1 Create `gui/src/lib/useSSE.ts` exporting `useSSE<T>(url: string, opts?: { lastEventId?: string })`
- [ ] 4.2 Open an `EventSource` once mounted; clean up on unmount
- [ ] 4.3 Reconnect ladder: 1 s, 2 s, 5 s, 10 s, 30 s (capped); reset on a successful event
- [ ] 4.4 Track `lastEventId` in a ref; pass via the standard `Last-Event-ID` mechanism on reconnect
- [ ] 4.5 Yield events via a callback prop; expose `connected: boolean` and `lastHeartbeatAt: number` from the hook

## 5. Frontend — wire Timeline + auth modal
- [ ] 5.1 Timeline backfills via the existing `useQuery(/timeline/recent)`; live updates come from `useSSE(/timeline/stream)`
- [ ] 5.2 Footer status pill reads "● connected" (green), "○ stale" (amber, no heartbeat in 30 s), or "○ disconnected" (red)
- [ ] 5.3 Add `Authorization: Bearer ${apiKey}` to every fetch in `gui/src/lib/api.ts`; throw `ApiError(401)` when key absent
- [ ] 5.4 Add `gui/src/shell/ApiKeyModal.tsx` shown when any 401 is observed; persists to `localStorage["cortex.api_key"]`
- [ ] 5.5 Modal explains how to mint a key (`cortex-api admin issue-api-key --scope dashboard`) — exact command in the body

## 6. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 6.1 Update or create documentation covering the implementation — extend `gui/README.md` with an "Auth" sub-section (mint command, modal flow, env-var override `CORTEX_API_KEY`); update `docs/specs/16-dashboard.md` Auth section to 🟢
- [ ] 6.2 Write tests covering the new behavior — Rust integration test for SSE round-trip (subscribe, push 3 envelopes via lane, assert 3 SSE events received in order); test for `Last-Event-ID` replay; test for 401 without header; test for backpressure dropping a slow subscriber; Vitest for `useSSE` reconnect ladder; RTL for the ApiKeyModal flow
- [ ] 6.3 Run tests and confirm they pass — `cargo test -p cortex-api`, `cargo clippy -p cortex-api --all-targets -- -D warnings`, `pnpm test`, `pnpm exec tsc --noEmit -p tsconfig.json`
