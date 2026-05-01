/**
 * Connection store + React context — phase3 §1.3 + §1.5.
 *
 * Mirrors the existing `FiltersContext` shape in `gui/src/lib/filters.ts`
 * so the renderer keeps a single state-management style — no Zustand,
 * no Redux, no new dependency. State lives in a useReducer; CRUD
 * operations dispatch typed actions; persistence runs as a debounced
 * effect so every mutation lands on disk within ~300ms.
 *
 * The reducer is the source of truth for the local connection's
 * non-removability rule (§1.5 + spec scenario "Cannot delete active
 * connection"). Views call `removeConnection(id)` and the reducer
 * silently no-ops when `id === LOCAL_CONNECTION_ID` or when `id`
 * matches `activeId` — see the action handlers below for the exact
 * guards.
 */

import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useReducer,
  useRef,
  useState,
  type ReactNode,
} from "react";

import {
  type Connection,
  type ConnectionAuth,
  type ConnectionId,
  type ConnectionsState,
  type ConnectionHealth,
  emptyConnectionsState,
  LOCAL_CONNECTION_ID,
} from "./types";
import {
  forgetSecret,
  loadConnections,
  saveConnections,
  tokensStoredInPlaintext,
  type LoadOutcome,
} from "./persistence";

type ConnectionDraft = {
  label: string;
  baseUrl: string;
  auth: ConnectionAuth;
  color?: string;
};

type Action =
  | { type: "hydrate"; payload: ConnectionsState }
  | { type: "add"; payload: { id: ConnectionId; draft: ConnectionDraft } }
  | {
      type: "update";
      payload: { id: ConnectionId; patch: Partial<ConnectionDraft> };
    }
  | { type: "duplicate"; payload: { sourceId: ConnectionId; newId: ConnectionId } }
  | { type: "remove"; payload: { id: ConnectionId } }
  | { type: "setActive"; payload: { id: ConnectionId } }
  | {
      type: "setHealth";
      payload: { id: ConnectionId; health: ConnectionHealth };
    };

function reducer(state: ConnectionsState, action: Action): ConnectionsState {
  switch (action.type) {
    case "hydrate":
      return action.payload;
    case "add": {
      const { id, draft } = action.payload;
      const conn: Connection = {
        id,
        label: draft.label.trim() || "Unnamed",
        baseUrl: draft.baseUrl.trim().replace(/\/+$/, ""),
        auth: draft.auth,
        color: draft.color ?? "#3b82f6",
        createdAt: Date.now(),
      };
      return { ...state, connections: [...state.connections, conn] };
    }
    case "update": {
      const { id, patch } = action.payload;
      // The local connection's identity (id, baseUrl) is locked. The
      // user may relabel it (read-only context for now) — but URL +
      // auth stay anchored to localhost so a renderer reload always
      // recovers a working backend.
      const isLocal = id === LOCAL_CONNECTION_ID;
      return {
        ...state,
        connections: state.connections.map((c) =>
          c.id !== id
            ? c
            : {
                ...c,
                label: patch.label?.trim() || c.label,
                baseUrl: isLocal
                  ? c.baseUrl
                  : patch.baseUrl?.trim().replace(/\/+$/, "") ?? c.baseUrl,
                auth: isLocal ? c.auth : patch.auth ?? c.auth,
                color: patch.color ?? c.color,
              },
        ),
      };
    }
    case "duplicate": {
      const { sourceId, newId } = action.payload;
      const src = state.connections.find((c) => c.id === sourceId);
      if (!src) return state;
      const clone: Connection = {
        ...src,
        id: newId,
        label: `${src.label} (copy)`,
        createdAt: Date.now(),
        auth:
          src.auth.kind === "bearer"
            ? { kind: "bearer", token: "" }
            : src.auth.kind === "basic"
              ? { kind: "basic", username: src.auth.username, password: "" }
              : { kind: "none" },
      };
      return { ...state, connections: [...state.connections, clone] };
    }
    case "remove": {
      const { id } = action.payload;
      // Guard 1 — local connection is non-removable (§1.5).
      if (id === LOCAL_CONNECTION_ID) return state;
      // Guard 2 — cannot delete the active connection (spec scenario
      // "Cannot delete active connection"). Caller switches first.
      if (id === state.activeId) return state;
      const next = state.connections.filter((c) => c.id !== id);
      // Defensive — never empty the list.
      if (next.length === 0) return state;
      return { ...state, connections: next };
    }
    case "setActive": {
      const { id } = action.payload;
      if (!state.connections.some((c) => c.id === id)) return state;
      return { ...state, activeId: id };
    }
    case "setHealth": {
      const { id, health } = action.payload;
      return {
        ...state,
        connections: state.connections.map((c) =>
          c.id === id ? { ...c, health } : c,
        ),
      };
    }
    default:
      return state;
  }
}

export interface ConnectionsContextValue {
  state: ConnectionsState;
  active: Connection;
  source: LoadOutcome["source"];
  warnings: string[];
  tokensInPlaintext: boolean;
  addConnection: (draft: ConnectionDraft) => Connection;
  updateConnection: (id: ConnectionId, patch: Partial<ConnectionDraft>) => void;
  duplicateConnection: (id: ConnectionId) => Connection | undefined;
  removeConnection: (id: ConnectionId) => Promise<void>;
  setActiveConnection: (id: ConnectionId) => void;
  setHealth: (id: ConnectionId, health: ConnectionHealth) => void;
}

