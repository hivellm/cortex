# Proposal: phase3_gui_multi_connection

## Why

The GUI today hard-codes a single Cortex backend at
`http://127.0.0.1:15011` (in [gui/src/lib/api.ts](../../../gui/src/lib/api.ts)).
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

Add a first-class "connections" surface to the GUI:

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
  `http://127.0.0.1:15011`, non-removable, always present.
- CORS / `cortex-api` side: document the allowed-origin list and
  the `Authorization: Bearer …` flow; if the API needs config
  changes for browser CORS, surface them as a follow-up note.

Search-friendly summary: "manage multiple Cortex backends from one
dashboard, switch between them in one click, ship to a server."

## Impact

- Affected specs: 16 (dashboard) — add a §Connections subsection
  describing the store + UI; possibly 11 (query API) if CORS rules
  need a knob.
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
- Breaking change: NO for the user-facing data model (existing
  installs migrate to a single `local` connection on first launch).
  Possibly YES for the renderer's internal `apiClient` shape — all
  fetcher signatures take an explicit `connection` argument or come
  from a context hook.
- User benefit: Cortex GUI becomes deployable on a server and
  usable from any browser / Electron client; teams can compare
  multiple Cortex instances side-by-side; "ship Cortex to a
  server" stops needing a per-user code edit.

Source: docs/specs/16-dashboard.md (will get a §Connections
addendum once design lands).
