/**
 * ConnectionSwitcher — phase3 §5.
 *
 * Header chip showing the active connection (color dot + label +
 * health state) plus a click-to-open dropdown listing every known
 * connection and a "Manage…" entry that navigates to the
 * /connections view.
 *
 * Health state per connection comes from a debounced 30-second
 * probe against `/v1/dashboard/status` for that connection's base
 * URL. The probe runs *outside* TanStack Query so the cache stays
 * keyed by the active connection — health for non-active
 * connections is a separate concern surfaced only inside the
 * dropdown.
 */

import { useEffect, useMemo, useRef, useState } from "react";

import { useConnections } from "../lib/connections";
import type { Connection, ConnectionHealth } from "../lib/connections";

type ConnectionSwitcherProps = {
  /// Caller switches the active view to /connections when the user
  /// picks "Manage…" from the dropdown. Optional — when omitted the
  /// dropdown drops the entry entirely.
  onManage?: () => void;
};

const PROBE_INTERVAL_MS = 30_000;
const PROBE_TIMEOUT_MS = 4_000;

async function probeConnection(conn: Connection): Promise<ConnectionHealth> {
  const startedAt = Date.now();
  const ac = new AbortController();
  const timer = setTimeout(() => ac.abort(), PROBE_TIMEOUT_MS);
  try {
    const headers: Record<string, string> = { accept: "application/json" };
    if (conn.auth.kind === "bearer" && conn.auth.token) {
      headers.authorization = `Bearer ${conn.auth.token}`;
    } else if (conn.auth.kind === "basic" && conn.auth.username) {
      const raw = `${conn.auth.username}:${conn.auth.password}`;
      headers.authorization = `Basic ${typeof btoa === "function" ? btoa(raw) : Buffer.from(raw).toString("base64")}`;
    }
    const resp = await fetch(`${conn.baseUrl}/v1/dashboard/status`, {
      headers,
      signal: ac.signal,
    });
    if (!resp.ok) {
      return {
        state: "down",
        checkedAt: Date.now(),
        reason: `HTTP ${resp.status}`,
      };
    }
    return {
      state: "ok",
      checkedAt: Date.now(),
      latencyMs: Date.now() - startedAt,
    };
  } catch (err) {
    return {
      state: "down",
      checkedAt: Date.now(),
      reason: (err as Error).message ?? "fetch failed",
    };
  } finally {
    clearTimeout(timer);
  }
}

function healthGlyph(h: ConnectionHealth | undefined): "ok" | "down" | "unknown" {
  if (!h || h.state === "unknown") return "unknown";
  return h.state;
}

export function ConnectionSwitcher({ onManage }: ConnectionSwitcherProps) {
  const { state, active, setActiveConnection, setHealth } = useConnections();
  const [open, setOpen] = useState(false);
  const wrapperRef = useRef<HTMLDivElement>(null);

  // Probe every connection on mount + every PROBE_INTERVAL_MS. The
  // probes run in parallel; results land on the store via setHealth.
  useEffect(() => {
    let cancelled = false;
    const tick = async () => {
      const checks = state.connections.map(async (c) => {
        const h = await probeConnection(c);
        if (!cancelled) setHealth(c.id, h);
      });
      await Promise.allSettled(checks);
    };
    void tick();
    const id = setInterval(() => {
      void tick();
    }, PROBE_INTERVAL_MS);
    return () => {
      cancelled = true;
      clearInterval(id);
    };
    // The probes only depend on the connection identities — adding
    // health to the dep array would trigger re-probes on every
    // result. We close over the latest list via the reducer and
    // rely on the interval's natural cadence.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [state.connections.length]);

  // Close dropdown on outside click + Escape key.
  useEffect(() => {
    if (!open) return;
    const onClick = (e: MouseEvent) => {
      if (!wrapperRef.current) return;
      if (!wrapperRef.current.contains(e.target as Node)) setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpen(false);
    };
    window.addEventListener("mousedown", onClick);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("mousedown", onClick);
      window.removeEventListener("keydown", onKey);
    };
  }, [open]);

  const sorted = useMemo(() => {
    // Pin local first, then user-added connections by createdAt.
    const local = state.connections.filter((c) => c.id === "local");
    const rest = state.connections
      .filter((c) => c.id !== "local")
      .sort((a, b) => a.createdAt - b.createdAt);
    return [...local, ...rest];
  }, [state.connections]);

  const activeGlyph = healthGlyph(active.health);

  return (
    <div className="conn-switcher" ref={wrapperRef}>
      <button
        type="button"
        className={`conn-chip is-${activeGlyph}`}
        onClick={() => setOpen((v) => !v)}
        title={
          active.health?.state === "down"
            ? `${active.label} · down · ${active.health.reason}`
            : active.health?.state === "ok"
              ? `${active.label} · ok · ${active.health.latencyMs}ms`
              : `${active.label} · ${active.baseUrl}`
        }
        aria-haspopup="menu"
        aria-expanded={open}
      >
        <span
          className="conn-chip__dot"
          style={{ backgroundColor: active.color }}
        />
        <span className="conn-chip__label">{active.label}</span>
        <span className={`conn-chip__health is-${activeGlyph}`} />
      </button>
      {open && (
        <div className="conn-switcher__menu" role="menu">
          {sorted.map((c) => {
            const g = healthGlyph(c.health);
            const isActive = c.id === active.id;
            return (
              <button
                key={c.id}
                type="button"
                className={`conn-switcher__item ${isActive ? "is-active" : ""}`}
                onClick={() => {
                  setActiveConnection(c.id);
                  setOpen(false);
                }}
                role="menuitem"
              >
                <span
                  className="conn-chip__dot"
                  style={{ backgroundColor: c.color }}
                />
                <span className="conn-switcher__item-label">{c.label}</span>
                <span className="conn-switcher__item-url">{c.baseUrl}</span>
                <span className={`conn-chip__health is-${g}`} />
              </button>
            );
          })}
          {onManage && (
            <button
              type="button"
              className="conn-switcher__manage"
              onClick={() => {
                setOpen(false);
                onManage();
              }}
              role="menuitem"
            >
              Manage connections…
            </button>
          )}
        </div>
      )}
    </div>
  );
}
