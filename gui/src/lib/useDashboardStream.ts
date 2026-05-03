/**
 * `useDashboardStream` — single EventSource against
 * `/v1/dashboard/stream` (spec 21). Listens for the typed dashboard
 * delta events (`task.changed`, `handoff.appended`, `decision.changed`,
 * `memory.appended`, `knowledge.added`) plus the `hello` framing event,
 * and dispatches `queryClient.invalidateQueries` per kind so the
 * affected views refetch within ~1 s of any rulebook write.
 *
 * One EventSource per active connection — re-keying on `connKey` tears
 * down the prior socket when the user switches projects so the GUI
 * never receives invalidations for a project it isn't viewing.
 *
 * Reconnect ladder mirrors `useSSE` (1 s → 30 s exponential). On the
 * first frame after every (re)connect, the hook fires a global
 * `invalidateQueries` for all dashboard keys so any window where the
 * stream was down is recovered without polling.
 */

import { useEffect, useRef, useState } from "react";
import { useQueryClient, type QueryClient } from "@tanstack/react-query";

import { dashboardStreamUrl } from "./api";
import { useConnKey } from "./connections/useConnKey";

const RECONNECT_LADDER_MS = [1000, 2000, 5000, 10000, 30000];

/// Event-kind tags the server emits. Mirrors `DashboardEventKind` in
/// `crates/cortex-core/src/dashboard_event.rs`.
export const DASHBOARD_EVENT_KINDS = [
  "task.changed",
  "handoff.appended",
  "decision.changed",
  "memory.appended",
  "knowledge.added",
] as const;

export type DashboardEventKind = (typeof DASHBOARD_EVENT_KINDS)[number];

export type DashboardEvent = {
  event_id: string;
  kind: DashboardEventKind;
  entity_id: string;
  ts: string;
  summary?: string;
  delta?: unknown;
  source: "mcp" | "watcher";
};

export type DashboardStreamStatus = {
  /// EventSource is in OPEN state.
  connected: boolean;
  /// Number of (re)connects since mount. Surfaced for the status pill.
  reconnects: number;
  /// `Date.now()` of the most recently received event of any kind.
  /// `0` until the first event lands. Lets the renderer flag a stale
  /// stream when the gap grows past the SSE keep-alive interval.
  lastEventAt: number;
};

/// Per-kind react-query key prefixes. Each entry is a tuple-prefix the
/// hook hands to `queryClient.invalidateQueries({ queryKey })` — every
/// matching key (any tail) is marked stale and refetched.
///
/// The list intentionally includes the sibling-summary keys
/// (`tasks-summary`) the sidebar pill consumes, so the badge counters
/// update at the same time the view does.
function keyPrefixesFor(
  connKey: string,
  kind: DashboardEventKind,
  entityId: string | null = null,
): string[][] {
  switch (kind) {
    case "task.changed":
      return [
        [connKey, "tasks"],
        [connKey, "tasks-summary"],
      ];
    case "handoff.appended":
      return [[connKey, "handoffs"]];
    case "decision.changed": {
      const out: string[][] = [[connKey, "decisions"]];
      if (entityId) out.push([connKey, "decision-detail", entityId]);
      return out;
    }
    case "memory.appended":
      return [[connKey, "memory"]];
    case "knowledge.added":
      return [[connKey, "knowledge"]];
  }
}

/// Invalidate every dashboard query for `connKey`. Used on (re)connect
/// to recover any events the GUI missed while the stream was down.
export function invalidateAllDashboardQueries(client: QueryClient, connKey: string): void {
  for (const kind of DASHBOARD_EVENT_KINDS) {
    for (const queryKey of keyPrefixesFor(connKey, kind)) {
      client.invalidateQueries({ queryKey });
    }
  }
}

