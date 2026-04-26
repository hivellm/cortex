/* Cortex — Memory, Decisions, Laws views */

const MemoryView = () => {
  const [kind, setKind] = React.useState("all");
  const [q, setQ] = React.useState("");
  const kinds = ["all", "project", "reference", "feedback", "user"];
  const filtered = MOCK.memories.filter(m => {
    if (kind !== "all" && m.kind !== kind) return false;
    if (q && !(m.title + " " + m.excerpt + " " + m.topics.join(" ")).toLowerCase().includes(q.toLowerCase())) return false;
    return true;
  });
  return (
    <div className="view">
      <div className="view__head">
        <div>
          <h1 className="view__title">Memory browser</h1>
          <p className="view__subtitle">Searchable, faceted memories federated from <span className="mono">CLAUDE.md</span>, Cursor rules, Rulebook KV</p>
        </div>
        <div className="view__actions">
          <button className="btn"><Icon name="external" size={13}/> Export</button>
          <button className="btn btn--primary"><Icon name="memory" size={13}/> New memory</button>
        </div>
      </div>

      <div className="filter-bar">
        <span className="filter-bar__label">Kind</span>
        <span className="chip-group">
          {kinds.map(k => (
            <button key={k} className={`chip ${kind === k ? "is-active" : ""}`} onClick={() => setKind(k)}>{k}</button>
          ))}
        </span>
        <span className="filter-divider"/>
        <input
          placeholder="Search memories…"
          value={q}
          onChange={e => setQ(e.target.value)}
          style={{ flex: 1, minWidth: 200, height: 26, padding: "0 10px", background: "var(--bg-2)", border: "1px solid var(--border)", borderRadius: 6, color: "var(--fg-0)", fontSize: 11.5, outline: "none" }}
        />
      </div>

      <div className="memory-grid">
        {filtered.map((m, i) => (
          <article key={i} className="memory">
            <div className="memory__head">
              <span className="memory__kind">{m.kind}</span>
              <span style={{ marginLeft: "auto", fontFamily: "var(--font-mono)", fontSize: 10.5, color: "var(--fg-4)" }}>{m.updated}</span>
            </div>
            <div className="memory__title">{m.title}</div>
            <div className="memory__excerpt">{m.excerpt}</div>
            <div className="memory__foot">
              <Tag tone="solid">{m.repo}</Tag>
              {m.topics.map(t => <Tag key={t}>#{t}</Tag>)}
            </div>
          </article>
        ))}
      </div>
    </div>
  );
};

const DecisionsView = () => {
  const [showSuperseded, setShowSuperseded] = React.useState(false);
  const list = MOCK.decisions.filter(d => showSuperseded || d.status !== "superseded");
  return (
    <div className="view">
      <div className="view__head">
        <div>
          <h1 className="view__title">Decision register</h1>
          <p className="view__subtitle">ADR-style decisions · supersedable · cited from pre-thinking bundles</p>
        </div>
        <div className="view__actions">
          <button className={`btn ${showSuperseded ? "" : "btn--ghost"}`} onClick={() => setShowSuperseded(!showSuperseded)}>
            {showSuperseded ? "✓ " : ""}Show superseded
          </button>
          <button className="btn btn--primary"><Icon name="decision" size={13}/> Promote candidate</button>
        </div>
      </div>

      <div className="stats-grid" style={{ gridTemplateColumns: "repeat(3, 1fr)" }}>
        <div className="stat">
          <div className="stat__label">Active decisions</div>
          <div className="stat__value tabular">87</div>
          <div className="stat__delta">across 17 repos</div>
        </div>
        <div className="stat">
          <div className="stat__label">Cited in last 7d</div>
          <div className="stat__value tabular">412<span className="stat__unit">cites</span></div>
          <div className="stat__delta is-up">▲ adherence rate 0.82</div>
        </div>
        <div className="stat">
          <div className="stat__label">Awaiting promotion</div>
          <div className="stat__value tabular">6<span className="stat__unit">candidates</span></div>
          <div className="stat__delta">from session summaries</div>
        </div>
      </div>

      <div className="decision-list">
        {list.map(d => (
          <article key={d.id} className={`decision ${d.status === "superseded" ? "is-superseded" : ""}`}>
            <div className="decision__head">
              <span className="decision__id">{d.id}</span>
              <span className="decision__title">{d.title}</span>
              {d.status === "active" && <Tag tone="ok">active</Tag>}
              {d.status === "superseded" && <Tag>superseded → {d.supersededBy}</Tag>}
              {d.supersedes && <Tag tone="warn">supersedes {d.supersedes}</Tag>}
            </div>
            <p className="decision__rationale">{d.rationale}</p>
            <div className="decision__foot">
              <span>by <span className="strong">{d.author}</span></span>
              <span>·</span>
              {d.sourceAnalysis && <><span>from <span style={{ color: "var(--accent)" }}>{d.sourceAnalysis}</span></span><span>·</span></>}
              <span>{d.occurredAt}</span>
              <span>·</span>
              <span>{d.cites.map((c, i) => <span key={i} style={{ marginRight: 6 }}>{c}</span>)}</span>
              <span style={{ marginLeft: "auto", display: "flex", gap: 6 }}>
                {d.tags.map(t => <Tag key={t}>#{t}</Tag>)}
              </span>
            </div>
            {d.chain && (
              <div className="supersede-chain">
                {d.chain.map((c, i) => (
                  <React.Fragment key={c.id}>
                    <div className={`supersede-node ${c.state === "current" ? "is-current" : c.state === "old" ? "is-old" : ""}`}>
                      <span className="mono" style={{ fontSize: 10.5, color: "var(--fg-3)" }}>{c.id}</span>
                      <span style={{ fontSize: 11.5, color: "var(--fg-1)" }}>{c.title}</span>
                    </div>
                    {i < d.chain.length - 1 && <span className="supersede-arrow"><Icon name="arrow-right" size={14}/></span>}
                  </React.Fragment>
                ))}
              </div>
            )}
          </article>
        ))}
      </div>
    </div>
  );
};

const LawsView = ({ onSelect }) => {
  const [activeId, setActiveId] = React.useState(null);
  const open = (law) => { setActiveId(law.id); onSelect({ kind: "law", law }); };
  return (
    <div className="view">
      <div className="view__head">
        <div>
          <h1 className="view__title">Law dashboard</h1>
          <p className="view__subtitle">Codified rules · graduated punishment · per-(model, repo) trust score</p>
        </div>
        <div className="view__actions">
          <button className="btn"><Icon name="external" size={13}/> Lint laws</button>
          <button className="btn btn--primary"><Icon name="law" size={13}/> Author new law</button>
        </div>
      </div>

      <div className="stats-grid">
        <div className="stat">
          <div className="stat__label" style={{ color: "var(--critical)" }}><Icon name="block" size={12}/> Blocking laws</div>
          <div className="stat__value tabular">7</div>
          <div className="stat__delta">3 fired · 7d</div>
        </div>
        <div className="stat">
          <div className="stat__label"><Icon name="alert" size={12}/> Observational</div>
          <div className="stat__value tabular">21</div>
          <div className="stat__delta">87 events flagged · 7d</div>
        </div>
        <div className="stat">
          <div className="stat__label">False-block rate</div>
          <div className="stat__value tabular">0.4<span className="stat__unit">%</span></div>
          <div className="stat__delta is-up">▲ within SLO (&lt;1%)</div>
        </div>
        <div className="stat">
          <div className="stat__label">Trust score · range</div>
          <div className="stat__value tabular">0.71 – 0.96</div>
          <div className="stat__delta">claude-opus-4-7 highest</div>
        </div>
      </div>

      <div className="card" style={{ marginBottom: 18 }}>
        <div className="card__head">
          <span className="card__title">Active laws</span>
          <span className="card__sub">{MOCK.laws.length} laws · sorted by violation rate</span>
        </div>
        <div>
          <div className="law-row" style={{ background: "var(--bg-2)", color: "var(--fg-3)", fontFamily: "var(--font-mono)", fontSize: 10.5, textTransform: "uppercase", letterSpacing: "0.06em", cursor: "default" }}>
            <span>ID</span>
            <span>Title</span>
            <span>Severity</span>
            <span>Action</span>
            <span>Scope</span>
            <span style={{ textAlign: "right" }}>Rate · 7d</span>
          </div>
          {MOCK.laws.map(law => (
            <div key={law.id} className={`law-row ${activeId === law.id ? "is-active" : ""}`} onClick={() => open(law)}>
              <span className="law-row__id">{law.id}</span>
              <span className="law-row__title">{law.title}</span>
              <span className="law-row__sev"><SeverityBar severity={law.severity}/> <span style={{ marginLeft: 6, color: law.severity === "critical" ? "var(--critical)" : law.severity === "warn" ? "var(--warn)" : "var(--info)" }}>{law.severity}</span></span>
              <span>{law.blocked ? <Tag tone="critical">block</Tag> : <Tag>observe</Tag>}</span>
              <span className="mono" style={{ fontSize: 11, color: "var(--fg-2)" }}>{law.scope}</span>
              <span className="law-row__rate" style={{ textAlign: "right" }}>
                {law.violations7d} <span className="muted">/ {fmtNum(law.applies)}</span>
              </span>
            </div>
          ))}
        </div>
      </div>

      <div className="card">
        <div className="card__head">
          <span className="card__title">Trust score · per (model, repo)</span>
          <span className="card__sub">recomputed nightly · 30-day rolling</span>
        </div>
        <div className="card__body">
          <div style={{ display: "grid", gridTemplateColumns: "180px repeat(5, 1fr)", gap: 6, fontSize: 11.5, fontFamily: "var(--font-mono)" }}>
            <div></div>
            {MOCK.repos.slice(0, 5).map(r => <div key={r.id} style={{ color: "var(--fg-3)", padding: 4 }}>{r.name}</div>)}
            {MOCK.models.map((m, mi) => (
              <React.Fragment key={m}>
                <div style={{ color: "var(--fg-1)", padding: 6, fontSize: 11 }}>{m}</div>
                {MOCK.repos.slice(0, 5).map((r, ri) => {
                  const score = 0.62 + ((mi * 13 + ri * 7) % 35) / 100;
                  const hue = 25 + score * 110;
                  const bg = `oklch(0.42 0.10 ${hue} / ${0.35 + score * 0.5})`;
                  const fg = score > 0.85 ? "oklch(0.95 0.05 155)" : score > 0.75 ? "oklch(0.95 0.10 90)" : "oklch(0.95 0.10 25)";
                  return (
                    <div key={r.id} style={{ padding: "8px 10px", background: bg, borderRadius: 4, color: fg, fontVariantNumeric: "tabular-nums", textAlign: "center", border: "1px solid var(--border-soft)" }}>
                      {score.toFixed(2)}
                    </div>
                  );
                })}
              </React.Fragment>
            ))}
          </div>
        </div>
      </div>
    </div>
  );
};

window.MemoryView = MemoryView;
window.DecisionsView = DecisionsView;
window.LawsView = LawsView;
