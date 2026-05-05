/**
 * Phase9i — Retention dashboard view.
 *
 * Three layers stacked vertically:
 *
 * 1. **Failure banner** — red bar visible only when any sweep type
 *    has `status='failed'` in its two most recent runs. Shows the
 *    most recent `last_error` so the operator sees exactly which
 *    sweep is broken without grepping `retention_sweeps`.
 * 2. **Header cards** — one card per sweep type. Color-coded state
 *    (`ok` / `degraded` / `failed`), last-run relative time, bytes
 *    reclaimed. Source: `/v1/retention/sweeps`.
 * 3. **Breakdown table + live log** — the table renders archive +
 *    CAS totals (sortable) from `/v1/retention/state`. The live log
 *    strip subscribes to the timeline SSE stream and filters for
 *    events whose `kind` starts with `retention.` (capped at 100
 *    entries).
 */

import { useMemo, useState } from "react";
import { useQuery } from "@tanstack/react-query";

import { Icon } from "../atoms/Icon";
import { Sparkline } from "../atoms/Sparkline";
import {
  api,
  type RetentionScheduledRun,
  type RetentionSweepRow,
  type RetentionStateBody,
} from "../lib/api";
import { fmtNum } from "../lib/format";
import { useSSE } from "../lib/useSSE";
import { useConnKey } from "../lib/connections/useConnKey";

/// Wire-shape for a timeline event the SSE stream yields. Mirrors
/// the daemon's `TimelineEvent` shape just enough that we can pluck
/// `kind` for the retention filter.
type LiveTimelineEvent = {
  event_id?: string;
  kind?: string;
  title?: string;
  occurred_at?: string;
};

const STREAM_URL = (() => {
  if (typeof window !== "undefined" && window.location.protocol === "file:") {
    return "http://127.0.0.1:17000/v1/dashboard/timeline/stream";
  }
  return "/v1/dashboard/timeline/stream";
})();

const SWEEP_TYPES: Array<{ id: string; label: string }> = [
  { id: "tier_sweep", label: "Tier sweep" },
  { id: "parquet_rollup", label: "Archive rollup" },
  { id: "cas_vacuum", label: "CAS vacuum" },
  { id: "pii_enforce", label: "PII enforcement" },
  { id: "turn_digest", label: "Turn digest" },
  { id: "meili_prune", label: "Meili prune" },
  { id: "metadata_reap", label: "Metadata reap" },
  { id: "tool_call_digest", label: "Tool-call digest" },
  { id: "consolidator_nightly", label: "Consolidator (nightly)" },
  { id: "consolidation_prune", label: "Consolidation prune" },
  { id: "memory_consolidate", label: "Memory consolidate" },
];

type SweepHealth = "ok" | "degraded" | "failed" | "never" | "disabled";

type SweepCardData = {
  id: string;
  label: string;
  state: SweepHealth;
  last_run: RetentionSweepRow | null;
  last_error: string | null;
  /// Number of consecutive `failed` runs at the top of the history.
  consecutive_failures: number;
  /// phase11v §1.5 — live `cron_jobs` snapshot for this sweep slug.
  /// Tells the truth about `last_run` / `next_run` / `last_status`
  /// even when `retention_sweeps` is empty (which it is until
  /// phase11v §6 lands).
  schedule: RetentionScheduledRun | null;
};

