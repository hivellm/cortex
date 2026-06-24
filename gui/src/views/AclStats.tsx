import { useQuery } from "@tanstack/react-query";

import { api } from "../lib/api";
import { useConnKey } from "../lib/connections/useConnKey";

const LEVEL_LABELS: Record<number, string> = {
  0: "public",
  1: "internal",
  2: "confidential",
  3: "restricted",
};

function levelLabel(n: number): string {
  return LEVEL_LABELS[n] ?? String(n);
}

/**
 * ACL audit statistics dashboard view (Phase21 §8.2).
 *
 * Surfaces three panels from `/v1/dashboard/acl-stats`:
 * - Total evaluated / granted / denied counters since boot.
 * - Classification distribution per repo (from the keyword lane).
 * - Top-10 denied principals (by cumulative denial count).
 *
 * When AC is disabled the endpoint returns all-zero stubs; this view
 * renders a "no data" notice rather than an empty table.
 */
export function AclStatsView() {
  const connKey = useConnKey();
  const { data, isLoading, error } = useQuery({
    queryKey: [connKey, "aclStats"],
    queryFn: () => api.aclStats(),
    refetchInterval: 30_000,
    refetchIntervalInBackground: true,
  });

  if (isLoading && !data) {
    return (
      <div className="view">
        <div className="view__head">
          <h2>Access Control</h2>
        </div>
        <p className="view__muted">Loading…</p>
      </div>
    );
  }

  if (error) {
    return (
      <div className="view">
        <div className="view__head">
          <h2>Access Control</h2>
        </div>
        <p className="view__error">Failed to load ACL stats.</p>
      </div>
    );
  }

  if (!data) {
    return null;
  }

  const noActivity = data.total_evaluated === 0;

  return (
    <div className="view">
      <div className="view__head">
        <h2>Access Control</h2>
        {noActivity && (
          <span className="view__muted">
            No access decisions recorded — AC may be disabled.
          </span>
        )}
      </div>

      {/* Totals ribbon */}
      <div className="view__stats-row">
        <span className="view__stat">
          <strong>{data.total_evaluated}</strong> evaluated
        </span>
        <span className="view__stat">
          <strong>{data.total_granted}</strong> granted
        </span>
        <span className="view__stat">
          <strong>{data.total_denied}</strong> denied
        </span>
      </div>

      {/* Classification distribution */}
      <h3>Classification distribution</h3>
      {data.classification_distribution.length === 0 ? (
        <p className="view__muted">No classified hits in the keyword lane.</p>
      ) : (
        <table className="data-table">
          <thead>
            <tr>
              <th>Repo</th>
              <th>Level</th>
              <th>Count</th>
            </tr>
          </thead>
          <tbody>
            {data.classification_distribution.map((row, i) => (
              <tr key={i}>
                <td style={{ fontFamily: "var(--font-mono)" }}>{row.repo}</td>
                <td>
                  <span
                    className="view__pill"
                    data-level={row.level}
                    data-state={row.level >= 2 ? "warn" : "ok"}
                  >
                    {levelLabel(row.level)}
                  </span>
                </td>
                <td>{row.count}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}

      {/* Top denied principals */}
      <h3>Top denied principals</h3>
      {data.top_denied_principals.length === 0 ? (
        <p className="view__muted">No denials recorded.</p>
      ) : (
        <table className="data-table">
          <thead>
            <tr>
              <th>Principal</th>
              <th>Denials</th>
            </tr>
          </thead>
          <tbody>
            {data.top_denied_principals.map((p) => (
              <tr key={p.principal_id}>
                <td style={{ fontFamily: "var(--font-mono)" }}>
                  {p.principal_id}
                </td>
                <td>{p.denial_count}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </div>
  );
}
