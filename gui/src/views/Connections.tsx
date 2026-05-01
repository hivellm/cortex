/**
 * Connections manage view — phase3 §6.
 *
 * Lists every known connection, lets the user add/edit/duplicate/
 * remove/test entries, and surfaces the "tokens stored in
 * plaintext" warning banner when the renderer is running in a
 * browser fallback path (no Electron preload bridge).
 *
 * The local connection is always at row 1 with its remove + edit-
 * url controls disabled. Active-connection deletion is also
 * disabled — the user must switch first (per spec scenario "Cannot
 * delete active connection").
 */

import { useEffect, useMemo, useState } from "react";

import {
  type Connection,
  type ConnectionAuth,
  LOCAL_CONNECTION_ID,
  useConnections,
} from "../lib/connections";

type AuthKind = ConnectionAuth["kind"];

type DraftState = {
  id: string | null; // null = creating; otherwise editing
  label: string;
  baseUrl: string;
  authKind: AuthKind;
  bearerToken: string;
  basicUser: string;
  basicPass: string;
  color: string;
};

const EMPTY_DRAFT: DraftState = {
  id: null,
  label: "",
  baseUrl: "",
  authKind: "none",
  bearerToken: "",
  basicUser: "",
  basicPass: "",
  color: "#3b82f6",
};

function draftFromConnection(c: Connection): DraftState {
  return {
    id: c.id,
    label: c.label,
    baseUrl: c.baseUrl,
    authKind: c.auth.kind,
    bearerToken: c.auth.kind === "bearer" ? c.auth.token : "",
    basicUser: c.auth.kind === "basic" ? c.auth.username : "",
    basicPass: c.auth.kind === "basic" ? c.auth.password : "",
    color: c.color,
  };
}

function authFromDraft(d: DraftState): ConnectionAuth {
  if (d.authKind === "bearer")
    return { kind: "bearer", token: d.bearerToken.trim() };
  if (d.authKind === "basic")
    return {
      kind: "basic",
      username: d.basicUser.trim(),
      password: d.basicPass,
    };
  return { kind: "none" };
}

type TestState =
  | { state: "idle" }
  | { state: "loading" }
  | { state: "ok"; latencyMs: number; service?: string }
  | { state: "err"; status: number | null; message: string };

async function probeFromDraft(d: DraftState): Promise<TestState> {
  const startedAt = Date.now();
  try {
    const headers: Record<string, string> = { accept: "application/json" };
    const auth = authFromDraft(d);
    if (auth.kind === "bearer" && auth.token) {
      headers.authorization = `Bearer ${auth.token}`;
    } else if (auth.kind === "basic" && auth.username) {
      const raw = `${auth.username}:${auth.password}`;
      headers.authorization = `Basic ${typeof btoa === "function" ? btoa(raw) : Buffer.from(raw).toString("base64")}`;
    }
    const url = `${d.baseUrl.replace(/\/+$/, "")}/v1/dashboard/status`;
    const resp = await fetch(url, { headers });
    if (!resp.ok) {
      return {
        state: "err",
        status: resp.status,
        message: `${resp.status} ${resp.statusText}`,
      };
    }
    const body = (await resp.json()) as { service?: string };
    return {
      state: "ok",
      latencyMs: Date.now() - startedAt,
      service: body.service,
    };
  } catch (err) {
    return {
      state: "err",
      status: null,
      message: (err as Error).message ?? "fetch failed",
    };
  }
}

