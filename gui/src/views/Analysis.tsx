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
  const concluded = rows.filter((a) => a.status === "concluded").length;
  const promoted = rows.filter((a) => !!a.decision_id).length;
  const totalRounds = rows.reduce((s, a) => s + (a.rounds ?? 0), 0);
  const avgRounds = rows.length ? (totalRounds / rows.length).toFixed(1) : "0";

  // An imported audit (phase4e) carries a `source_path` and is keyed
  // by a bootstrap event id; spec-15 deep-analyses do not. The two
  // shapes share the same surface but the imports are pure documents
  // (no panel / judge / rounds / duration), so we relabel the empty-
  // state and stat copy depending on what's present.
  const imports = rows.filter((a) => !!a.source_path).length;
  const debates = rows.length - imports;
  const subtitleSegments: string[] = [];
  if (imports > 0) {
    subtitleSegments.push(`${imports} imported audit${imports === 1 ? "" : "s"} (docs/analysis/)`);
  }
  if (debates > 0) {
    subtitleSegments.push(`${debates} spec-15 debate${debates === 1 ? "" : "s"}`);
  }
  const subtitleSummary =
    subtitleSegments.length > 0
      ? subtitleSegments.join(" · ")
      : "kind=analysis envelopes — bootstrap imports + spec-15 debates land here";

  return (
    <div className="view">
      <div className="view__head">
        <div>
          <h1 className="view__title">Analysis library</h1>
          <p className="view__subtitle">{subtitleSummary}</p>
        </div>
      </div>

      <div className="stats-grid" style={{ gridTemplateColumns: "repeat(3, 1fr)" }}>
        <Stat label="Captured" value={String(rows.length)} sub={`${concluded} concluded · ${imports} imports`} />
        <Stat label="Promoted" value={String(promoted)} sub="linked to a decision" />
        <Stat label="Avg rounds" value={avgRounds} sub="across captured analyses" />
      </div>

      {error ? (
        <Empty msg="cortex-api unreachable." />
      ) : isLoading ? (
        <Empty msg="Loading analyses…" />
      ) : rows.length === 0 ? (
        <Empty msg="No analyses captured yet. Run cortex-bootstrap to ingest docs/analysis/**, or wait for spec-15 deep-analysis envelopes." />
      ) : (
        rows.map((a) => {
          const isImport = !!a.source_path;
          return (
            <article key={a.id} className="analysis-card">
              <div>
                <div style={{ display: "flex", alignItems: "center", gap: 10, marginBottom: 4, flexWrap: "wrap" }}>
                  <span className="mono" style={{ fontSize: 11, color: "var(--accent)", fontWeight: 600 }}>
                    {a.id}
                  </span>
                  <Tag tone={a.status === "concluded" ? "ok" : "default"}>{a.status}</Tag>
                  {a.repo ? <Tag tone="default">{a.repo}</Tag> : null}
                  {isImport ? <Tag tone="default">imported</Tag> : null}
                  <span className="mono" style={{ fontSize: 10.5, color: "var(--fg-3)", marginLeft: "auto" }}>
                    {a.occurred_at}
                  </span>
                </div>
                <h3 className="analysis-card__title">{a.title}</h3>
                {isImport ? (
                  <div
                    className="mono"
                    style={{ fontSize: 11, color: "var(--fg-3)", marginBottom: 6 }}
                    title={a.source_path}
                  >
                    {a.source_path}
                  </div>
                ) : (
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
                )}
                <div className="analysis-card__verdict">{a.verdict}</div>
              </div>
              <div className="analysis-panel">
                <div style={{ color: "var(--fg-3)", fontSize: 10, textTransform: "uppercase", letterSpacing: "0.08em" }}>
                  {isImport ? "Source" : "Panel"}
                </div>
                {isImport ? (
                  <div className="muted" style={{ fontSize: 11 }}>
                    bootstrap import — no debate panel
                  </div>
                ) : a.panel.length === 0 ? (
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
          );
        })
      )}
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
