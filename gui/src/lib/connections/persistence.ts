/**
 * Persistence adapter — phase3_gui_multi_connection §1.4.
 *
 * Two backends:
 *
 * - **Electron** — writes `connections.json` under `app.getPath("userData")`
 *   via the `cortex.connections.{read,write}` IPC bridge declared in
 *   `gui/electron/preload.ts`. Bearer tokens are kept *out* of the
 *   JSON document and stored separately in the OS keychain via
 *   Electron's `safeStorage` (`cortex.secret.{read,write}`); the
 *   document records `auth.kind=bearer` with `token=""` and the
 *   adapter rehydrates the token at read time.
 *
 * - **Browser fallback** — writes to `localStorage["cortex.connections"]`
 *   in plaintext. The manage view surfaces a banner warning the user
 *   that tokens are not protected by an OS keychain when this path
 *   is taken.
 *
 * The adapter exposes a single async `loadConnections()` /
 * `saveConnections(state)` pair. The store layer in `store.tsx` is
 * the only caller; views read the store, never the adapter directly.
 */

import {
  type ConnectionsState,
  type Connection,
  emptyConnectionsState,
  LOCAL_CONNECTION_ID,
} from "./types";
import { validateConnectionsState } from "./schema";

const LS_KEY = "cortex.connections";

/** Discriminator returned alongside the loaded state so the manage
 * view can surface the "tokens stored in plaintext" banner only on
 * the browser fallback path. */
export type LoadOutcome = {
  state: ConnectionsState;
  source: "electron" | "browser-fallback" | "fresh";
  warnings: string[];
};

interface CortexBridge {
  connections?: {
    read: () => Promise<unknown>;
    write: (raw: unknown) => Promise<void>;
  };
  secret?: {
    read: (key: string) => Promise<string | null>;
    write: (key: string, value: string) => Promise<void>;
    remove: (key: string) => Promise<void>;
  };
}

function bridge(): CortexBridge | undefined {
  if (typeof window === "undefined") return undefined;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const w = window as any;
  return w.cortex as CortexBridge | undefined;
}

function hasElectronConnections(b: CortexBridge | undefined): b is CortexBridge & {
  connections: NonNullable<CortexBridge["connections"]>;
} {
  return !!b && !!b.connections;
}

function hasElectronSecrets(b: CortexBridge | undefined): b is CortexBridge & {
  secret: NonNullable<CortexBridge["secret"]>;
} {
  return !!b && !!b.secret;
}

/** Strip volatile + secret fields before serialising. The persisted
 * shape SHALL NOT carry health snapshots (refreshed at boot) and
 * SHALL NOT carry bearer tokens when the OS keychain is available
 * — those round-trip through `secret.{read,write}` instead. */
function projectForPersistence(
  state: ConnectionsState,
  storeTokensInline: boolean,
): ConnectionsState {
  const connections = state.connections.map<Connection>((c) => {
    const out: Connection = {
      id: c.id,
      label: c.label,
      baseUrl: c.baseUrl,
      auth: c.auth,
      color: c.color,
      createdAt: c.createdAt,
    };
    // Strip secrets when keychain is the storage path. Tokens land
    // on disk via secret.write under the same connection id.
    if (!storeTokensInline && c.auth.kind === "bearer") {
      out.auth = { kind: "bearer", token: "" };
    }
    if (!storeTokensInline && c.auth.kind === "basic") {
      out.auth = { kind: "basic", username: c.auth.username, password: "" };
    }
    return out;
  });
  return { connections, activeId: state.activeId };
}

async function rehydrateSecrets(
  state: ConnectionsState,
  b: CortexBridge,
): Promise<ConnectionsState> {
  if (!hasElectronSecrets(b)) return state;
  const connections = await Promise.all(
    state.connections.map<Promise<Connection>>(async (c) => {
      if (c.auth.kind === "bearer") {
        const token = (await b.secret.read(`bearer:${c.id}`)) ?? "";
        return { ...c, auth: { kind: "bearer", token } };
      }
      if (c.auth.kind === "basic") {
        const password = (await b.secret.read(`basic:${c.id}`)) ?? "";
        return { ...c, auth: { kind: "basic", username: c.auth.username, password } };
      }
      return c;
    }),
  );
  return { connections, activeId: state.activeId };
}

