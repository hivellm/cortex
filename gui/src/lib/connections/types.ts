/**
 * Connection model — phase3_gui_multi_connection §1.1.
 *
 * The renderer points at one or more Cortex backends. Each backend
 * lives behind a `Connection` record persisted across sessions. The
 * built-in `local` connection (id `"local"`, base URL
 * `http://127.0.0.1:17000`, auth `none`) ships seeded on first
 * launch and is non-removable.
 *
 * Auth lives INSIDE the connection — not as a login gate on the
 * dashboard. Localhost stays unauthenticated by default; remote
 * deployments opt in via `CORTEX_DASHBOARD_AUTH=1` on the daemon
 * side and a bearer token on the matching Connection.
 */

/** Unique connection identifier. The built-in local backend uses
 * the literal `"local"`. User-added connections use a ULID-style
 * identifier minted at creation time. */
export type ConnectionId = string;

/** Authentication shape attached to a Connection.
 *
 * - `none` — no `Authorization` header sent (default for localhost).
 * - `bearer` — adds `Authorization: Bearer <token>` to every request;
 *   for SSE, the token is also passed via the `?api_key=…` URL
 *   query-param escape hatch the daemon honours when the
 *   `EventSource` API can't carry custom headers.
 * - `basic` — adds `Authorization: Basic base64(user:password)`. Kept
 *   minimal for v1; mTLS lands later.
 */
export type ConnectionAuth =
  | { kind: "none" }
  | { kind: "bearer"; token: string }
  | { kind: "basic"; username: string; password: string };

/** Live health snapshot for a connection. Refreshed by a debounced
 * 30-second probe per connection; absent means "never probed". */
export type ConnectionHealth =
  | { state: "unknown" }
  | { state: "ok"; checkedAt: number; latencyMs: number }
  | { state: "down"; checkedAt: number; reason: string };

/** Persistent connection record. Shape stable across sessions. */
export interface Connection {
  /** Stable identifier. `"local"` for the built-in connection. */
  id: ConnectionId;
  /** Human-readable label shown in the header switcher. */
  label: string;
  /** Base URL of the cortex-api daemon, no trailing slash. */
  baseUrl: string;
  /** Authentication payload. Defaults to `{ kind: "none" }`. */
  auth: ConnectionAuth;
  /** CSS color for the dot-indicator in the header. Hex string
   * including the leading `#`. */
  color: string;
  /** Unix epoch milliseconds at creation. */
  createdAt: number;
  /** Volatile — never persisted. Last health probe outcome. */
  health?: ConnectionHealth;
}

/** Persisted root document shape. The renderer writes this to
 * `userData/connections.json` (Electron) or
 * `localStorage["cortex.connections"]` (browser fallback). */
export interface ConnectionsState {
  /** Stable insertion order — UI renders connections in array
   * order. The built-in `local` is always at index 0. */
  connections: Connection[];
  /** Identifier of the active connection. Always references one of
   * `connections[].id`. */
  activeId: ConnectionId;
}

/** Stable identifier for the built-in connection. Code paths that
 * need to refuse deletion key off this constant. */
export const LOCAL_CONNECTION_ID: ConnectionId = "local";

/** Default base URL for the local connection. Mirrors
 * `gui/src/lib/api.ts`'s previous hard-coded value so existing
 * installs migrate seamlessly. */
export const LOCAL_BASE_URL = "http://127.0.0.1:17000";

/** Factory for the built-in local connection. Idempotent — each
 * call returns a fresh object so callers can mutate safely. */
export function buildLocalConnection(): Connection {
  return {
    id: LOCAL_CONNECTION_ID,
    label: "Local",
    baseUrl: LOCAL_BASE_URL,
    auth: { kind: "none" },
    color: "#22c55e",
    createdAt: 0,
  };
}

/** Factory for the empty persisted state. Used when persistence
 * layer reports no prior `connections.json`. */
export function emptyConnectionsState(): ConnectionsState {
  const local = buildLocalConnection();
  return {
    connections: [local],
    activeId: local.id,
  };
}