/// Open the dashboard SSE stream for the active connection and wire
/// each event to a `queryClient.invalidateQueries` call. Returns a
/// status snapshot the header can render.
export function useDashboardStream(): DashboardStreamStatus {
  const queryClient = useQueryClient();
  const connKey = useConnKey();
  const [status, setStatus] = useState<DashboardStreamStatus>({
    connected: false,
    reconnects: 0,
    lastEventAt: 0,
  });

  // Stable refs so the EventSource isn't torn down when the parent
  // re-renders — only the (connKey, url) pair gates re-open.
  const queryClientRef = useRef(queryClient);
  queryClientRef.current = queryClient;
  const connKeyRef = useRef(connKey);
  connKeyRef.current = connKey;

  useEffect(() => {
    let closed = false;
    let source: EventSource | null = null;
    let reconnectStep = 0;
    let reconnectTimer: ReturnType<typeof setTimeout> | null = null;

    const noteEvent = () => {
      setStatus((prev) => ({ ...prev, lastEventAt: Date.now() }));
    };

    const handleDashboardEvent = (raw: MessageEvent, kind: DashboardEventKind) => {
      noteEvent();
      // The body is the typed envelope. We don't actually need the
      // contents — knowing the kind is enough to invalidate. Parsing
      // is still useful because a malformed payload signals a server
      // bug worth noticing.
      try {
        const parsed: DashboardEvent = JSON.parse(raw.data);
        if (parsed.kind !== kind) {
          // Tag mismatch between the SSE `event:` field and the body.
          // The bus is wrong — log and skip rather than dispatching the
          // wrong invalidation.
          console.warn("dashboard stream: kind mismatch", { sseKind: kind, body: parsed });
          return;
        }
      } catch (err) {
        console.warn("dashboard stream: payload parse failed", err);
        return;
      }
      for (const queryKey of keyPrefixesFor(connKeyRef.current, kind)) {
        queryClientRef.current.invalidateQueries({ queryKey });
      }
    };

    const open = () => {
      if (closed) return;
      const url = dashboardStreamUrl();
      source = new EventSource(url, { withCredentials: false });

      source.addEventListener("open", () => {
        reconnectStep = 0;
        setStatus((prev) => ({
          ...prev,
          connected: true,
          lastEventAt: Date.now(),
        }));
      });

      source.addEventListener("hello", (raw) => {
        noteEvent();
        // Always resync on hello — the server signals lost_window=true
        // when our subscription lagged, and even on a clean hello the
        // GUI may have been disconnected long enough to miss writes.
        // Re-fetching is cheap; missing an update is not.
        try {
          const body = JSON.parse((raw as MessageEvent).data) as { lost_window?: boolean };
          if (body.lost_window === false && status.reconnects === 0) {
            // First-ever hello on this mount — the existing react-query
            // queries are still warm, no need to invalidate.
            return;
          }
        } catch {
          // Fall through to invalidate; safer to over-invalidate than
          // miss a delta because the hello body was malformed.
        }
        invalidateAllDashboardQueries(queryClientRef.current, connKeyRef.current);
      });

      for (const kind of DASHBOARD_EVENT_KINDS) {
        source.addEventListener(kind, (raw) => handleDashboardEvent(raw as MessageEvent, kind));
      }

      source.addEventListener("error", () => {
        if (source) {
          source.close();
          source = null;
        }
        const delay =
          RECONNECT_LADDER_MS[Math.min(reconnectStep, RECONNECT_LADDER_MS.length - 1)];
        reconnectStep += 1;
        setStatus((prev) => ({
          ...prev,
          connected: false,
          reconnects: prev.reconnects + 1,
        }));
        if (closed) return;
        reconnectTimer = setTimeout(open, delay);
      });
    };

    open();

    return () => {
      closed = true;
      if (reconnectTimer !== null) clearTimeout(reconnectTimer);
      if (source) source.close();
    };
    // The hook intentionally re-opens when `connKey` changes so a
    // project switch does not pollute the new project with the old
    // project's invalidations.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [connKey]);

  return status;
}
