import { useQuery } from "@tanstack/react-query";

import { Tag } from "../atoms/Tag";
import { api } from "../lib/api";

export function AnalysisView() {
  const { data, isLoading, error } = useQuery({
    queryKey: ["analyses"],
    queryFn: () => api.analyses(),
    refetchInterval: 15_000,
  });
  const rows = data ?? [];

  return (
    <div className="view">
      <div className="view__head">
        <div>
          <h1 className="view__title">Analysis library</h1>
          <p className="view__subtitle">
            Structured 2–5 agent debates · judged · promoted to decisions. Sourced from
            <span className="mono"> kind=analysis</span> envelopes; populated when spec-15
            ships the deep-analysis workflow.
          </p>
        </div>
      </div>

      {error ? (
        <Empty msg="cortex-api unreachable." />
      ) : isLoading ? (
        <Empty msg="Loading analyses…" />
      ) : rows.length === 0 ? (
        <Empty msg="No analyses captured yet. Spec-15's analysis envelopes populate this view as they land." />
      ) : (
        rows.map((a) => (
          <article key={a.id} className="analysis-card">
            <div>
              <div style={{ display: "flex", alignItems: "center", gap: 10, marginBottom: 4 }}>
                <span className="mono" style={{ fontSize: 11, color: "var(--accent)", fontWeight: 600 }}>
                  {a.id}
                </span>
                <Tag tone={a.status === "concluded" ? "ok" : "default"}>{a.status}</Tag>
                <span className="mono" style={{ fontSize: 10.5, color: "var(--fg-3)", marginLeft: "auto" }}>
                  {a.occurred_at}
                </span>
              </div>
              <h3 className="analysis-card__title">{a.title}</h3>
              <div
                style={{
                  display: "flex",
                  gap: 10,
                  fontSize: 11.5,
                  color: "var(--fg-3)",
                  fontFamily: "var(--font-mono)",
                }}
              >
                <span>{a.rounds} rounds</span>
                <span>·</span>
                <span>{a.duration_s}s</span>
                <span>·</span>
                <span>
                  judge: <span style={{ color: "var(--fg-1)" }}>{a.judge || "—"}</span>
                </span>
                {a.decision_id ? (
                  <>
                    <span>·</span>
                    <span>
                      → <span style={{ color: "var(--accent)" }}>{a.decision_id}</span>
                    </span>
                  </>
                ) : null}
              </div>
              <div className="analysis-card__verdict">{a.verdict}</div>
            </div>
            <div className="analysis-panel">
              <div style={{ color: "var(--fg-3)", fontSize: 10, textTransform: "uppercase", letterSpacing: "0.08em" }}>
                Panel
              </div>
              {a.panel.length === 0 ? (
                <div className="muted" style={{ fontSize: 11 }}>
                  no panelists recorded
                </div>
              ) : (
                a.panel.map((p) => (
                  <div key={p} className="panelist-row">
                    <span className="panelist-dot" />
                    <span style={{ color: "var(--fg-1)" }}>{p}</span>
                  </div>
                ))
              )}
            </div>
          </article>
        ))
      )}
    </div>
  );
}

function Empty({ msg }: { msg: string }) {
  return (
    <div
      style={{
        marginTop: 24,
        padding: 32,
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
