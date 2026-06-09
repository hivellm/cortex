import { useQuery } from "@tanstack/react-query";

import { api, type CanaryReport, type CanaryRunView } from "../lib/api";
import { useConnKey } from "../lib/connections/useConnKey";

/// Phase15f §3.4 — Canary dashboard view.
///
/// Shows the last 24 h of pre-thinking health canary ticks as a status
/// sparkline and highlights the most recent failure (if any) so the
/// operator sees at a glance whether the pre-thinking pipeline is healthy.
export function CanaryView() {
  const connKey = useConnKey();
  const { data, isLoading, error } = useQuery({
    queryKey: [connKey, "dashboard", "canary"],
    queryFn: () => api.dashboardCanary(),
    refetchInterval: 60_000,
    refetchIntervalInBackground: true,
  });

  if (isLoading && !data) {
    return (
      <div className="view">
        <div className="view__head">
          <h2>Canary</h2>
        </div>
        <p className="view__muted">Loading…</p>
      </div>
    );
  }

  if (error) {
    return (
      <div className="view">
        <div className="view__head">
          <h2>Canary</h2>
        </div>
        <p className="view__error">Failed to load canary data.</p>
      </div>
    );
  }

  if (!data) {
    return null;
  }

  return (
    <div className="view">
      <div className="view__head">
        <h2>Canary</h2>
        <StatusPill report={data} />
      </div>
      {data.last_failure && <FailureBanner run={data.last_failure} />}
      <SparklineTable runs={data.runs} />
    </div>
  );
}

function StatusPill({ report }: { report: CanaryReport }) {
  const healthy = report.last_failure === null;
  return (
    <span
      className={`health-pill health-pill--${healthy ? "ok" : "critical"}`}
      style={{ marginLeft: 8 }}
    >
      {healthy ? "ok" : "alarm"}
    </span>
  );
}

function FailureBanner({ run }: { run: CanaryRunView }) {
  return (
    <div className="health-banner health-banner--down" style={{ marginBottom: 12 }}>
      <strong>Last failure:</strong>{" "}
      <span className="mono">{run.ts}</span>
      {run.error_message ? (
        <span> — {run.error_message}</span>
      ) : null}
    </div>
  );
}

function SparklineTable({ runs }: { runs: CanaryRunView[] }) {
  if (runs.length === 0) {
    return <p className="view__muted">No canary ticks recorded yet.</p>;
  }
  const okCount = runs.filter((r) => r.status === "ok").length;
  const errCount = runs.length - okCount;
  return (
    <section>
      <p className="view__muted" style={{ margin: "0 0 8px" }}>
        {runs.length} tick{runs.length === 1 ? "" : "s"} — {okCount} ok
        {errCount > 0 ? `, ${errCount} error${errCount === 1 ? "" : "s"}` : ""}
      </p>
      <table className="health-table">
        <thead>
          <tr>
            <th>timestamp</th>
            <th>status</th>
            <th>latency</th>
            <th>error</th>
          </tr>
        </thead>
        <tbody>
          {runs.map((r, i) => (
            <tr key={`${r.ts}-${i}`}>
              <td className="mono">{r.ts}</td>
              <td>
                <span
                  className={`health-pill health-pill--${r.status === "ok" ? "ok" : "critical"}`}
                >
                  {r.status}
                </span>
              </td>
              <td className="mono">
                {r.latency_ms !== null ? `${r.latency_ms}ms` : "—"}
              </td>
              <td className="mono">{r.error_message ?? "—"}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </section>
  );
}