/// Project the raw sweep history + the live cron schedule into one
/// card per sweep type. The schedule lane (phase11v §1) is the
/// source of truth for `last_run` / `last_status` / `next_run`; the
/// sweep history lane (`retention_sweeps`) supplies bytes-reclaimed
/// counters when present. Either lane being empty does not stop the
/// other from rendering.
function projectCards(
  rows: RetentionSweepRow[],
  schedules: RetentionScheduledRun[],
): SweepCardData[] {
  const scheduleBySlug = new Map<string, RetentionScheduledRun>(
    schedules.map((s) => [s.sweep, s]),
  );
  return SWEEP_TYPES.map(({ id, label }) => {
    const matches = rows.filter((r) =>
      Object.prototype.hasOwnProperty.call(r.stages ?? {}, id),
    );
    const last_run = matches[0] ?? null;
    let consecutive_failures = 0;
    for (const r of matches) {
      if (r.status === "failed") consecutive_failures += 1;
      else break;
    }
    const schedule = scheduleBySlug.get(id) ?? null;
    // phase11v §1.5 — the schedule lane drives state when the sweep
    // history lane is empty (which is the steady state until §6 lands).
    let state: SweepHealth;
    if (schedule?.next_run === "disabled") {
      state = "disabled";
    } else if ((schedule?.failure_streak ?? 0) > 0 || schedule?.last_status === "failed") {
      state = "failed";
    } else if (last_run !== null) {
      if (last_run.status === "failed") state = "failed";
      else if (last_run.status === "abandoned") state = "degraded";
      else state = "ok";
    } else if (schedule?.last_status === "success") {
      state = "ok";
    } else {
      state = "never";
    }
    const last_error =
      consecutive_failures > 0 && last_run !== null
        ? extractErrorString(last_run.stages, id)
        : null;
    return { id, label, state, last_run, last_error, consecutive_failures, schedule };
  });
}

function extractErrorString(stages: Record<string, unknown>, key: string): string | null {
  const stage = stages?.[key];
  if (stage && typeof stage === "object") {
    const e = (stage as Record<string, unknown>).error ?? (stage as Record<string, unknown>).last_error;
    if (typeof e === "string" && e.length > 0) return e;
  }
  return null;
}

function relativeTime(ts: string | null | undefined): string {
  if (!ts) return "never";
  const t = Date.parse(ts);
  if (!Number.isFinite(t)) return "—";
  const delta = Math.max(0, Date.now() - t);
  if (delta < 60_000) return `${Math.floor(delta / 1000)}s ago`;
  if (delta < 3_600_000) return `${Math.floor(delta / 60_000)}m ago`;
  if (delta < 86_400_000) return `${Math.floor(delta / 3_600_000)}h ago`;
  return `${Math.floor(delta / 86_400_000)}d ago`;
}

function fmtBytes(n: number): string {
  if (n === 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  let v = n;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i += 1;
  }
  return `${v.toFixed(v >= 100 ? 0 : 1)} ${units[i]}`;
}

type SortKey = "name" | "size_now" | "delta";
type BreakdownRow = {
  source: string;
  name: string;
  size_now: number;
  delta: number;
};

