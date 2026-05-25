import { useQuery } from "@tanstack/react-query";

import { api } from "../lib/api";
import { useConnKey } from "../lib/connections/useConnKey";

/**
 * Identity-coverage dashboard view (ADR-014 / phase13f §5.3).
 *
 * Consumes the typed `CoverageReportView` returned by
 * `/v1/dashboard/coverage` verbatim. No local fallback branches —
 * the dashboard handler is the single source of truth for the
 * displayed state, including the "not wired" case (signalled by an
 * empty `metadata_db` string).
 */
export function CoverageView() {
  const connKey = useConnKey();
  const { data, isLoading, error } = useQuery({
    queryKey: [connKey, "coverage"],
    queryFn: () => api.coverage(),
    refetchInterval: 30_000,
    refetchIntervalInBackground: true,
  });

  if (isLoading && !data) {
    return (
      <div className="view">
        <div className="view__head">
          <h2>Identity coverage</h2>
        </div>
        <p className="view__muted">Loading…</p>
      </div>
    );
  }

  if (error) {
    return (
      <div className="view">
        <div className="view__head">
          <h2>Identity coverage</h2>
        </div>
        <p className="view__error">Failed to load coverage report.</p>
      </div>
    );
  }

  if (!data) {
    return null;
  }

  const notWired = data.metadata_db === "";

  return (
    <div className="view">
      <div className="view__head">
        <h2>Identity coverage</h2>
        <span className="view__muted">
          {notWired
            ? "Metadata store not wired."
            : `Scanned ${data.rows_total.toLocaleString()} rows in ${data.latency_ms} ms.`}
        </span>
      </div>

      <div
        className="view__pill"
        data-state={data.is_healthy ? "ok" : "fail"}
        style={{ marginBottom: 12 }}
      >
        {data.is_healthy ? "Healthy" : "Coverage gaps detected"}
      </div>

      <table className="data-table">
        <thead>
          <tr>
            <th>Backend</th>
            <th style={{ textAlign: "right" }}>Missing rows</th>
            <th>Sample event_ids</th>
          </tr>
        </thead>
        <tbody>
          {data.backends.map((b) => (
            <tr key={b.backend}>
              <td style={{ fontFamily: "var(--font-mono)" }}>{b.backend}</td>
              <td style={{ textAlign: "right" }}>{b.missing.toLocaleString()}</td>
              <td style={{ fontFamily: "var(--font-mono)", fontSize: 11 }}>
                {b.sample_event_ids.length === 0
                  ? ""
                  : b.sample_event_ids.join(", ")}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
