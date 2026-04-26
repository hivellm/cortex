/* Cortex — Analysis, Tools, Graph views */

const AnalysisView = () => (
  <div className="view">
    <div className="view__head">
      <div>
        <h1 className="view__title">Analysis library</h1>
        <p className="view__subtitle">Structured 2–5 agent debates · judged · promoted to decisions</p>
      </div>
      <div className="view__actions">
        <button className="btn"><Icon name="external" size={13}/> Replay transcript</button>
        <button className="btn btn--primary"><Icon name="analysis" size={13}/> Start analysis</button>
      </div>
    </div>

    <div className="stats-grid" style={{ gridTemplateColumns: "repeat(3, 1fr)" }}>
      <div className="stat">
        <div className="stat__label">Concluded · 30d</div>
        <div className="stat__value tabular">14</div>
        <div className="stat__delta is-up">▲ 11 promoted to decisions</div>
      </div>
      <div className="stat">
        <div className="stat__label">Avg rounds · cost</div>
        <div className="stat__value tabular">2.7<span className="stat__unit">rds · $0.84</span></div>
        <div className="stat__delta">budget cap $5 / run</div>
      </div>
      <div className="stat">
        <div className="stat__label">Top-5 judge precision</div>
        <div className="stat__value tabular">0.78</div>
        <div className="stat__delta is-up">▲ above 0.7 floor</div>
      </div>
    </div>

    {MOCK.analyses.map(a => (
      <article key={a.id} className="analysis-card">
        <div>
          <div style={{ display: "flex", alignItems: "center", gap: 10, marginBottom: 4 }}>
            <span className="mono" style={{ fontSize: 11, color: "var(--accent)", fontWeight: 600 }}>{a.id}</span>
            <Tag tone="ok">{a.status}</Tag>
            <span className="mono" style={{ fontSize: 10.5, color: "var(--fg-3)", marginLeft: "auto" }}>{a.occurredAt}</span>
          </div>
          <h3 className="analysis-card__title">{a.title}</h3>
          <div style={{ display: "flex", gap: 10, fontSize: 11.5, color: "var(--fg-3)", fontFamily: "var(--font-mono)" }}>
            <span>{a.rounds} rounds</span>
            <span>·</span>
            <span>{a.durationS}s</span>
            <span>·</span>
            <span>judge: <span style={{ color: "var(--fg-1)" }}>{a.judge}</span></span>
            {a.decisionId && (<><span>·</span><span>→ <span style={{ color: "var(--accent)" }}>{a.decisionId}</span></span></>)}
          </div>
          <div className="analysis-card__verdict">{a.verdict}</div>
        </div>
        <div className="analysis-panel">
          <div style={{ color: "var(--fg-3)", fontSize: 10, textTransform: "uppercase", letterSpacing: "0.08em" }}>Panel</div>
          {a.panel.map(p => (
            <div key={p} className="panelist-row">
              <span className="panelist-dot"/>
              <span style={{ color: "var(--fg-1)" }}>{p}</span>
            </div>
          ))}
        </div>
      </article>
    ))}
  </div>
);