export function RetentionView() {
  const connKey = useConnKey();
  const sweepsQ = useQuery({
    queryKey: [connKey, "retention-sweeps"],
    queryFn: () => api.retentionSweeps(50),
    refetchInterval: 10_000,
    refetchIntervalInBackground: true,
  });
  const stateQ = useQuery({
    queryKey: [connKey, "retention-state"],
    queryFn: () => api.retentionState(),
    refetchInterval: 10_000,
  });
  const sweeps = sweepsQ.data ?? [];
  const schedules = stateQ.data?.next_runs ?? [];
  const cards = useMemo(
    () => projectCards(sweeps, schedules),
    [sweeps, schedules],
  );

  // Time-series for "bytes reclaimed per day, last 30 d". We bucket
  // every sweep that carries a numeric `bytes_reclaimed` field into
  // a 30-day timeline starting at today-29 → today.
  const byteSeries = useMemo(() => buildByteSeries(sweeps), [sweeps]);

  // Failure banner: any sweep type with ≥ 2 consecutive failures.
  const failures = cards.filter((c) => c.consecutive_failures >= 2);

  const [sortKey, setSortKey] = useState<SortKey>("size_now");
  const [sortDesc, setSortDesc] = useState(true);
  const breakdown = useMemo(() => buildBreakdown(stateQ.data), [stateQ.data]);
  const sortedBreakdown = useMemo(() => {
    const rows = [...breakdown];
    rows.sort((a, b) => {
      const av = sortKey === "name" ? a.name : sortKey === "size_now" ? a.size_now : a.delta;
      const bv = sortKey === "name" ? b.name : sortKey === "size_now" ? b.size_now : b.delta;
      if (av === bv) return 0;
      const cmp = av > bv ? 1 : -1;
      return sortDesc ? -cmp : cmp;
    });
    return rows;
  }, [breakdown, sortKey, sortDesc]);

  const onSortClick = (key: SortKey) => {
    if (sortKey === key) {
      setSortDesc(!sortDesc);
    } else {
      setSortKey(key);
      setSortDesc(true);
    }
  };

  // Live log strip — SSE timeline filtered to retention.* events,
  // capped at 100 entries (newest first).
  const [logEntries, setLogEntries] = useState<LiveTimelineEvent[]>([]);
  const sseStatus = useSSE<LiveTimelineEvent>(STREAM_URL, {
    eventName: "timeline",
    onEvent: (payload) => {
      const kind = payload?.kind ?? "";
      if (!kind.startsWith("retention.")) return;
      setLogEntries((prev) => {
        const next = [payload, ...prev];
        return next.length > 100 ? next.slice(0, 100) : next;
      });
    },
  });

  return (
    <>
      <section className="health-section">
        <h2>Retention sweeps</h2>
        {failures.length > 0 ? (
          <FailureBanner failures={failures} />
        ) : null}
        <div className="retention-cards">
          {cards.map((c) => (
            <SweepCard key={c.id} card={c} />
          ))}
        </div>
      </section>

      <section className="health-section">
        <h2 className="retention-section__title">
          <Icon name="spark" size={14} /> Bytes reclaimed · last 30 d
        </h2>
        {byteSeries.some((v) => v > 0) ? (
          <div style={{ padding: "0 8px", color: "var(--accent)" }}>
            <Sparkline data={byteSeries} height={48} />
          </div>
        ) : (
          <div className="muted" style={{ padding: 12 }}>
            No reclamation reported in the last 30 days.
          </div>
        )}
      </section>

      <section className="health-section">
        <h2 className="retention-section__title">
          <Icon name="archive" size={14} /> Storage breakdown
        </h2>
        <table className="retention-table">
          <thead>
            <tr>
              <th onClick={() => onSortClick("name")} role="button">
                Source {sortKey === "name" ? (sortDesc ? "↓" : "↑") : ""}
              </th>
              <th onClick={() => onSortClick("size_now")} role="button">
                Size now {sortKey === "size_now" ? (sortDesc ? "↓" : "↑") : ""}
              </th>
              <th>Size 30 d ago</th>
              <th onClick={() => onSortClick("delta")} role="button">
                Δ {sortKey === "delta" ? (sortDesc ? "↓" : "↑") : ""}
              </th>
            </tr>
          </thead>
          <tbody>
            {sortedBreakdown.length === 0 ? (
              <tr>
                <td colSpan={4} className="muted" style={{ padding: 12 }}>
                  Storage probe unavailable on this stack — wire
                  Vectorizer / Meili creds for live counters.
                </td>
              </tr>
            ) : (
              sortedBreakdown.map((row) => (
                <tr key={`${row.source}/${row.name}`}>
                  <td>
                    <span className="mono" style={{ fontSize: 12 }}>
                      {row.source}/{row.name}
                    </span>
                  </td>
                  <td>{fmtBytes(row.size_now)}</td>
                  <td className="muted">
                    {row.delta === 0 ? "—" : fmtBytes(row.size_now - row.delta)}
                  </td>
                  <td
                    style={{
                      color:
                        row.delta > 0
                          ? "var(--ok)"
                          : row.delta < 0
                          ? "var(--alert)"
                          : "var(--fg-3)",
                    }}
                  >
                    {row.delta === 0
                      ? "—"
                      : (row.delta > 0 ? "+" : "−") + fmtBytes(Math.abs(row.delta))}
                  </td>
                </tr>
              ))
            )}
          </tbody>
        </table>
      </section>

      <section className="health-section">
        <h2 className="retention-section__title">
          <Icon name="timeline" size={14} /> Live log
          <span
            className="muted"
            style={{ marginLeft: 8, fontSize: 11 }}
            title={
              sseStatus.connected ? "SSE connected" : "SSE disconnected"
            }
          >
            {sseStatus.connected ? "● live" : "○ offline"}
          </span>
        </h2>
        {logEntries.length === 0 ? (
          <div className="muted" style={{ padding: 12 }}>
            Waiting for retention events…
          </div>
        ) : (
          <ul className="retention-log">
            {logEntries.map((e, i) => (
              <li key={`${e.event_id ?? i}`}>
                <span className="mono" style={{ color: "var(--accent)" }}>
                  {e.kind ?? "retention.?"}
                </span>{" "}
                <span style={{ color: "var(--fg-2)" }}>
                  {e.title ?? "(no title)"}
                </span>{" "}
                <span className="muted">{relativeTime(e.occurred_at)}</span>
              </li>
            ))}
          </ul>
        )}
      </section>
    </>
  );
}

