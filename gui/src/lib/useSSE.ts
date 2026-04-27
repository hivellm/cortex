/**
 * `useSSE` — open an `EventSource` against `cortex-api`'s
 * `/v1/dashboard/timeline/stream` (or any spec-16 SSE endpoint),
 * yield each captured event to a callback, and surface a
 * `connected` / `lastHeartbeatAt` snapshot so the renderer can
 * flip a "stale" pill when the server stops talking.
 *
 * Reconnect ladder is exponential with a hard cap: 1 s, 2 s, 5 s,
 * 10 s, 30 s. A successful event resets the back-off step. The
 * browser auto-supplies `Last-Event-ID` on reconnect, so the
 * server can replay anything newer than the last id we saw.
 *
 * Spec-16 §SSE event envelope: every payload arrives as
 * `event: <type>\ndata: <json>\n\n`. The hook yields the parsed JSON
 * body to the caller — the caller picks the type via the third arg
 * (defaults to `timeline`).
 */

import { useEffect, useRef, useState } from "react";

const RECONNECT_LADDER_MS = [1000, 2000, 5000, 10000, 30000];
const STALE_AFTER_MS = 30000;

export type SSEStatus = {
  /// True while the EventSource reports `OPEN`.
  connected: boolean;
  /// `Date.now()` of the most recent event of any kind (heartbeat or
  /// data). Lets the renderer flip a "stale" pill when the gap grows.
  lastHeartbeatAt: number;
  /// Number of times the hook has reconnected since mount. Surfaced
  /// for the operator-side debug pill.
  reconnects: number;
};

export type UseSSEOptions<T> = {
  /// SSE event type the caller cares about. Defaults to "timeline".
  /// `heartbeat` and the EventSource's built-in `error` events are
  /// always handled internally and never forwarded.
  eventName?: string;
  /// Called with each parsed payload. Stable across renders is the
  /// caller's responsibility — the hook stores the latest reference
  /// in a ref so the EventSource isn't torn down on every prop
  /// change.
  onEvent: (payload: T) => void;
  /// Called when the JSON parse fails. Default: log to console.
  onParseError?: (raw: string, err: unknown) => void;
};

/// Open a single `EventSource` per (url, eventName) pair. The hook
/// is keyed on `url` — changing the URL closes the prior source and
/// opens a fresh one.
export function useSSE<T = unknown>(url: string, opts: UseSSEOptions<T>): SSEStatus {
  const eventName = opts.eventName ?? "timeline";
  const onEventRef = useRef(opts.onEvent);
  const onParseErrorRef = useRef(opts.onParseError);
  onEventRef.current = opts.onEvent;
  onParseErrorRef.current = opts.onParseError;

  const [status, setStatus] = useState<SSEStatus>({
    connected: false,
    lastHeartbeatAt: 0,
    reconnects: 0,
  });

  useEffect(() => {
    let closed = false;
    let source: EventSource | null = null;
    let reconnectStep = 0;
    let reconnectTimer: ReturnType<typeof setTimeout> | null = null;

    const noteActivity = () => {
      setStatus((prev) => ({ ...prev, lastHeartbeatAt: Date.now() }));
    };

    const open = () => {
      if (closed) return;
      // EventSource handles `Last-Event-ID` internally — we only
      // need to recreate the object on reconnect.
      source = new EventSource(url, { withCredentials: false });

      source.addEventListener("open", () => {
        reconnectStep = 0;
        setStatus((prev) => ({
          ...prev,
          connected: true,
          lastHeartbeatAt: Date.now(),
        }));
      });

      source.addEventListener(eventName, (raw) => {
        const message = raw as MessageEvent;
        noteActivity();
        let parsed: T;
        try {
          parsed = JSON.parse(message.data) as T;
        } catch (err) {
          if (onParseErrorRef.current) {
            onParseErrorRef.current(message.data, err);
          } else {
            console.warn("useSSE: payload parse failed", err);
          }
          return;
        }
        try {
          onEventRef.current(parsed);
        } catch (err) {
          console.error("useSSE: onEvent threw", err);
        }
      });

      source.addEventListener("heartbeat", () => {
        noteActivity();
      });

      source.addEventListener("error", () => {
        // The browser auto-reconnects via `EventSource` defaults,
        // but those defaults are aggressive (one retry per second
        // with no cap). We force-close and run our own ladder so a
        // long server outage doesn't pin the CPU.
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
  }, [url, eventName]);

  return status;
}

/// Helper for renderers: flip the `stale` flag once `STALE_AFTER_MS`
/// has elapsed without a heartbeat. Pure function so tests can drive
/// it without mounting the hook.
export function isStreamStale(status: SSEStatus, now: number = Date.now()): boolean {
  if (!status.connected) return true;
  if (status.lastHeartbeatAt === 0) return false;
  return now - status.lastHeartbeatAt > STALE_AFTER_MS;
}