const ToolsView = () => {
  const max = Math.max(...MOCK.toolUsage.map(t => t.calls));
  const heat = Array.from({ length: 7 }, (_, day) => Array.from({ length: 24 }, (_, h) => {
    const v = (Math.sin(day * 1.2 + h * 0.4) * 0.5 + 0.5) * (h > 7 && h < 22 ? 1 : 0.3);
    return v;
  }));
  return (
    <div className="view">
      <div className="view__head">
        <div>
          <h1 className="view__title">Tool analytics</h1>
          <p className="view__subtitle">Heatmap, latency, error rate · last 7 days · all adapters</p>
        </div>
        <div className="view__actions">
          <button className="btn">last 7d ▾</button>
          <button className="btn"><Icon name="external" size={13}/> Export</button>
        </div>
      </div>

      <div className="card" style={{ marginBottom: 16 }}>
        <div className="card__head">
          <span className="card__title">Tool usage breakdown</span>
          <span className="card__sub">{fmtNum(MOCK.toolUsage.reduce((s, t) => s + t.calls, 0))} calls · 7d</span>
        </div>
        <div>
          <div className="tool-row" style={{ background: "var(--bg-2)", color: "var(--fg-3)", fontFamily: "var(--font-mono)", fontSize: 10.5, textTransform: "uppercase", letterSpacing: "0.06em" }}>
            <span>Tool</span>
            <span style={{ textAlign: "right" }}>Calls</span>
            <span style={{ textAlign: "right" }}>Avg ms</span>
            <span style={{ textAlign: "right" }}>Err rate</span>
            <span>Share</span>
          </div>
          {MOCK.toolUsage.map(t => (
            <div key={t.tool} className="tool-row">
              <span className="tool-name">
                <span className="tool-icon">{t.icon}</span>
                {t.tool}
              </span>
              <span className="mono tabular" style={{ textAlign: "right", color: "var(--fg-1)" }}>{fmtNum(t.calls)}</span>
              <span className="mono tabular" style={{ textAlign: "right", color: t.avgMs > 1000 ? "var(--warn)" : "var(--fg-1)" }}>{t.avgMs}</span>
              <span className="mono tabular" style={{ textAlign: "right", color: t.errRate > 0.05 ? "var(--critical)" : t.errRate > 0.02 ? "var(--warn)" : "var(--fg-2)" }}>
                {(t.errRate * 100).toFixed(1)}%
              </span>
              <span className="tool-bar">
                <span className="tool-bar__track">
                  <span className="tool-bar__fill" style={{ width: `${(t.calls / max) * 100}%` }}/>
                </span>
                <span className="mono tabular" style={{ fontSize: 10.5, color: "var(--fg-3)", width: 40, textAlign: "right" }}>{(t.share * 100).toFixed(1)}%</span>
              </span>
            </div>
          ))}
        </div>
      </div>

      <div className="card">
        <div className="card__head">
          <span className="card__title">Tool-call density · day × hour</span>
          <span className="card__sub">UTC · darker = fewer calls</span>
        </div>
        <div className="card__body">
          <div className="heat-grid">
            <div></div>
            {Array.from({ length: 24 }, (_, h) => (
              <div key={h} style={{ fontSize: 9.5, color: "var(--fg-4)", textAlign: "center", fontFamily: "var(--font-mono)" }}>{h % 3 === 0 ? h : ""}</div>
            ))}
            {["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"].map((d, di) => (
              <React.Fragment key={d}>
                <div className="heat-row-label">{d}</div>
                {heat[di].map((v, hi) => {
                  const intensity = v;
                  const bg = `oklch(from var(--accent) calc(0.22 + ${intensity * 0.5}) calc(c * ${intensity}) h / ${0.15 + intensity * 0.85})`;
                  return <div key={hi} className="heat-cell" style={{ background: bg }}/>;
                })}
              </React.Fragment>
            ))}
          </div>
        </div>
      </div>
    </div>
  );
};