function FailureBanner({ failures }: { failures: SweepCardData[] }) {
  return (
    <div
      className="retention-banner retention-banner--fail"
      role="alert"
      aria-live="assertive"
    >
      <Icon name="alert" size={14} />
      <div>
        <strong>
          {failures.length === 1 ? "Sweep failing" : "Sweeps failing"}:
        </strong>{" "}
        {failures
          .map(
            (f) =>
              `${f.label} (${f.consecutive_failures} consecutive failure${
                f.consecutive_failures === 1 ? "" : "s"
              })`,
          )
          .join(", ")}
        {failures.some((f) => f.last_error) ? (
          <div
            className="mono"
            style={{ marginTop: 4, fontSize: 11, color: "var(--fg-2)" }}
          >
            last_error: {failures.find((f) => f.last_error)!.last_error}
          </div>
        ) : null}
      </div>
    </div>
  );
}

function SweepCard({ card }: { card: SweepCardData }) {
  const colour =
    card.state === "ok"
      ? "var(--ok)"
      : card.state === "degraded"
      ? "var(--warn, #d49a00)"
      : card.state === "failed"
      ? "var(--alert, #c0392b)"
      : card.state === "disabled"
      ? "var(--fg-3)"
      : "var(--fg-3)";
  const reclaimed = card.last_run ? sweepBytesReclaimed(card.last_run, card.id) : 0;
  // phase11v §1.5 — prefer the live schedule lane for last/next-run
  // strings; fall back to the sweep history lane only when the
  // schedule didn't carry timestamps.
  const lastRunStr =
    card.schedule?.last_run ?? card.last_run?.started_at ?? null;
  const nextRunStr = card.schedule?.next_run ?? null;
  const lastStatus = card.schedule?.last_status ?? card.last_run?.status ?? null;
  const failureStreak = card.schedule?.failure_streak ?? 0;
  return (
    <div className="retention-card">
      <div className="retention-card__head">
        <span
          className="retention-card__dot"
          style={{ background: colour }}
          aria-label={`state ${card.state}`}
        />
        <span className="retention-card__label">{card.label}</span>
        {card.state === "disabled" ? (
          <span className="muted" style={{ marginLeft: "auto", fontSize: 11 }}>
            disabled
          </span>
        ) : null}
      </div>
      <div className="retention-card__meta">
        <span className="muted">last run</span>
        <span>{relativeTime(lastRunStr)}</span>
      </div>
      <div className="retention-card__meta">
        <span className="muted">next run</span>
        <span>
          {nextRunStr === null || nextRunStr === "never"
            ? "—"
            : nextRunStr === "disabled"
            ? "(disabled)"
            : relativeNextRun(nextRunStr)}
        </span>
      </div>
      <div className="retention-card__meta">
        <span className="muted">status</span>
        <span
          className="mono"
          style={{
            color:
              lastStatus === "success"
                ? "var(--ok)"
                : lastStatus === "failed"
                ? "var(--alert, #c0392b)"
                : "var(--fg-3)",
          }}
        >
          {lastStatus ?? "—"}
          {failureStreak > 0 ? ` (×${failureStreak})` : ""}
        </span>
      </div>
      <div className="retention-card__meta">
        <span className="muted">reclaimed</span>
        <span className="mono">
          {reclaimed > 0 ? fmtBytes(reclaimed) : "—"}
        </span>
      </div>
      {card.last_run ? (
        <div className="retention-card__meta">
          <span className="muted">demoted / dropped</span>
          <span className="mono">
            {fmtNum(card.last_run.records_demoted)} / {fmtNum(card.last_run.records_dropped)}
          </span>
        </div>
      ) : null}
    </div>
  );
}

