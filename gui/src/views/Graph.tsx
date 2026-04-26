import { useQuery } from "@tanstack/react-query";

import { Icon } from "../atoms/Icon";
import { api } from "../lib/api";

const NODE_COLOR: Record<string, string> = {
  session: "var(--info)",
  turn: "var(--info)",
  tool_call: "var(--accent)",
  artifact: "var(--fg-2)",
  decision: "var(--ok)",
  law: "var(--critical)",
  violation: "var(--critical)",
  analysis: "oklch(0.75 0.13 290)",
};

export function GraphView() {
  const { data, isLoading, error } = useQuery({
    queryKey: ["graph"],
    queryFn: () => api.graph(),
    refetchInterval: 12_000,
    refetchIntervalInBackground: true,
  });

  const W = 820;
  const H = 400;
  const nodes = data?.nodes ?? [];
  const edges = data?.edges ?? [];

  return (
    <div className="view">
      <div className="view__head">
        <div>
          <h1 className="view__title">Graph explorer</h1>
          <p className="view__subtitle">
            Session → Turn → ToolCall lineage derived from{" "}
            <span className="mono">/v1/dashboard/graph</span>. Decision / Law / Analysis nodes
            join in once spec-13/14/15 envelopes start flowing through capture.
          </p>
        </div>
        <div className="view__actions">
          <button className="btn">
            <Icon name="external" size={13} /> Open in Nexus
          </button>
        </div>
      </div>

      <div className="graph-wrap">
        <div className="graph-canvas">
          <div className="graph-legend">
            {Object.entries(NODE_COLOR).map(([kind, color]) => (
              <span key={kind} className="legend-item">
                <span className="legend-dot" style={{ background: color }} />
                {kind}
              </span>
            ))}
          </div>
          {error ? (
            <CenterMsg msg="cortex-api unreachable." />
          ) : isLoading ? (
            <CenterMsg msg="Loading graph…" />
          ) : nodes.length === 0 ? (
            <CenterMsg msg="No graph data yet. Capture a Claude Code session with the Cortex plugin to populate it." />
          ) : (
            <svg
              viewBox={`0 0 ${W} ${H}`}
              style={{ width: "100%", height: "100%" }}
              preserveAspectRatio="xMidYMid meet"
            >
              <defs>
                <marker
                  id="arr"
                  viewBox="0 0 10 10"
                  refX="9"
                  refY="5"
                  markerWidth="6"
                  markerHeight="6"
                  orient="auto"
                >
                  <path d="M0,0 L10,5 L0,10 z" fill="var(--fg-3)" />
                </marker>
              </defs>
              {edges.map((e, i) => {
                const a = nodes.find((n) => n.id === e.from);
                const b = nodes.find((n) => n.id === e.to);
                if (!a || !b) return null;
                return (
                  <g key={`${e.from}-${e.to}-${i}`}>
                    <line
                      x1={a.x}
                      y1={a.y}
                      x2={b.x}
                      y2={b.y}
                      stroke="var(--border-strong)"
                      strokeWidth="1.2"
                      markerEnd="url(#arr)"
                    />
                    <text
                      x={(a.x + b.x) / 2}
                      y={(a.y + b.y) / 2 - 4}
                      textAnchor="middle"
                      fill="var(--fg-3)"
                      fontSize="9"
                      fontFamily="var(--font-mono)"
                    >
                      {e.label}
                    </text>
                  </g>
                );
              })}
              {nodes.map((n) => (
                <g key={n.id} transform={`translate(${n.x},${n.y})`}>
                  <circle r="14" fill="var(--bg-2)" stroke={NODE_COLOR[n.kind] ?? "var(--fg-2)"} strokeWidth="1.8" />
                  <circle r="4" fill={NODE_COLOR[n.kind] ?? "var(--fg-2)"} />
                  <text
                    y="28"
                    textAnchor="middle"
                    fill="var(--fg-1)"
                    fontSize="10"
                    fontFamily="var(--font-mono)"
                  >
                    {n.label}
                  </text>
                </g>
              ))}
            </svg>
          )}
        </div>
        <div className="card">
          <div className="card__head">
            <span className="card__title">Selection</span>
          </div>
          <div className="card__body">
            <dl className="kv-list">
              <dt>nodes</dt>
              <dd className="mono">{nodes.length}</dd>
              <dt>edges</dt>
              <dd className="mono">{edges.length}</dd>
              <dt>source</dt>
              <dd className="mono">archive_loader</dd>
              <dt>refresh</dt>
              <dd className="mono">12 s</dd>
            </dl>
          </div>
        </div>
      </div>
    </div>
  );
}

function CenterMsg({ msg }: { msg: string }) {
  return (
    <div
      style={{
        position: "absolute",
        inset: 0,
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
        color: "var(--fg-3)",
        fontSize: 12,
        textAlign: "center",
        padding: 32,
      }}
    >
      {msg}
    </div>
  );
}
