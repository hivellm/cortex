import { useQuery } from "@tanstack/react-query";

import { api } from "../lib/api";
import { useConnKey } from "../lib/connections/useConnKey";

/**
 * Producer-checkpoint dashboard view (ADR-014 / phase13f §5.4).
 *
 * Consumes the typed `ProducerCheckpointsReportView` returned by
 * `/v1/dashboard/producers` verbatim. One row per
 * `(producer_name, scope)` pair carrying the most recent checkpoint
 * the worker persisted. No local fallback branches — the absence of
 * a row means no checkpoint has been written for that pair yet.
 */
export function ProducersView() {
  const connKey = useConnKey();
  const { data, isLoading, error } = useQuery({
    queryKey: [connKey, "producers"],
    queryFn: () => api.producers(),
    refetchInterval: 30_000,
    refetchIntervalInBackground: true,
  });

  if (isLoading && !data) {
    return (
      <div className="view">
        <div className="view__head">
          <h2>Producers</h2>
        </div>
        <p className="view__muted">Loading…</p>
      </div>
    );
  }

  if (error) {
    return (
      <div className="view">
        <div className="view__head">
          <h2>Producers</h2>
        </div>
        <p className="view__error">Failed to load producer checkpoints.</p>
      </div>
    );
  }

  if (!data) {
    return null;
  }

  return (
    <div className="view">
      <div className="view__head">
        <h2>Producers</h2>
        <span className="view__muted">
          {data.total === 0
            ? "No producer checkpoints recorded."
            : `${data.total} (producer, scope) pair${data.total === 1 ? "" : "s"}.`}
        </span>
      </div>

      <table className="data-table">
        <thead>
          <tr>
            <th>Producer</th>
            <th>Scope</th>
            <th>Last event id</th>
            <th>Last occurred</th>
            <th>Accumulated</th>
            <th>Has progress</th>
          </tr>
        </thead>
        <tbody>
          {data.rows.map((r) => (
            <tr key={`${r.producer_name}​${r.scope}`}>
              <td style={{ fontFamily: "var(--font-mono)" }}>{r.producer_name}</td>
              <td style={{ fontFamily: "var(--font-mono)" }}>{r.scope}</td>
              <td style={{ fontFamily: "var(--font-mono)", fontSize: 11 }}>
                {r.last_event_id ?? ""}
              </td>
              <td style={{ fontSize: 11 }}>{r.last_occurred_at}</td>
              <td style={{ fontSize: 11 }}>{r.accumulated_at}</td>
              <td>
                <span className="view__pill" data-state={r.has_progress ? "ok" : "warn"}>
                  {r.has_progress ? "yes" : "no"}
                </span>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
