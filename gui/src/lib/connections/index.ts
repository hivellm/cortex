/**
 * Public surface for the connections module — phase3 §3.4.
 *
 * Importers should pull from this barrel rather than reaching into
 * the file layout. Keeps the call sites one-step removed from the
 * internal split between types / schema / persistence / store so a
 * later refactor (e.g. moving the reducer into a separate file)
 * does not ripple through the codebase.
 */

export { ConnectionsProvider, useConnections, useActiveConnection } from "./store";
export type { ConnectionsContextValue } from "./store";
export {
  buildLocalConnection,
  emptyConnectionsState,
  LOCAL_BASE_URL,
  LOCAL_CONNECTION_ID,
} from "./types";
export type {
  Connection,
  ConnectionAuth,
  ConnectionHealth,
  ConnectionId,
  ConnectionsState,
} from "./types";
export { tokensStoredInPlaintext } from "./persistence";