const GraphView = () => {
  const { nodes, edges } = MOCK.graph;
  const W = 820, H = 400;
  const nodeColor = {
    session: "var(--info)",
    turn: "var(--info)",
    tool_call: "var(--accent)",
    artifact: "var(--fg-2)",
    decision: "var(--ok)",
    law: "var(--critical)",
    violation: "var(--critical)",
    analysis: "oklch(0.75 0.13 290)"
  };
  return (
    <div className="view">
      <div className="view__head">
        <div>
          <h1 className="view__title">Graph explorer</h1>
          <p className="view__subtitle">Nexus relations · Cypher console · expand neighborhoods 1–2 hops</p>
        </div>
        <div className="view__actions">
          <button className="btn"><Icon name="external" size={13}/> Open in Nexus</button>
          <button className="btn btn--primary">Run query</button>
        </div>
      </div>

      <div className="card" style={{ marginBottom: 12 }}>
        <div className="card__body" style={{ padding: 10 }}>
          <pre className="code-block" style={{ margin: 0 }}>
{`MATCH (s:Session)-[:CONTAINS]->(t:Turn)-[:INVOKED]->(tc:ToolCall)-[:WROTE|READ]->(a:Artifact)
OPTIONAL MATCH (a)<-[:REFERENCES]-(d:Decision)
OPTIONAL MATCH (tc)<-[:OBSERVED_IN]-(v:LawViolation)-[:OF]->(l:Law)
WHERE s.id = $session_id
RETURN s, t, tc, a, d, v, l LIMIT 50`}
          </pre>
        </div>
      </div>

      <div className="graph-wrap">
        <div className="graph-canvas">
          <div className="graph-toolbar">
            <button className="btn btn--sm">Fit</button>
            <button className="btn btn--sm">+</button>
            <button className="btn btn--sm">−</button>
          </div>
          <div className="graph-legend">
            {[["session","Session"],["tool_call","ToolCall"],["artifact","Artifact"],["decision","Decision"],["analysis","Analysis"],["law","Law/Violation"]].map(([k,l])=>(
              <span key={k} className="legend-item"><span className="legend-dot" style={{background: nodeColor[k]}}/>{l}</span>
            ))}
          </div>
          <svg viewBox={`0 0 ${W} ${H}`} style={{ width: "100%", height: "100%" }} preserveAspectRatio="xMidYMid meet">
            <defs>
              <marker id="arr" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="6" markerHeight="6" orient="auto">
                <path d="M0,0 L10,5 L0,10 z" fill="var(--fg-3)"/>
              </marker>
            </defs>
            {edges.map((e, i) => {
              const a = nodes.find(n => n.id === e.from);
              const b = nodes.find(n => n.id === e.to);
              if (!a || !b) return null;
              return (
                <g key={i}>
                  <line x1={a.x} y1={a.y} x2={b.x} y2={b.y} stroke="var(--border-strong)" strokeWidth="1.2" markerEnd="url(#arr)"/>
                  <text x={(a.x + b.x) / 2} y={(a.y + b.y) / 2 - 4} textAnchor="middle" fill="var(--fg-3)" fontSize="9" fontFamily="var(--font-mono)">{e.label}</text>
                </g>
              );
            })}
            {nodes.map(n => (
              <g key={n.id} transform={`translate(${n.x},${n.y})`}>
                <circle r="14" fill="var(--bg-2)" stroke={nodeColor[n.kind] || "var(--fg-2)"} strokeWidth="1.8"/>
                <circle r="4" fill={nodeColor[n.kind] || "var(--fg-2)"}/>
                <text y="28" textAnchor="middle" fill="var(--fg-1)" fontSize="10" fontFamily="var(--font-mono)">{n.label}</text>
              </g>
            ))}
          </svg>
        </div>
        <div className="card">
          <div className="card__head">
            <span className="card__title">Selection</span>
          </div>
          <div className="card__body">
            <div style={{ fontSize: 11.5, color: "var(--fg-3)", marginBottom: 10 }}>Click a node to inspect.</div>
            <dl className="kv-list">
              <dt>nodes</dt><dd className="mono">{nodes.length}</dd>
              <dt>edges</dt><dd className="mono">{edges.length}</dd>
              <dt>scope</dt><dd className="mono">ses_8a3f</dd>
              <dt>depth</dt><dd className="mono">2</dd>
              <dt>runtime</dt><dd className="mono">38 ms</dd>
            </dl>
            <div className="divider"/>
            <div style={{ fontSize: 11, color: "var(--fg-3)", fontFamily: "var(--font-mono)" }}>
              Saved queries: <span style={{ color: "var(--accent)" }}>decisions_for_file</span>, <span style={{ color: "var(--accent)" }}>violations_per_session</span>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
};

window.AnalysisView = AnalysisView;
window.ToolsView = ToolsView;
window.GraphView = GraphView;