export function ConnectionsView() {
  const ctx = useConnections();
  const [draft, setDraft] = useState<DraftState>(EMPTY_DRAFT);
  const [test, setTest] = useState<TestState>({ state: "idle" });
  const [confirmDelete, setConfirmDelete] = useState<string | null>(null);

  // Reset the test pill when the user starts typing in the form.
  useEffect(() => {
    setTest({ state: "idle" });
  }, [draft.baseUrl, draft.authKind, draft.bearerToken, draft.basicUser, draft.basicPass]);

  const isEditing = draft.id !== null;
  const isLocalEdit = draft.id === LOCAL_CONNECTION_ID;

  const sorted = useMemo(() => {
    const local = ctx.state.connections.filter((c) => c.id === LOCAL_CONNECTION_ID);
    const rest = ctx.state.connections
      .filter((c) => c.id !== LOCAL_CONNECTION_ID)
      .sort((a, b) => a.createdAt - b.createdAt);
    return [...local, ...rest];
  }, [ctx.state.connections]);

  const onSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    if (!draft.label.trim() || !draft.baseUrl.trim()) return;
    if (isEditing && draft.id) {
      ctx.updateConnection(draft.id, {
        label: draft.label,
        baseUrl: draft.baseUrl,
        auth: authFromDraft(draft),
        color: draft.color,
      });
    } else {
      ctx.addConnection({
        label: draft.label,
        baseUrl: draft.baseUrl,
        auth: authFromDraft(draft),
        color: draft.color,
      });
    }
    setDraft(EMPTY_DRAFT);
    setTest({ state: "idle" });
  };

  return (
    <div className="connections-view">
      <h2>Connections</h2>
      {ctx.tokensInPlaintext && (
        <div className="connections-banner">
          ⚠ This renderer is running outside Electron. Bearer tokens
          are persisted in <code>localStorage</code> without OS-keychain
          protection. Use Electron desktop builds for sensitive
          deployments.
        </div>
      )}
      {ctx.warnings.length > 0 && (
        <div className="connections-banner">
          {ctx.warnings.length} issue(s) recovering connections.json — see
          DevTools console for details.
        </div>
      )}

      <table className="connections-table">
        <thead>
          <tr>
            <th>Label</th>
            <th>Base URL</th>
            <th>Auth</th>
            <th>Health</th>
            <th>Actions</th>
          </tr>
        </thead>
        <tbody>
          {sorted.map((c) => {
            const isActive = c.id === ctx.active.id;
            const isLocal = c.id === LOCAL_CONNECTION_ID;
            const canDelete = !isLocal && !isActive;
            const healthGlyph =
              c.health?.state === "ok"
                ? "ok"
                : c.health?.state === "down"
                  ? "down"
                  : "unknown";
            return (
              <tr key={c.id} className={isActive ? "is-active-row" : ""}>
                <td>
                  <span
                    className="conn-chip__dot"
                    style={{ backgroundColor: c.color, marginRight: 8 }}
                  />
                  {c.label}
                  {isActive && <span style={{ marginLeft: 6, fontSize: 10, color: "var(--fg-2)" }}>active</span>}
                  {isLocal && <span style={{ marginLeft: 6, fontSize: 10, color: "var(--fg-2)" }}>built-in</span>}
                </td>
                <td className="mono" style={{ fontSize: 11 }}>{c.baseUrl}</td>
                <td>{c.auth.kind}</td>
                <td>
                  <span className={`conn-chip__health is-${healthGlyph}`} />
                </td>
                <td>
                  <div className="row-actions">
                    {!isActive && (
                      <button onClick={() => ctx.setActiveConnection(c.id)}>
                        Use
                      </button>
                    )}
                    <button
                      className="secondary"
                      onClick={() => setDraft(draftFromConnection(c))}
                    >
                      Edit
                    </button>
                    {!isLocal && (
                      <button
                        className="secondary"
                        onClick={() => {
                          const fresh = ctx.duplicateConnection(c.id);
                          if (fresh) setDraft(draftFromConnection(fresh));
                        }}
                      >
                        Duplicate
                      </button>
                    )}
                    {confirmDelete === c.id ? (
                      <>
                        <button
                          onClick={async () => {
                            await ctx.removeConnection(c.id);
                            setConfirmDelete(null);
                          }}
                        >
                          Confirm
                        </button>
                        <button
                          className="secondary"
                          onClick={() => setConfirmDelete(null)}
                        >
                          Cancel
                        </button>
                      </>
                    ) : (
                      <button
                        className="secondary"
                        disabled={!canDelete}
                        onClick={() => setConfirmDelete(c.id)}
                        title={
                          isLocal
                            ? "The built-in local connection cannot be removed"
                            : isActive
                              ? "Switch to another connection before removing this one"
                              : "Remove this connection"
                        }
                      >
                        Remove
                      </button>
                    )}
                  </div>
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>

      <form className="connections-form" onSubmit={onSubmit}>
        <h3 style={{ gridColumn: "1 / -1", margin: "16px 0 4px", fontSize: 13 }}>
          {isEditing ? `Editing "${draft.label || draft.id}"` : "Add a connection"}
        </h3>
        <label htmlFor="conn-label">Label</label>
        <input
          id="conn-label"
          type="text"
          value={draft.label}
          onChange={(e) => setDraft({ ...draft, label: e.target.value })}
          required
        />
        <label htmlFor="conn-url">Base URL</label>
        <input
          id="conn-url"
          type="url"
          value={draft.baseUrl}
          onChange={(e) => setDraft({ ...draft, baseUrl: e.target.value })}
          required
          disabled={isLocalEdit}
        />
        <label htmlFor="conn-auth">Auth</label>
        <select
          id="conn-auth"
          value={draft.authKind}
          onChange={(e) => setDraft({ ...draft, authKind: e.target.value as AuthKind })}
          disabled={isLocalEdit}
        >
          <option value="none">none</option>
          <option value="bearer">bearer</option>
          <option value="basic">basic</option>
        </select>
        {draft.authKind === "bearer" && (
          <>
            <label htmlFor="conn-token">Token</label>
            <input
              id="conn-token"
              type="password"
              value={draft.bearerToken}
              onChange={(e) => setDraft({ ...draft, bearerToken: e.target.value })}
            />
          </>
        )}
        {draft.authKind === "basic" && (
          <>
            <label htmlFor="conn-user">Username</label>
            <input
              id="conn-user"
              type="text"
              value={draft.basicUser}
              onChange={(e) => setDraft({ ...draft, basicUser: e.target.value })}
            />
            <label htmlFor="conn-pass">Password</label>
            <input
              id="conn-pass"
              type="password"
              value={draft.basicPass}
              onChange={(e) => setDraft({ ...draft, basicPass: e.target.value })}
            />
          </>
        )}
        <label htmlFor="conn-color">Color</label>
        <input
          id="conn-color"
          type="color"
          value={draft.color}
          onChange={(e) => setDraft({ ...draft, color: e.target.value })}
          style={{ width: 60 }}
        />
        <div className="actions">
          <button
            type="button"
            className="secondary"
            onClick={async () => {
              setTest({ state: "loading" });
              const result = await probeFromDraft(draft);
              setTest(result);
            }}
            disabled={!draft.baseUrl}
          >
            Test
          </button>
          <button type="submit" disabled={!draft.label || !draft.baseUrl}>
            {isEditing ? "Save changes" : "Add connection"}
          </button>
          {isEditing && (
            <button
              type="button"
              className="secondary"
              onClick={() => {
                setDraft(EMPTY_DRAFT);
                setTest({ state: "idle" });
              }}
            >
              Cancel
            </button>
          )}
          <span style={{ marginLeft: 12, alignSelf: "center", fontSize: 12 }}>
            {test.state === "loading" && <em>probing…</em>}
            {test.state === "ok" && (
              <span style={{ color: "#22c55e" }}>
                ok · {test.latencyMs}ms{test.service ? ` · ${test.service}` : ""}
              </span>
            )}
            {test.state === "err" && (
              <span style={{ color: "#f87171" }}>err · {test.message}</span>
            )}
          </span>
        </div>
      </form>
    </div>
  );
}
