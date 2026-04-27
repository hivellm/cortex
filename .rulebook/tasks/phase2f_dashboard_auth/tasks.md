## 1. Backend — auth middleware + storage
- [ ] 1.1 `crates/cortex-api/src/auth.rs` — `require_api_key` Axum middleware. Reads `Authorization: Bearer …` (and the `?api_key=…` query-param escape hatch the SSE endpoint needs). Compares against the SQLite-stored Argon2id hashes via `subtle::ConstantTimeEq`.
- [ ] 1.2 `crates/cortex-api/src/storage/api_keys.rs` — SQLite-backed `api_keys (id, scope, label, hash, created_at, last_used_at)` table managed via `sqlx`. Migration on first boot.
- [ ] 1.3 Wire `Router::layer(from_fn(require_api_key))` onto the dashboard sub-router only — `/v1/status` stays anonymous so liveness probes from operators / load balancers don't need a key.
- [ ] 1.4 401 body: `{ "reason": "missing_or_invalid_api_key" }` — matches the existing `/v1/query` error shape.
- [ ] 1.5 Cold-stack auto-seed: when `CORTEX_DEV_AUTOSEED_KEY=1` (default in `.env`) and the `api_keys` table is empty, mint a `cortex_dev_localhost` key and write it to `~/.cortex/dev-key.txt`. Renderer reads from there as a last-resort fallback.

## 2. Admin sub-commands
- [ ] 2.1 `cortex-api admin issue-api-key --scope dashboard [--label <name>]` via `clap` — generates a 32-byte `OsRng` key, encodes as `cortex_dash_<base32>`, prints cleartext once, persists Argon2id hash + metadata.
- [ ] 2.2 `cortex-api admin list-api-keys` — prints `id / scope / label / created_at / last_used_at`. Hashes never printed.
- [ ] 2.3 `cortex-api admin revoke-api-key <id>` — soft-deletes (sets `revoked_at`); the middleware checks the column on every request.

## 3. Frontend — auth wiring
- [ ] 3.1 `gui/src/lib/useApiKey.ts` — `useApiKey()` hook around `localStorage["cortex.api_key"]` with the `import.meta.env.CORTEX_API_KEY` fallback for the dev `.env`.
- [ ] 3.2 `gui/src/lib/api.ts` — every `fetch` adds `Authorization: Bearer ${apiKey}`; throws `ApiError(401)` when the key is absent so the modal pops immediately rather than after the first user action.
- [ ] 3.3 `gui/src/lib/useSSE.ts` — `EventSource` has no header API, so the URL gets an extra `?api_key=…` param (the backend's `require_api_key` middleware accepts either source).
- [ ] 3.4 `gui/src/shell/ApiKeyModal.tsx` — shown when any 401 is observed. Body lists the `cortex-api admin issue-api-key --scope dashboard` command, the resulting `cortex_dash_…` shape, and a paste field that writes to `localStorage` on submit. Backdrop click and ESC do NOT close — the user has to enter a key or close the renderer.
- [ ] 3.5 Header status pill flips to "auth required" (amber) while the modal is open.

## 4. Live verification
- [ ] 4.1 Mint a key via `cortex-api admin issue-api-key --scope dashboard --label local-gui`; copy the cleartext.
- [ ] 4.2 Reload the GUI — the modal pops; paste the key; renderer reconnects with all 12 dashboard endpoints + the SSE stream returning data.
- [ ] 4.3 `curl -H "Authorization: Bearer <key>" http://127.0.0.1:15011/v1/dashboard/overview` returns 200; without the header returns 401 with the spec-shaped body.
- [ ] 4.4 `curl -H "Authorization: Bearer <revoked-key>"` after `revoke-api-key` returns 401.

## 5. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 5.1 Update or create documentation covering the implementation — `docs/specs/16-dashboard.md` §Auth flips to 🟢 with the actual command surface; `gui/README.md` gains an Auth sub-section listing the mint command, the modal flow, and the `CORTEX_API_KEY` env-var override.
- [ ] 5.2 Write tests covering the new behavior — Rust integration test for 401 without header, 200 with valid header, 401 with revoked key, constant-time comparison non-regression; `useApiKey` round-trip through localStorage; ApiKeyModal RTL flow (paste → submit → renderer reconnects).
- [ ] 5.3 Run tests and confirm they pass — `cargo test -p cortex-api`, `cargo clippy -p cortex-api --all-targets -- -D warnings`, `pnpm typecheck`.