async function persistSecrets(state: ConnectionsState, b: CortexBridge): Promise<void> {
  if (!hasElectronSecrets(b)) return;
  await Promise.all(
    state.connections.map(async (c) => {
      if (c.auth.kind === "bearer" && c.auth.token) {
        await b.secret.write(`bearer:${c.id}`, c.auth.token);
      } else if (c.auth.kind === "basic" && c.auth.password) {
        await b.secret.write(`basic:${c.id}`, c.auth.password);
      } else {
        // No-op when the connection has no secret to store. Removal
        // happens via `removeConnection` which calls secret.remove
        // explicitly so we avoid surprise deletions here.
      }
    }),
  );
}

export async function loadConnections(): Promise<LoadOutcome> {
  const b = bridge();
  const warnings: string[] = [];

  if (hasElectronConnections(b)) {
    try {
      const raw = await b.connections.read();
      if (raw === null || raw === undefined) {
        return {
          state: emptyConnectionsState(),
          source: "fresh",
          warnings,
        };
      }
      const result = validateConnectionsState(raw);
      if (!result.ok) {
        warnings.push(...result.issues);
        return { state: emptyConnectionsState(), source: "fresh", warnings };
      }
      const rehydrated = await rehydrateSecrets(result.value, b);
      return { state: rehydrated, source: "electron", warnings };
    } catch (err) {
      warnings.push(`electron load failed: ${(err as Error).message}`);
      return { state: emptyConnectionsState(), source: "fresh", warnings };
    }
  }

  if (typeof localStorage === "undefined") {
    return { state: emptyConnectionsState(), source: "fresh", warnings };
  }
  const raw = localStorage.getItem(LS_KEY);
  if (!raw) {
    return { state: emptyConnectionsState(), source: "fresh", warnings };
  }
  try {
    const parsed: unknown = JSON.parse(raw);
    const result = validateConnectionsState(parsed);
    if (!result.ok) {
      warnings.push(...result.issues);
      return { state: emptyConnectionsState(), source: "fresh", warnings };
    }
    return { state: result.value, source: "browser-fallback", warnings };
  } catch (err) {
    warnings.push(`localStorage parse failed: ${(err as Error).message}`);
    return { state: emptyConnectionsState(), source: "fresh", warnings };
  }
}

export async function saveConnections(state: ConnectionsState): Promise<void> {
  const b = bridge();
  if (hasElectronConnections(b)) {
    const useKeychain = hasElectronSecrets(b);
    const projected = projectForPersistence(state, !useKeychain);
    await b.connections.write(projected);
    if (useKeychain) await persistSecrets(state, b);
    return;
  }
  if (typeof localStorage === "undefined") return;
  const projected = projectForPersistence(state, /* storeTokensInline */ true);
  localStorage.setItem(LS_KEY, JSON.stringify(projected));
}

/** Best-effort secret deletion when the user removes a connection.
 * Browser fallback path is a no-op (token already lived inside the
 * persisted document). */
export async function forgetSecret(connectionId: string): Promise<void> {
  const b = bridge();
  if (!hasElectronSecrets(b)) return;
  await Promise.allSettled([
    b.secret.remove(`bearer:${connectionId}`),
    b.secret.remove(`basic:${connectionId}`),
  ]);
}

/** Whether the current persistence path stores tokens in plaintext.
 * The manage view reads this to drive the "browser fallback" warning
 * banner. Pure read; no side-effects. */
export function tokensStoredInPlaintext(): boolean {
  const b = bridge();
  if (hasElectronConnections(b) && hasElectronSecrets(b)) return false;
  return true;
}

/** Re-export for tests so they don't reach into types.ts directly. */
export { LOCAL_CONNECTION_ID };
