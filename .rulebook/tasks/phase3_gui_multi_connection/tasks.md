## 1. Connection store + persistence
- [ ] 1.1 Define `Connection` type (id, label, baseUrl, auth: bearer|basic|none, color, createdAt) under `gui/src/lib/connections/types.ts`
- [ ] 1.2 Add schema validation (zod or hand-rolled) for safe load/save round-trip
- [ ] 1.3 Implement `connectionsStore` (Zustand or context) with CRUD + `activeConnectionId`
- [ ] 1.4 Persist non-secret fields to `userData/connections.json` via Electron IPC; tokens via `safeStorage` when available
- [ ] 1.5 Seed a non-removable built-in `local` connection on first launch

## 2. Electron IPC bridge
- [ ] 2.1 Add `connections.read` / `connections.write` IPC channels in electron/main.ts
- [ ] 2.2 Add `secret.read` / `secret.write` channels using `safeStorage`
- [ ] 2.3 Expose via preload as `window.cortex.connections.*`
- [ ] 2.4 Browser-only fallback to `localStorage` with a plaintext-token warning

## 3. API layer rewrite
- [ ] 3.1 Replace static `BASE_URL` in gui/src/lib/api.ts with per-call `Connection`
- [ ] 3.2 `apiFor(connection)` factory returning typed fetchers bound to that backend
- [ ] 3.3 Inject `Authorization: Bearer <token>` when the connection carries one
- [ ] 3.4 Single `useApi()` hook reading the active connection from the store

## 4. React Query scoping
- [ ] 4.1 Prefix every query key with `connection.id`
- [ ] 4.2 No manual invalidate on switch — new key pulls fresh data
- [ ] 4.3 GC cached data older than 5 min for non-active connections

## 5. Header switcher
- [ ] 5.1 Active-connection chip in Header.tsx (label + color dot + status)
- [ ] 5.2 Dropdown with all known connections + "Manage..." entry
- [ ] 5.3 Status dot from a debounced 30s probe per connection

## 6. Manage view + ApiKeyPrompt
- [ ] 6.1 New route `/connections` (views/Connections.tsx)
- [ ] 6.2 Table with edit / duplicate / remove (local non-removable)
- [ ] 6.3 Form: label, base URL, auth selector, color picker
- [ ] 6.4 Test button against `/v1/dashboard/status`
- [ ] 6.5 Delete confirms; can't delete the active connection without switching first
- [ ] 6.6 `gui/src/lib/useApiKey.ts` — typed hook around the active connection's `auth.token` w/ `import.meta.env.CORTEX_API_KEY` fallback for dev `.env`
- [ ] 6.7 `gui/src/shell/ApiKeyPrompt.tsx` — modal triggered when any fetcher observes a 401 from a non-localhost connection; paste-field writes onto the active Connection's `auth` field; ESC + backdrop click do not close (operator pastes a key or switches connection)
- [ ] 6.8 Header status pill flips to "auth required" (amber) while the prompt is open

## 7. cortex-api side — opt-in auth + CORS
- [ ] 7.1 Audit CORS for `/v1/*` against non-Electron browser origins
- [ ] 7.2 `crates/cortex-api/src/storage/api_keys.rs` — SQLite-backed `api_keys (id, scope, label, hash, created_at, last_used_at, revoked_at)` table managed via `sqlx`; migration runs on first boot when `CORTEX_DASHBOARD_AUTH=1`
- [ ] 7.3 `crates/cortex-api/src/auth.rs` — `require_api_key` Axum middleware: reads `Authorization: Bearer …` plus the `?api_key=…` query-param escape hatch for SSE; constant-time compare via `subtle::ConstantTimeEq`; checks `revoked_at` on every request
- [ ] 7.4 Wire `Router::layer(from_fn(require_api_key))` onto the dashboard sub-router only when `CORTEX_DASHBOARD_AUTH=1`; `/v1/status` stays anonymous so liveness probes do not need a key
- [ ] 7.5 401 body: `{ "reason": "missing_or_invalid_api_key" }` (matches existing `/v1/query` error shape)
- [ ] 7.6 `cortex-api admin issue-api-key --scope dashboard [--label <name>]` via `clap` — generates a 32-byte `OsRng` key, encodes as `cortex_dash_<base32>`, prints cleartext once, persists Argon2id hash + metadata
- [ ] 7.7 `cortex-api admin list-api-keys` — prints `id / scope / label / created_at / last_used_at / revoked_at`; hashes never printed
- [ ] 7.8 `cortex-api admin revoke-api-key <id>` — sets `revoked_at`; middleware blocks the key on the next request
- [ ] 7.9 Document bearer-token contract in `docs/specs/16-dashboard.md` §Connections + flip §Auth to 🟢 (opt-in)
- [ ] 7.10 Add `CORTEX_DASHBOARD_AUTH=0` and `CORTEX_API_ALLOWED_ORIGINS` to `.env.example` with explanatory comments

## 8. Tests
- [ ] 8.1 Unit: connections store CRUD + persistence round-trip
- [ ] 8.2 Unit: `apiFor(connection)` injects correct base URL + auth
- [ ] 8.3 Unit: query-key scoping prevents cross-connection cache hits
- [ ] 8.4 E2E: add → test → switch → manage delete
- [ ] 8.5 Rust IT `crates/cortex-api/tests/auth_it.rs`: 401 without header, 200 with valid header, 401 with revoked key, constant-time compare regression guard
- [ ] 8.6 Rust IT `auth_disabled_it.rs`: when `CORTEX_DASHBOARD_AUTH=0` (default), all dashboard endpoints accept anonymous requests — no regression for localhost dev
- [ ] 8.7 RTL `ApiKeyPrompt.test.tsx`: paste → submit → fetcher reconnects with bearer; ESC + backdrop click do not close

## 9. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 9.1 Update or create documentation covering the implementation — `docs/specs/16-dashboard.md` (§Connections + §Auth flip), `gui/README.md` Auth sub-section listing the mint command + ApiKeyPrompt flow + `CORTEX_API_KEY` env override, CHANGELOG entry
- [ ] 9.2 Write tests covering the new behavior — every IT named in §8 above; coverage ≥ 95 % on new modules
- [ ] 9.3 Run tests and confirm they pass — `cargo test -p cortex-api`, `cargo clippy -p cortex-api --all-targets -- -D warnings`, `cargo fmt --check`, `pnpm typecheck`, `pnpm test`