/// phase11v §1.5 — render an RFC-3339 next_run timestamp as
/// "in 4h", "in 2d", "due now". Falls back to an em-dash on
/// unparseable input rather than silently swallowing the error.
function relativeNextRun(ts: string): string {
  const t = Date.parse(ts);
  if (!Number.isFinite(t)) return "—";
  const delta = t - Date.now();
  if (delta <= 0) return "due now";
  if (delta < 60_000) return `in ${Math.floor(delta / 1000)}s`;
  if (delta < 3_600_000) return `in ${Math.floor(delta / 60_000)}m`;
  if (delta < 86_400_000) return `in ${Math.floor(delta / 3_600_000)}h`;
  return `in ${Math.floor(delta / 86_400_000)}d`;
}

/// Pluck `bytes_reclaimed` (or per-stage equivalents) from a sweep
/// row's `stages` payload. Falls back to `0` when the stage didn't
/// emit a numeric reclaim counter.
function sweepBytesReclaimed(row: RetentionSweepRow, id: string): number {
  const stage = row.stages?.[id];
  if (stage && typeof stage === "object") {
    const v =
      (stage as Record<string, unknown>).bytes_reclaimed ??
      (stage as Record<string, unknown>).bytes;
    if (typeof v === "number" && Number.isFinite(v)) return v;
  }
  return 0;
}

function buildByteSeries(rows: RetentionSweepRow[]): number[] {
  const series = new Array(30).fill(0) as number[];
  const today = new Date();
  today.setUTCHours(0, 0, 0, 0);
  for (const r of rows) {
    const t = Date.parse(r.started_at);
    if (!Number.isFinite(t)) continue;
    const dayDelta = Math.floor((today.getTime() - t) / 86_400_000);
    if (dayDelta < 0 || dayDelta >= 30) continue;
    const idx = 29 - dayDelta;
    let bytes = 0;
    if (r.stages && typeof r.stages === "object") {
      for (const v of Object.values(r.stages)) {
        if (v && typeof v === "object") {
          const b = (v as Record<string, unknown>).bytes_reclaimed ?? (v as Record<string, unknown>).bytes;
          if (typeof b === "number" && Number.isFinite(b)) bytes += b;
        }
      }
    }
    series[idx] += bytes;
  }
  return series;
}

function buildBreakdown(state: RetentionStateBody | undefined): BreakdownRow[] {
  if (!state) return [];
  const rows: BreakdownRow[] = [];
  if (state.archive_bytes.available) {
    rows.push({
      source: "archive",
      name: "le_30d",
      size_now: state.archive_bytes.le_30d,
      delta: 0,
    });
    rows.push({
      source: "archive",
      name: "30d_to_365d",
      size_now: state.archive_bytes["30d_to_365d"],
      delta: 0,
    });
    rows.push({
      source: "archive",
      name: "gt_365d",
      size_now: state.archive_bytes.gt_365d,
      delta: 0,
    });
  }
  if (state.cas.available) {
    rows.push({
      source: "cas",
      name: "blobs",
      size_now: state.cas.bytes,
      delta: 0,
    });
  }
  return rows;
}
