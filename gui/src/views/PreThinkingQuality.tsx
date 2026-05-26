import { useQuery } from "@tanstack/react-query";

import {
  api,
  type IntentByteQuantilesView,
  type IntentHelpfulRateView,
  type PreThinkingHealthReport,
} from "../lib/api";
import { useConnKey } from "../lib/connections/useConnKey";

// phase14f §4.3 — Pre-Thinking Quality view. Reads
// `/v1/health/pre-thinking` (extended in phase14f §4.1/§4.2 with
// per-intent bundle-bytes quantiles + helpful-rate counters) and
// renders three panels:
//   1) Breaker banner — current state + per-reason fail-open
//      counters carried over from phase14e.
//   2) Per-intent bundle-size table — p50 / p95 / p99 per intent.
//   3) Per-intent helpful-rate table — helpful, unhelpful, rate.
export function PreThinkingQualityView() {
  const connKey = useConnKey();
  const { data, isLoading, error } = useQuery({
    queryKey: [connKey, "pre-thinking-quality"],
    queryFn: () => api.preThinkingQuality(),
    refetchInterval: 30_000,
    refetchIntervalInBackground: true,
  });

  return (
    <div className="view">
      <div className="view__head">
        <div>
          <h1 className="view__title">Pre-Thinking Quality</h1>
          <p className="view__subtitle">
            Per-intent bundle-size distribution + helpful-rate +
            circuit-breaker state · phase14f
          </p>
        </div>
      </div>

      {error ? (
        <Empty msg="cortex-api unreachable." />
      ) : isLoading || !data ? (
        <Empty msg="Loading pre-thinking quality…" />
      ) : (
        <>
          <BreakerBanner report={data} />
          <PerIntentBytesTable report={data} />
          <PerIntentHelpfulTable report={data} />
        </>
      )}
    </div>
  );
}

function BreakerBanner({ report }: { report: PreThinkingHealthReport }) {
  const tone: "ok" | "warn" | "info" =
    report.breaker_state === "closed"
      ? "ok"
      : report.breaker_state === "open"
        ? "warn"
        : "info";
  const failCount =
    typeof report.fail_open_sum === "bigint"
      ? Number(report.fail_open_sum)
      : Number(report.fail_open_sum ?? 0);
  return (
    <div
      data-testid="pt-breaker-banner"
      style={{
        marginTop: 16,
        padding: "10px 14px",
        border: "1px solid var(--border)",
        borderRadius: "var(--radius-md)",
        display: "flex",
        gap: 18,
        alignItems: "center",
        flexWrap: "wrap",
        fontSize: 12,
      }}
    >
      <span className="muted" style={{ fontWeight: 600, letterSpacing: 0.4 }}>
        BREAKER
      </span>
      <span
        className="mono"
        style={{
          color: tone === "warn" ? "var(--err, #f88)" : tone === "ok" ? "var(--ok, #8c8)" : "var(--fg-2)",
          fontWeight: 600,
        }}
      >
        {report.breaker_state}
      </span>
      <span className="muted">
        failures_in_window: {String(report.failures_in_window ?? 0)}
      </span>
      <span className="muted">fail_open_sum: {failCount}</span>
      {Object.entries(report.fail_open_total ?? {}).map(([reason, n]) => (
        <span key={reason} className="muted mono" style={{ fontSize: 10.5 }}>
          {reason}={String(n)}
        </span>
      ))}
    </div>
  );
}

function PerIntentBytesTable({ report }: { report: PreThinkingHealthReport }) {
  const rows = Object.entries(report.bundle_bytes_per_intent ?? {}) as Array<
    [string, IntentByteQuantilesView]
  >;
  return (
    <Section
      title="Bundle bytes per intent (p50 / p95 / p99)"
      empty="No samples yet — pre-thinking has not run since boot."
      hasRows={rows.length > 0}
    >
      <table className="table" data-testid="pt-bytes-table">
        <thead>
          <tr>
            <th>intent</th>
            <th>count</th>
            <th>p50</th>
            <th>p95</th>
            <th>p99</th>
          </tr>
        </thead>
        <tbody>
          {rows.map(([intent, q]) => (
            <tr key={intent}>
              <td className="mono">{intent}</td>
              <td>{String(q.count)}</td>
              <td>{q.p50}</td>
              <td>{q.p95}</td>
              <td>{q.p99}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </Section>
  );
}

function PerIntentHelpfulTable({ report }: { report: PreThinkingHealthReport }) {
  const rows = Object.entries(report.helpful_rate_per_intent ?? {}) as Array<
    [string, IntentHelpfulRateView]
  >;
  return (
    <Section
      title="Helpful rate per intent"
      empty="No feedback recorded yet — POST /v1/pre-thinking/feedback to seed."
      hasRows={rows.length > 0}
    >
      <table className="table" data-testid="pt-helpful-table">
        <thead>
          <tr>
            <th>intent</th>
            <th>helpful</th>
            <th>unhelpful</th>
            <th>rate</th>
          </tr>
        </thead>
        <tbody>
          {rows.map(([intent, r]) => (
            <tr key={intent}>
              <td className="mono">{intent}</td>
              <td>{String(r.helpful)}</td>
              <td>{String(r.unhelpful)}</td>
              <td>
                {r.rate === null || r.rate === undefined
                  ? "—"
                  : `${(r.rate * 100).toFixed(1)}%`}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </Section>
  );
}

function Section({
  title,
  empty,
  hasRows,
  children,
}: {
  title: string;
  empty: string;
  hasRows: boolean;
  children: React.ReactNode;
}) {
  return (
    <section style={{ marginTop: 24 }}>
      <h2 style={{ fontSize: 14, margin: "0 0 8px 0" }}>{title}</h2>
      {hasRows ? children : <Empty msg={empty} />}
    </section>
  );
}

function Empty({ msg }: { msg: string }) {
  return (
    <div
      style={{
        marginTop: 12,
        padding: 24,
        border: "1px dashed var(--border)",
        borderRadius: "var(--radius-md)",
        color: "var(--fg-3)",
        textAlign: "center",
        fontSize: 12,
      }}
    >
      {msg}
    </div>
  );
}
