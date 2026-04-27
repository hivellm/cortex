# Proposal: phase2f_dashboard_auth

## Why

`/v1/dashboard/*` and the new `/v1/dashboard/timeline/stream` SSE endpoint (shipped under `phase2f_dashboard_sse_stream_and_auth`) are unauthenticated. Spec 16 §Auth + `phase2_dashboard` §2 require an `Authorization: Bearer <api_key>` middleware before the dashboard can be safely exposed beyond `127.0.0.1`. The carve-out from the parent SSE task: the auth piece is a breaking change that locks every running renderer out mid-session unless paired with an `ApiKeyModal` flow; shipping it independently keeps the SSE roll-out non-breaking.

The auth surface needs SQLite + sqlx + Argon2id, an admin sub-command, and a frontend modal — substantial scope on its own.

## What Changes

### Backend
- `crates/cortex-api/src/auth.rs` — `require_api_key` Axum middleware, applied via `Router::layer(from_fn(require_api_key))` to the dashboard sub-router only (skip `/v1/status` so liveness probes stay anonymous).
- `crates/cortex-api/src/storage/api_keys.rs` — SQLite table `api_keys (id, scope, hash, created_at, last_used_at)` migrated via `sqlx`. Hashes at rest with Argon2id.
- 401 body matches the existing `/v1/query` shape: `{ "reason": "missing_or_invalid_api_key" }`.
- Constant-time comparison via `subtle::ConstantTimeEq`.

### Admin sub-commands
- `cortex-api admin issue-api-key --scope dashboard [--label <name>]`
- `cortex-api admin list-api-keys`
- `cortex-api admin revoke-api-key <id>`
- 32-byte random key via `rand::rngs::OsRng`, encoded as `cortex_dash_<base32>`. Cleartext printed once; only the Argon2id hash persists.

### Frontend
- `Authorization: Bearer ${apiKey}` on every fetch in `gui/src/lib/api.ts`. `EventSource` doesn't accept custom headers, so the SSE URL gets a `?api_key=...` query-param escape hatch (the backend honours either source).
- `gui/src/shell/ApiKeyModal.tsx` — shown when any 401 is observed; persists to `localStorage["cortex.api_key"]`. Body explains `cortex-api admin issue-api-key --scope dashboard`.
- `gui/src/lib/useApiKey.ts` — typed hook around the localStorage entry plus a `CORTEX_API_KEY` env-var fallback for the operator's `.env`.

## Impact

- Affected specs: `docs/specs/16-dashboard.md` (§Auth flips to 🟢); `phase2_dashboard` §2 closes.
- Affected code: `crates/cortex-api/src/auth.rs` (new), `crates/cortex-api/src/storage/api_keys.rs` (new), `crates/cortex-api/src/main.rs` (admin sub-command wiring), `gui/src/lib/api.ts`, `gui/src/lib/useApiKey.ts` (new), `gui/src/shell/ApiKeyModal.tsx` (new).
- Breaking change: YES — every existing dashboard request now requires `Authorization`. The `gui/README.md` Auth sub-section explains how to mint and configure the key. Cold-stack dev keeps a `cortex_dev_localhost` key seeded automatically when `CORTEX_DEV_AUTOSEED_KEY=1` (default in `.env`).
- Depends on: `phase2f_dashboard_sse_stream_and_auth` SSE half (shipped).
- User benefit: dashboard becomes safe to expose on a multi-user host or behind a reverse proxy.
