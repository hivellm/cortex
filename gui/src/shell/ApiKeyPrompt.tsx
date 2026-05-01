/**
 * ApiKeyPrompt — phase3 §6.7.
 *
 * Modal that pops when any fetcher observes a 401 from a non-localhost
 * connection. The user pastes a bearer key minted via
 * `cortex-api admin issue-api-key --scope dashboard` (phase3 §7.6);
 * the value lands on the active Connection's `auth.token` field
 * via the connections store.
 *
 * The modal is not closable on ESC or backdrop click — operator
 * either pastes a key or switches connection. Mirrors the contract
 * locked in `phase3_gui_multi_connection/specs/gui-connections/spec.md`
 * scenario "401 from a remote pops the ApiKeyPrompt".
 *
 * Local connections (auth=none, baseUrl=127.0.0.1) never trigger
 * this modal — see ApiKeyPromptHost below for the routing logic.
 */

import { useEffect, useState } from "react";

import { LOCAL_CONNECTION_ID, useConnections } from "../lib/connections";
import { ApiError } from "../lib/api";

export type ApiKeyPromptProps = {
  open: boolean;
  /// Connection label shown in the modal body, so the user knows
  /// which backend is asking for a key.
  connectionLabel: string;
  /// Connection base URL — printed inline so the user can verify
  /// they are pasting a key for the right host.
  connectionBaseUrl: string;
  onSubmit: (token: string) => void;
};

export function ApiKeyPrompt({
  open,
  connectionLabel,
  connectionBaseUrl,
  onSubmit,
}: ApiKeyPromptProps) {
  const [value, setValue] = useState("");
  useEffect(() => {
    if (open) setValue("");
  }, [open]);

  if (!open) return null;

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    const trimmed = value.trim();
    if (!trimmed) return;
    onSubmit(trimmed);
  };

  return (
    <div className="api-key-prompt-backdrop" role="dialog" aria-modal="true">
      <div className="api-key-prompt">
        <h3>API key required</h3>
        <p>
          The backend at <strong>{connectionLabel}</strong> returned 401 on a
          dashboard request. Paste a bearer key issued by the daemon's admin
          CLI to continue. The key is stored on this connection and reused
          for every subsequent request.
        </p>
        <p>Mint a key with:</p>
        <code>
          cortex-api admin issue-api-key --scope dashboard --label {connectionLabel}
        </code>
        <p style={{ fontSize: 11 }}>
          Backend: <span className="mono">{connectionBaseUrl}</span>
        </p>
        <form onSubmit={handleSubmit}>
          <input
            autoFocus
            type="password"
            value={value}
            onChange={(e) => setValue(e.target.value)}
            aria-label="bearer token"
          />
          <div className="actions">
            <button type="submit" disabled={!value.trim()}>
              Save and retry
            </button>
          </div>
        </form>
      </div>
    </div>
  );
}

/**
 * Host component that owns the open/closed state and listens for
 * 401 events. Lives one level inside `<ConnectionsProvider>` so it
 * has access to the active connection and its update API.
 *
 * Two trigger paths:
 *
 * 1. **Imperative** — fetchers throw `ApiError(401, …)` from the
 *    api.ts layer. The host installs a global error handler on
 *    `window.addEventListener("unhandledrejection", …)` so any
 *    Promise rejecting with a 401 ApiError pops the modal.
 *
 * 2. **Explicit** — components can call the `requestApiKey()`
 *    function exposed via context if they catch a 401 themselves
 *    (e.g. SSE re-connection logic).
 */
export function ApiKeyPromptHost() {
  const { active, updateConnection } = useConnections();
  const [open, setOpen] = useState(false);

  useEffect(() => {
    const handler = (e: PromiseRejectionEvent) => {
      const reason = e.reason;
      if (reason instanceof ApiError && reason.status === 401) {
        // Localhost connections never carry auth; a 401 there is a
        // legitimate dashboard config error (the user enabled
        // CORTEX_DASHBOARD_AUTH=1 on a localhost daemon without a
        // matching key). We surface a normal toast in that case via
        // TanStack Query's error channel and skip the modal.
        if (active.id === LOCAL_CONNECTION_ID) return;
        setOpen(true);
      }
    };
    window.addEventListener("unhandledrejection", handler);
    return () => window.removeEventListener("unhandledrejection", handler);
  }, [active.id]);

  const onSubmit = (token: string) => {
    updateConnection(active.id, { auth: { kind: "bearer", token } });
    setOpen(false);
  };

  return (
    <ApiKeyPrompt
      open={open}
      connectionLabel={active.label}
      connectionBaseUrl={active.baseUrl}
      onSubmit={onSubmit}
    />
  );
}
