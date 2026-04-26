import { useQuery } from "@tanstack/react-query";

import { Tag } from "../atoms/Tag";
import { api } from "../lib/api";

export function DecisionsView() {
  const { data, isLoading, error } = useQuery({
    queryKey: ["decisions"],
    queryFn: () => api.decisions(),
    refetchInterval: 10_000,
    refetchIntervalInBackground: true,
  });

  const rows = data ?? [];

  return (
    <div className="view">
      <div className="view__head">
        <div>
          <h1 className="view__title">Decisions</h1>
          <p className="view__subtitle">
            Architectural records derived from <span className="mono">kind=decision</span> envelopes.
            Promotion and supersession arrive when spec-15 (deep analysis) starts emitting them.
          </p>
        </div>
      </div>

      {error ? (
        <Empty msg="cortex-api unreachable. Start it with cargo run -p cortex-api." />
      ) : isLoading ? (
        <Empty msg="Loading decisions…" />
      ) : rows.length === 0 ? (
        <Empty msg="No decisions captured yet. The Cortex graph populates this list as decision envelopes flow in." />
      ) : (
        <div className="decision-list">
          {rows.map((d) => (
            <article key={d.id} className="decision-card">
              <header className="decision-card__head">
                <span className="mono" style={{ color: "var(--accent)", fontWeight: 600 }}>
                  {d.id}
                </span>
                <Tag tone={d.status === "active" ? "ok" : "default"}>{d.status}</Tag>
                <span className="muted mono" style={{ marginLeft: "auto", fontSize: 10.5 }}>
                  {d.occurred_at}
                </span>
              </header>
              <h3 className="decision-card__title">{d.title}</h3>
              {d.rationale ? (
                <p className="decision-card__rationale">{d.rationale}</p>
              ) : null}
              <footer className="decision-card__footer">
                {d.tags.map((t) => (
                  <span key={t} className="memory-topic">
                    {t}
                  </span>
                ))}
                {d.cites.length > 0 ? (
                  <span className="muted mono" style={{ fontSize: 10.5 }}>
                    cites: {d.cites.join(", ")}
                  </span>
                ) : null}
                {d.supersedes ? (
                  <span className="muted mono" style={{ fontSize: 10.5 }}>
                    supersedes <span style={{ color: "var(--accent)" }}>{d.supersedes}</span>
                  </span>
                ) : null}
              </footer>
            </article>
          ))}
        </div>
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
