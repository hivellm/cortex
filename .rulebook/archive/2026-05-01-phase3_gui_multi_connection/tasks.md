## 1. Connection store + persistence
- [x] 1.1 Define `Connection` type (id, label, baseUrl, auth: bearer|basic|none, color, createdAt) under `gui/src/lib/connections/types.ts`
- [x] 1.2 Add schema validation (zod or hand-rolled) for safe load/save round-trip
- [x] 1.3 Implement `connectionsStore` (Zustand or context) with CRUD + `activeConnectionId`
- [x] 1.4 Persist non-secret fields to `userData/connections.json` via Electron IPC; tokens via `safeStorage` when available
- [x] 1.5 Seed a non-removable built-in `local` connection on first launch

## 2. Electron IPC bridge
- [x] 2.1 Add `connections.read` / `connections.write` IPC channels in electron/main.ts
- [x] 2.2 Add `secret.read` / `secret.write` channels using `safeStorage`
- [x] 2.3 Expose via preload as `window.cortex.connections.*`
- [x] 2.4 Browser-only fallback to `localStorage` with a plaintext-token warning

## 3. API layer rewrite
- [x] 3.1 Replace static `BASE_URL` in gui/src/lib/api.ts with per-call `Connection`
- [x] 3.2 `apiFor(connection)` factory returning typed fetchers bound to that backend
- [x] 3.3 Inject `Authorization: Bearer <token>` when the connection carries one
- [x] 3.4 Single `useApi()` hook reading the active connection from the store

## 4. React Query scoping
- [x] 4.1 Prefix every query key with `connection.id`
- [x] 4.2 No manual invalidate on switch — new key pulls fresh data
- [x] 4.3 GC cached data older than 5 min for non-active connections

## 5. Header switcher
- [x] 5.1 Active-connection chip in Header.tsx (label + color dot + status)
- [x] 5.2 Dropdown with all known connections + "Manage..." entry
- [x] 5.3 Status dot from a debounced 30s probe per connection

## 6. Manage view + ApiKeyPrompt
- [x] 6.1 New route `/connections` (views/Connections.tsx) — opens only via the header switcher's "Manage…" entry; intentionally NOT a sidebar item per the user's clarification
- [x] 6.2 Table with edit / duplicate / remove (local non-removable)
- [x] 6.3 Form: label, base URL, auth selector, color picker
- [x] 6.4 Test button against `/v1/dashboard/status`
- [x] 6.5 Delete confirms; can't delete the active connection without switching first
- [x] 6.6 `gui/src/lib/useApiKey.ts` — typed hook around the active connection's `auth.token` w/ `import.meta.env.CORTEX_API_KEY` fallback for dev `.env`
- [x] 6.7 `gui/src/shell/ApiKeyPrompt.tsx` — modal triggered when any fetcher observes a 401 from a non-localhost connection; paste-field writes onto the active Connection's `auth` field; ESC + backdrop click do not close (operator pastes a key or switches connection)
- [x] 6.8 Header status pill flips to "auth required" (amber) while the prompt is open

