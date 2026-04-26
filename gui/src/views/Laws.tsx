import { useQuery } from "@tanstack/react-query";

import { Tag } from "../atoms/Tag";
import { api } from "../lib/api";

export function LawsView() {
  const lawsQ = useQuery({
    queryKey: ["laws"],
    queryFn: () => api.laws(),
    refetchInterval: 30_000,
  });
  const violationsQ = useQuery({
    queryKey: ["violations"],
    queryFn: () => api.violations(),
    refetchInterval: 10_000,
    refetchIntervalInBackground: true,
  });

  const laws = lawsQ.data ?? [];
  const violations = violationsQ.data ?? [];

  return (
    <div className="view">
      <div className="view__head">
        <div>
          <h1 className="view__title">Laws</h1>
          <p className="view__subtitle">
            Spec-13 catalogue + spec-14 enforcement. The catalogue endpoint
            (<span className="mono">/v1/dashboard/laws</span>) returns empty until spec-13 ships;
            <span className="mono"> /v1/dashboard/violations</span> tracks observed
            <span className="mono"> kind=law_violation</span> envelopes.
          </p>
        </div>
      </div>

      <div className="stats-grid" style={{ gridTemplateColumns: "repeat(3, 1fr)" }}>
        <Stat label="Active laws" value={String(laws.length)} sub="spec-13 catalogue" />
        <Stat
          label="Critical · 7d"
          value={String(violations.filter((v) => v.action === "blocked").length)}
          sub={`${violations.length} total`}
        />
        <Stat
          label="Annotated · 7d"
          value={String(violations.filter((v) => v.action === "annotated").length)}
          sub="recorded but not blocked"
        />
      </div>

      <section style={{ marginTop: 16 }}>
        <h2 style={{ fontSize: 13, color: "var(--fg-2)", marginBottom: 8 }}>Catalogue</h2>
        {lawsQ.error ? (
          <Empty msg="cortex-api unreachable." />
        ) : lawsQ.isLoading ? (
          <Empty msg="Loading laws…" />
        ) : laws.length === 0 ? (
          <Empty msg="The law catalogue is empty. Spec-13 will populate it." />
        ) : (
          <div className="law-table">
            {laws.map((law) => (
              <div key={law.id} className="law-row">
                <span className="mono" style={{ color: "var(--accent)", fontSize: 11 }}>
                  {law.id}
                </span>
                <span className="law-row__title">{law.title}</span>
                <Tag tone={law.severity === "critical" ? "critical" : law.severity === "notable" ? "warn" : "info"}>
                  {law.severity}
                </Tag>
                <span className="muted mono" style={{ fontSize: 10.5 }}>
                  {law.scope}
                </span>
                <span className="mono tabular" style={{ fontSize: 10.5 }}>
                  {law.violations_7d}/7d
                </span>
              </div>
            ))}
          </div>
        )}
      </section>

      <section style={{ marginTop: 16 }}>
        <h2 style={{ fontSize: 13, color: "var(--fg-2)", marginBottom: 8 }}>
          Recent violations
        </h2>
        {violationsQ.error ? (
          <Empty msg="cortex-api unreachable." />
        ) : violationsQ.isLoading ? (
          <Empty msg="Loading violations…" />
        ) : violations.length === 0 ? (
          <Empty msg="No violations observed yet." />
        ) : (
          <div className="violation-list">
            {violations.map((v) => (
              <div key={v.id} className="violation-row">
                <span className="mono" style={{ color: "var(--accent)" }}>
                  {v.id}
                </span>
                <Tag tone={v.action === "blocked" ? "critical" : "warn"}>{v.action}</Tag>
                <span className="muted">
                  {v.law_id ?? "—"} · {v.repo ?? "—"} · {v.at}
                </span>
                <pre className="violation-evidence">{v.evidence}</pre>
              </div>
            ))}
          </div>
        )}
      </section>
    </div>
  );
}

function Stat({ label, value, sub }: { label: string; value: string; sub?: string }) {
  return (
    <div className="stat">
      <div className="stat__label">{label}</div>
      <div className="stat__value tabular">{value}</div>
      {sub ? <div className="stat__delta">{sub}</div> : null}
    </div>
  );
}

function Empty({ msg }: { msg: string }) {
  return (
    <div
      style={{
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
