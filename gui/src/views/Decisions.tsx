import { useMemo, useState } from "react";
import { useQuery } from "@tanstack/react-query";

import { Icon } from "../atoms/Icon";
import { Tag } from "../atoms/Tag";
import { api, type DecisionChainNode } from "../lib/api";

export function DecisionsView() {
  const [showSuperseded, setShowSuperseded] = useState(false);

  const { data, isLoading, error } = useQuery({
    queryKey: ["decisions"],
    queryFn: () => api.decisions(),
    refetchInterval: 10_000,
    refetchIntervalInBackground: true,
  });

  const rowsRaw = data ?? [];
  const active = rowsRaw.filter((d) => d.status === "active").length;
  const superseded = rowsRaw.filter((d) => d.status === "superseded").length;
  const withRationale = rowsRaw.filter((d) => !!d.rationale).length;

  const rows = useMemo(
    () =>
      showSuperseded ? rowsRaw : rowsRaw.filter((d) => d.status !== "superseded"),
    [rowsRaw, showSuperseded],
  );

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
        <div className="view__actions">
          <button
            className={`btn ${showSuperseded ? "" : "btn--ghost"}`}
            onClick={() => setShowSuperseded((s) => !s)}
            title={
              showSuperseded
                ? "Hide superseded decisions"
                : `Show ${superseded} superseded decision${superseded === 1 ? "" : "s"}`
            }
          >
            {showSuperseded ? "✓ " : ""}Show superseded
          </button>
        </div>
      </div>

      <div className="stats-grid" style={{ gridTemplateColumns: "repeat(3, 1fr)" }}>
        <Stat label="Total decisions" value={String(rows.length)} sub="captured envelopes" />
        <Stat label="Active" value={String(active)} sub={`${superseded} superseded`} />
        <Stat label="With rationale" value={String(withRationale)} sub="non-empty body" />
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
            <article
              key={d.id}
              className={`decision-card ${d.status === "superseded" ? "is-superseded" : ""}`}
            >
              <header className="decision-card__head">
                <span className="mono" style={{ color: "var(--accent)", fontWeight: 600 }}>
                  {d.id}
                </span>
                <Tag tone={d.status === "active" ? "ok" : "default"}>{d.status}</Tag>
                {d.supersedes ? (
                  <Tag tone="warn">supersedes {d.supersedes}</Tag>
                ) : null}
                {d.superseded_by ? (
                  <Tag>superseded → {d.superseded_by}</Tag>
                ) : null}
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
                    #{t}
                  </span>
                ))}
                {d.cites.length > 0 ? (
                  <span className="muted mono" style={{ fontSize: 10.5 }}>
                    cites: {d.cites.join(", ")}
                  </span>
                ) : null}
              </footer>
              {d.chain && d.chain.length > 1 ? <SupersedeChain chain={d.chain} /> : null}
            </article>
          ))}
        </div>
      )}
    </div>
  );
}

function SupersedeChain({ chain }: { chain: DecisionChainNode[] }) {
  return (
    <div className="supersede-chain">
      {chain.map((c, i) => (
        <span key={c.id} style={{ display: "contents" }}>
          <div
            className={`supersede-node ${
              c.state === "current" ? "is-current" : "is-old"
            }`}
          >
            <span className="mono" style={{ fontSize: 10.5, color: "var(--fg-3)" }}>
              {c.id}
            </span>
            <span style={{ fontSize: 11.5, color: "var(--fg-1)" }}>{c.title}</span>
          </div>
          {i < chain.length - 1 ? (
            <span className="supersede-arrow">
              <Icon name="arrow-right" size={14} />
            </span>
          ) : null}
        </span>
      ))}
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