const ConnectionsContext = createContext<ConnectionsContextValue | undefined>(
  undefined,
);

/** ULID-shaped identifier (Crockford base32, 26 chars). The renderer
 * uses `crypto.randomUUID()` when available and falls back to a
 * timestamp+random hex pair on older runtimes. ULID would be nicer
 * but we keep dep tree clean — the IDs only need to be unique per
 * connection set, never compared chronologically by users. */
function newConnectionId(): ConnectionId {
  if (typeof crypto !== "undefined" && typeof crypto.randomUUID === "function") {
    return crypto.randomUUID();
  }
  return `conn-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 10)}`;
}

export function ConnectionsProvider({ children }: { children: ReactNode }) {
  const [state, dispatch] = useReducer(reducer, undefined, emptyConnectionsState);
  const [source, setSource] = useState<LoadOutcome["source"]>("fresh");
  const [warnings, setWarnings] = useState<string[]>([]);
  const [hydrated, setHydrated] = useState(false);
  const debounceRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  // Boot — hydrate from disk once.
  useEffect(() => {
    let cancelled = false;
    void (async () => {
      const outcome = await loadConnections();
      if (cancelled) return;
      dispatch({ type: "hydrate", payload: outcome.state });
      setSource(outcome.source);
      setWarnings(outcome.warnings);
      setHydrated(true);
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  // Persist — debounced 300ms after every mutation, post-hydration.
  useEffect(() => {
    if (!hydrated) return;
    if (debounceRef.current) clearTimeout(debounceRef.current);
    debounceRef.current = setTimeout(() => {
      void saveConnections(state);
    }, 300);
    return () => {
      if (debounceRef.current) clearTimeout(debounceRef.current);
    };
  }, [state, hydrated]);

  const active = useMemo(() => {
    return (
      state.connections.find((c) => c.id === state.activeId) ??
      state.connections[0]
    );
  }, [state]);

  const addConnection = useCallback(
    (draft: ConnectionDraft): Connection => {
      const id = newConnectionId();
      dispatch({ type: "add", payload: { id, draft } });
      // Caller often wants the freshly-minted connection back to
      // route the user into edit mode. Reconstruct the same record
      // the reducer assembles so we don't need to re-read state on
      // the next render.
      return {
        id,
        label: draft.label.trim() || "Unnamed",
        baseUrl: draft.baseUrl.trim().replace(/\/+$/, ""),
        auth: draft.auth,
        color: draft.color ?? "#3b82f6",
        createdAt: Date.now(),
      };
    },
    [],
  );

  const updateConnection = useCallback(
    (id: ConnectionId, patch: Partial<ConnectionDraft>) => {
      dispatch({ type: "update", payload: { id, patch } });
    },
    [],
  );

  const duplicateConnection = useCallback(
    (id: ConnectionId) => {
      const src = state.connections.find((c) => c.id === id);
      if (!src) return undefined;
      const newId = newConnectionId();
      dispatch({ type: "duplicate", payload: { sourceId: id, newId } });
      return { ...src, id: newId };
    },
    [state],
  );

  const removeConnection = useCallback(async (id: ConnectionId) => {
    dispatch({ type: "remove", payload: { id } });
    if (id !== LOCAL_CONNECTION_ID) {
      await forgetSecret(id);
    }
  }, []);

  const setActiveConnection = useCallback((id: ConnectionId) => {
    dispatch({ type: "setActive", payload: { id } });
  }, []);

  const setHealth = useCallback((id: ConnectionId, health: ConnectionHealth) => {
    dispatch({ type: "setHealth", payload: { id, health } });
  }, []);

  const value: ConnectionsContextValue = useMemo(
    () => ({
      state,
      active,
      source,
      warnings,
      tokensInPlaintext: tokensStoredInPlaintext(),
      addConnection,
      updateConnection,
      duplicateConnection,
      removeConnection,
      setActiveConnection,
      setHealth,
    }),
    [
      state,
      active,
      source,
      warnings,
      addConnection,
      updateConnection,
      duplicateConnection,
      removeConnection,
      setActiveConnection,
      setHealth,
    ],
  );

  return (
    <ConnectionsContext.Provider value={value}>
      {children}
    </ConnectionsContext.Provider>
  );
}

/** Hook for components that need the full store — manage view, header
 * switcher, API layer factory. Throws when no provider is mounted
 * because every CRUD path actually needs the dispatcher; fixtures
 * that don't care should reach for `useActiveConnection` instead. */
export function useConnections(): ConnectionsContextValue {
  const ctx = useContext(ConnectionsContext);
  if (!ctx) {
    throw new Error(
      "useConnections must be called inside a <ConnectionsProvider>",
    );
  }
  return ctx;
}

/** Lightweight hook for components that only need the active
 * connection (most fetchers). Avoids subscribing the component to
 * unrelated CRUD changes. Falls back to a synthetic local
 * connection when no provider is mounted so the test harness can
 * render individual views without wrapping every fixture in a
 * provider — the fallback mirrors api.ts's default resolver, so
 * fetchers and queryKeys agree on the fallback id. */
export function useActiveConnection(): Connection {
  const ctx = useContext(ConnectionsContext);
  if (ctx) return ctx.active;
  return {
    id: LOCAL_CONNECTION_ID,
    label: "Local",
    baseUrl: "http://127.0.0.1:17000",
    auth: { kind: "none" },
    color: "#22c55e",
    createdAt: 0,
  };
}

export { LOCAL_CONNECTION_ID } from "./types";