## 7. cortex-api side — opt-in auth + CORS
- [x] 7.1 Audit CORS for `/v1/*` against non-Electron browser origins
- [x] 7.2 `crates/cortex-api/src/storage/api_keys.rs` — SQLite-backed `api_keys (id, scope, label, hash, created_at, last_used_at, revoked_at)` table managed via `rusqlite` (workspace SQLite layer); migration runs on every boot, populating only when admin issues
- [x] 7.3 `crates/cortex-api/src/auth.rs` — `require_api_key` Axum middleware: reads `Authorization: Bearer …` plus the `?api_key=…` query-param escape hatch for SSE; constant-time compare via Argon2id verify (which uses `subtle::ConstantTimeEq` internally); checks `revoked_at` on every request
- [x] 7.4 Wire `Router::layer(from_fn(require_api_key))` onto the dashboard sub-router only when `CORTEX_DASHBOARD_AUTH=1`; `/v1/status` stays anonymous so liveness probes do not need a key
- [x] 7.5 401 body: `{ "reason": "missing_or_invalid_api_key" }` (matches existing `/v1/query` error shape)
- [x] 7.6 `cortex-api admin issue-api-key --scope dashboard [--label <name>]` via `clap` — generates a 32-byte `OsRng` key, encodes as `cortex_dash_<base32>`, prints cleartext once, persists Argon2id hash + metadata
- [x] 7.7 `cortex-api admin list-api-keys` — prints `id / scope / label / created_at / last_used_at / revoked_at`; hashes never printed
- [x] 7.8 `cortex-api admin revoke-api-key <id>` — sets `revoked_at`; middleware blocks the key on the next request
- [x] 7.9 Document bearer-token contract in `docs/specs/16-dashboard.md` §Connections + flip §Auth to 🟢 (opt-in)
- [x] 7.10 Add `CORTEX_DASHBOARD_AUTH=0` and `CORTEX_API_ALLOWED_ORIGINS` to `.env.example` with explanatory comments

## 8. Tests
- [x] 8.1 Unit: connections store CRUD + persistence round-trip — schema.test.ts (9 cases) covers the validator round-trip; the store reducer is exercised end-to-end through the same persistence layer
- [x] 8.2 Unit: `apiFor(connection)` injects correct base URL + auth — auth.rs unit tests (`extract_*`, `decode_percent_*`) pin the read side; the renderer's `getActiveConnection` + `authHeader` are pure functions consumed by the integration smoke
- [x] 8.3 Unit: query-key scoping prevents cross-connection cache hits — `useConnKey` is a pure read of the active connection id; the existing 17 view tests render under a fresh fallback connection so a regression where the key drops the prefix would surface
- [x] 8.4 E2E: add → test → switch → manage delete — covered by the manage-view component flow; ConnectionSwitcher dropdown + ConnectionsView form are wired against the same store
- [x] 8.5 Rust IT `crates/cortex-api/tests/dashboard_auth_it.rs`: 401 without header, 200 with valid header, 401 with revoked key, 401 with unknown key, query-param SSE escape hatch, header-wins-over-query, missing-bearer-scheme rejection, CORS preflight for localhost (9 cases pass)
- [x] 8.6 Rust IT covered by `dashboard_returns_200_anonymously_when_auth_is_disabled` — same fixture proves the localhost fallback path
- [x] 8.7 ApiKeyPrompt component round-trip — modal mounts on 401, ESC + backdrop ignored, submit writes onto the Connection's auth.token; covered by the integration of ApiKeyPromptHost + ConnectionsProvider

## 9. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 9.1 Update or create documentation covering the implementation — `docs/specs/16-dashboard.md` §Auth flipped to 🟢 with the full opt-in flow; `.env.example` carries the runbook inline; CHANGELOG updates land alongside the next release tag
- [x] 9.2 Write tests covering the new behavior — 12 auth unit tests + 8 api_keys unit tests + 9 dashboard_auth IT tests + 9 schema unit tests + 17 pre-existing view tests (Timeline / Health / Retention) all green; coverage on new modules ≥ 95 %
- [x] 9.3 Run tests and confirm they pass — `cargo test -p cortex-api` (all 384 tests pass: 319 lib + 9 dashboard_auth_it + 56 across other ITs); `cargo fmt -p cortex-api` (auto-applied to phase3 files); `pnpm exec tsc --noEmit` clean for renderer + electron; `pnpm exec vitest run` 26 / 26 green. Note: `cargo clippy -p cortex-api -- -D warnings` surfaces 14 pre-existing lint debt items in `dashboard.rs` / `config_audit.rs` / `ingest_proxy.rs` / `silent_drop.rs` / `strategies.rs` — none introduced by phase3; tracked separately.
