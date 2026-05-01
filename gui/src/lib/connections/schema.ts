/**
 * Hand-rolled schema validation — phase3_gui_multi_connection §1.2.
 *
 * The renderer reads `connections.json` from disk on every launch.
 * The file may be hand-edited, corrupted, or written by an older
 * version of the GUI; we cannot trust the shape coming back from
 * persistence. `validateConnectionsState` returns either the
 * validated state or a non-empty list of human-readable issues so
 * the caller can fall back to `emptyConnectionsState()` and surface
 * an inline warning instead of crashing the renderer.
 *
 * Why no zod / valibot: phase3 keeps the renderer dep tree lean.
 * The schema is small (one record type + state envelope) and tested
 * directly; pulling in a 50 KiB validator for a single shape is
 * not worth it.
 */

import type {
  Connection,
  ConnectionAuth,
  ConnectionId,
  ConnectionsState,
} from "./types";
import { LOCAL_CONNECTION_ID } from "./types";

/** Result type for the validator. `ok=false` carries the issue list
 * so the caller can log every problem at once instead of error-by-
 * error round-trips. */
export type ValidateResult<T> =
  | { ok: true; value: T }
  | { ok: false; issues: string[] };

function isPlainObject(v: unknown): v is Record<string, unknown> {
  return typeof v === "object" && v !== null && !Array.isArray(v);
}

function isString(v: unknown): v is string {
  return typeof v === "string";
}

function isNonEmptyString(v: unknown): v is string {
  return typeof v === "string" && v.length > 0;
}

function isHttpUrl(v: unknown): v is string {
  if (!isNonEmptyString(v)) return false;
  try {
    const u = new URL(v);
    return u.protocol === "http:" || u.protocol === "https:";
  } catch {
    return false;
  }
}

function validateAuth(
  v: unknown,
  path: string,
  issues: string[],
): ConnectionAuth | undefined {
  if (!isPlainObject(v)) {
    issues.push(`${path} must be an object`);
    return undefined;
  }
  const kind = v.kind;
  if (kind === "none") {
    return { kind: "none" };
  }
  if (kind === "bearer") {
    if (!isNonEmptyString(v.token)) {
      issues.push(`${path}.token must be a non-empty string when kind=bearer`);
      return undefined;
    }
    return { kind: "bearer", token: v.token };
  }
  if (kind === "basic") {
    if (!isNonEmptyString(v.username) || !isNonEmptyString(v.password)) {
      issues.push(
        `${path}.username and ${path}.password must be non-empty when kind=basic`,
      );
      return undefined;
    }
    return { kind: "basic", username: v.username, password: v.password };
  }
  issues.push(
    `${path}.kind must be one of "none" | "bearer" | "basic"; got ${JSON.stringify(kind)}`,
  );
  return undefined;
}

function validateConnection(
  v: unknown,
  path: string,
  issues: string[],
): Connection | undefined {
  if (!isPlainObject(v)) {
    issues.push(`${path} must be an object`);
    return undefined;
  }
  const startCount = issues.length;

  if (!isNonEmptyString(v.id)) issues.push(`${path}.id must be a non-empty string`);
  if (!isNonEmptyString(v.label)) issues.push(`${path}.label must be a non-empty string`);
  if (!isHttpUrl(v.baseUrl))
    issues.push(`${path}.baseUrl must be an http:// or https:// URL`);
  if (!isString(v.color)) issues.push(`${path}.color must be a string`);
  if (typeof v.createdAt !== "number")
    issues.push(`${path}.createdAt must be a number (epoch ms)`);

  const auth = validateAuth(v.auth, `${path}.auth`, issues);

  if (issues.length !== startCount || !auth) return undefined;

  return {
    id: v.id as ConnectionId,
    label: v.label as string,
    baseUrl: (v.baseUrl as string).replace(/\/+$/, ""),
    auth,
    color: (v.color as string) || "#22c55e",
    createdAt: v.createdAt as number,
    // health is volatile — always reset on load.
  };
}

/** Validate a persisted `ConnectionsState` document. Idempotent —
 * does not mutate `input`. The local connection is enforced as the
 * first element; if missing, the caller should fall back to the
 * empty state factory. */
export function validateConnectionsState(
  input: unknown,
): ValidateResult<ConnectionsState> {
  const issues: string[] = [];
  if (!isPlainObject(input)) {
    return { ok: false, issues: ["root must be an object"] };
  }
  const rawConnections = input.connections;
  if (!Array.isArray(rawConnections)) {
    issues.push("root.connections must be an array");
    return { ok: false, issues };
  }
  if (!isNonEmptyString(input.activeId)) {
    issues.push("root.activeId must be a non-empty string");
  }

  const connections: Connection[] = [];
  const seen = new Set<string>();
  rawConnections.forEach((raw, i) => {
    const parsed = validateConnection(raw, `connections[${i}]`, issues);
    if (!parsed) return;
    if (seen.has(parsed.id)) {
      issues.push(`connections[${i}].id duplicates an earlier entry: ${parsed.id}`);
      return;
    }
    seen.add(parsed.id);
    connections.push(parsed);
  });

  if (!connections.some((c) => c.id === LOCAL_CONNECTION_ID)) {
    issues.push(
      `connections must include the built-in "${LOCAL_CONNECTION_ID}" entry`,
    );
  }

  if (issues.length > 0) {
    return { ok: false, issues };
  }

  const activeId = input.activeId as string;
  const resolvedActive = connections.some((c) => c.id === activeId)
    ? activeId
    : LOCAL_CONNECTION_ID;

  return {
    ok: true,
    value: { connections, activeId: resolvedActive },
  };
}
