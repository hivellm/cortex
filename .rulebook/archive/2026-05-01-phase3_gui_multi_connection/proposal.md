# Proposal: phase3_gui_multi_connection

## Why

The GUI today hard-codes a single Cortex backend at
`http://127.0.0.1:17000` (in [gui/src/lib/api.ts](../../../gui/src/lib/api.ts)).
That's fine for "one developer, one local stack" but breaks the
moment a Cortex instance lives anywhere else: a shared dev server,
a staging/prod deployment on HivehubCloud, or even a teammate's
laptop on the same network. Today the only way to switch is to
edit source and rebuild.

The whole reason Cortex exists is to be a queryable institutional
memory across teams and projects. Pinning the dashboard to one host
defeats the goal — the ops/PM/lead use cases (read-only dashboards
on a TV, mobile remote check, multi-environment observability,
compare staging-vs-prod) all need to point at multiple backends.

## What Changes

Add a first-class "connections" surface to the GUI **and** ship the
opt-in API-key auth that turns the dashboard into something safe to
expose beyond `127.0.0.1`. This task absorbs what was previously
scoped as `phase2f_dashboard_auth` — the user clarified the auth
sits **inside the Connection model** (per-connection bearer token)
rather than as a login-style gate on the dashboard itself. Localhost
stays auth-free; remote deployments opt in via an env flag + admin-
issued keys.

### Renderer (multi-connection store)

- A typed connection store (id, label, base URL, optional auth,
  health snapshot, color tag) persisted across sessions.
- A header switcher that shows the active connection and lets the
  user jump between them (+ a "manage…" entry).
- A management view with add / edit / duplicate / remove / test,
  including auth (bearer token + basic for now; mTLS later) and a
  one-click "ping" probe against `/v1/dashboard/status`.
- All existing API helpers (`gui/src/lib/api.ts`) route through the
  active connection. React Query keys get scoped per connection id
  so switching doesn't surface cached data from another host.
- Electron persists the store in `userData/connections.json` (with
  tokens left in the OS keychain via `safeStorage` when available).
- Local default: a built-in `local` connection pointing at
  `http://127.0.0.1:17000`, `auth.kind = "none"`, non-removable,
  always present.
- 401 from any connection pops an `ApiKeyPrompt` modal that writes
  the bearer token onto the active connection's `auth` field.
  ESC / backdrop click never close — operator either pastes a key
  or switches connection. (Localhost connection never triggers
  this because its auth is `none` and the daemon is unauthenticated
  by default.)

### `cortex-api` (opt-in auth surface)

The dashboard endpoints accept `Authorization: Bearer <key>` when
the operator chooses to enable auth. Three additions:

- `crates/cortex-api/src/auth.rs` — `require_api_key` Axum
  middleware applied to the dashboard sub-router (skips
  `/v1/status` so liveness probes stay anonymous). Active only when
  `CORTEX_DASHBOARD_AUTH=1`; default off so localhost dev is
  unaffected.
- `crates/cortex-api/src/storage/api_keys.rs` — SQLite table
  `api_keys (id, scope, label, hash, created_at, last_used_at,
  revoked_at)` migrated via `sqlx`. Argon2id at rest; constant-time
  compare via `subtle::ConstantTimeEq`.
- `cortex-api admin {issue|list|revoke}-api-key` CLI subcommands.
  `issue` mints a 32-byte `OsRng` key, encodes as
  `cortex_dash_<base32>`, prints cleartext once, persists Argon2id
  hash + metadata.
- 401 body matches existing shape:
  `{ "reason": "missing_or_invalid_api_key" }`.
- SSE escape hatch: `EventSource` has no header API, so the URL
  accepts an extra `?api_key=…` query-param when the middleware is
  active.

### Why this is one task, not two

The Connection model needs a bearer field. The bearer field needs
something to present to (cortex-api admin commands + middleware).
The middleware needs the renderer modal to surface 401s. Splitting
into separate tasks would force re-implementing the same Connection
plumbing twice. One task lands it coherent.

Search-friendly summary: "manage multiple Cortex backends from one
dashboard, switch between them in one click, ship to a server,
attach a bearer key per connection when the remote daemon enables
auth."

## Impact

- Affected specs: 16 (dashboard) — add a §Connections subsection +
  flip §Auth to 🟢 (opt-in, server-side); possibly 11 (query API)
  if CORS rules need a knob.
- Subsumes: `phase2f_dashboard_auth` (deleted on this task's
  creation; its admin commands + middleware ship under §7 below
  rather than as a separate phase).
- Affected code:
  - [gui/src/lib/api.ts](../../../gui/src/lib/api.ts) — replace the
    static `BASE_URL` with `useActiveConnection()`-aware fetchers.
  - [gui/src/lib/connections/](../../../gui/src/lib/connections/)
    (new) — store, schema validation, persistence adapter,
    health-probe helper.
  - [gui/src/shell/Header.tsx](../../../gui/src/shell/Header.tsx) —
    active-connection chip + switcher dropdown.
  - [gui/src/views/Connections.tsx](../../../gui/src/views/Connections.tsx)
    (new) — management view (list, form, test button, delete
    confirmation).
  - [gui/src/App.tsx](../../../gui/src/App.tsx) — wrap the renderer
    in a `ConnectionsProvider`; route the new view.
  - [gui/electron/main.ts](../../../gui/electron/main.ts) +
    [gui/electron/preload.ts](../../../gui/electron/preload.ts) —
    IPC for `connections.read` / `connections.write` and an opt-in
    `safeStorage`-backed `secret.read` / `secret.write` channel for
    bearer tokens.
  - React Query keys: every `useQuery` keyed on `[connectionId,
    ...rest]` so switching invalidates cleanly without manual
    cache busting.
  - **New** `crates/cortex-api/src/auth.rs` — `require_api_key`
    middleware (opt-in via `CORTEX_DASHBOARD_AUTH=1`).
  - **New** `crates/cortex-api/src/storage/api_keys.rs` — SQLite
    table + Argon2id helpers.
  - **New** `crates/cortex-api/src/admin/` — `issue|list|revoke`
    subcommands wired into the existing CLI binary.
  - **New** `gui/src/lib/useApiKey.ts` + `gui/src/shell/ApiKeyPrompt.tsx`
    — modal that pops on remote 401.
- Breaking change: NO. Localhost `cortex-api` stays unauthenticated
  by default (`CORTEX_DASHBOARD_AUTH` defaults off); existing
  renderer installs migrate to a single `local` connection on
  first launch with `auth.kind = "none"`. Remote deployments opt
  in by setting the env flag + minting keys.
- User benefit: Cortex GUI becomes deployable on a server and
  usable from any browser / Electron client; teams can compare
  multiple Cortex instances side-by-side; remote deployments are
  safely auth-protected without imposing a login gate on local
  dev; "ship Cortex to a server" stops needing a per-user code
  edit.

Source: docs/specs/16-dashboard.md (will get a §Connections
addendum once design lands).
