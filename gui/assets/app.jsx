/* Cortex — App shell + Inspector + main entry */

const NAV = [
  { id: "timeline", label: "Live timeline", icon: "timeline", count: "live" },
  { id: "memory", label: "Memory", icon: "memory", count: "8.4k" },
  { id: "decisions", label: "Decisions", icon: "decision", count: "87" },
  { id: "laws", label: "Laws", icon: "law", count: "28" },
  { id: "analysis", label: "Analysis", icon: "analysis", count: "14" },
  { id: "tools", label: "Tool analytics", icon: "tools", count: null },
  { id: "graph", label: "Graph explorer", icon: "graph", count: null }
];

const Sidebar = ({ view, setView, collapsed }) => (
  <aside className="sidebar">
    <div className="sidebar__group-label">Workspace</div>
    {NAV.map(item => (
      <button
        key={item.id}
        className={`nav-item ${view === item.id ? "is-active" : ""}`}
        onClick={() => setView(item.id)}
        title={collapsed ? item.label : undefined}
      >
        <span className="nav-icon"><Icon name={item.icon} size={15}/></span>
        <span className="nav-label">{item.label}</span>
        {item.count && <span className="nav-count">{item.count}</span>}
      </button>
    ))}

    <div className="sidebar__group-label">Repos · 17 indexed</div>
    {MOCK.repos.slice(0, 6).map(r => (
      <div key={r.id} className="nav-item" style={{ cursor: "default" }}>
        <span className="nav-icon" style={{ width: 8, height: 8, borderRadius: 2, background: "var(--accent-dim)", display: "inline-block" }}/>
        <span className="nav-label mono" style={{ fontSize: 11.5 }}>{r.name}</span>
        <span className="nav-count">{fmtNum(r.events)}</span>
      </div>
    ))}

    <div className="sidebar__footer">
      <div className="repo-pill">
        <span className="repo-dot"/>
        <span className="repo-name">cortex-core</span>
        <span className="repo-meta">v0.1.4</span>
      </div>
    </div>
  </aside>
);

const Header = ({ collapsed, setCollapsed, live, setLive, theme, setTheme, onTweaks }) => (
  <header className="header">
    <div className="header__brand">
      <button className="icon-btn" onClick={() => setCollapsed(!collapsed)} title="Toggle sidebar">
        <Icon name="menu" size={15}/>
      </button>
      <span className="brand-mark"/>
      <span className="header__brand-text" style={{ display: "flex", alignItems: "baseline", gap: 8 }}>
        <span className="brand-name">Cortex</span>
        <span className="brand-version mono">v0.1</span>
      </span>
    </div>

    <div className="header__search">
      <span className="icon"><Icon name="search" size={14}/></span>
      <input placeholder="Search events, decisions, laws, memories…"/>
      <kbd>⌘K</kbd>
    </div>

    <div className="header__right">
      <span className={`status-pill ${live ? "" : "is-paused"}`}>
        <span className="dot"/>
        <span className="mono">{live ? "ingesting · 312 eps" : "paused"}</span>
      </span>
      <button className="icon-btn" onClick={() => setTheme(theme === "dark" ? "light" : "dark")} title="Toggle theme">
        <Icon name={theme === "dark" ? "sun" : "moon"} size={15}/>
      </button>
      <button className="icon-btn" title="Notifications">
        <Icon name="bell" size={15}/>
      </button>
      <button className="icon-btn" title="Docs">
        <Icon name="book" size={15}/>
      </button>
      <button className="icon-btn" onClick={onTweaks} title="Tweaks">
        <Icon name="settings" size={15}/>
      </button>
      <span className="avatar">A</span>
    </div>
  </header>
);

/* Inspector — opens for events and law violations */
const Inspector = ({ payload, onClose }) => {
  const isOpen = !!payload;
  return (
    <>
      <div className={`inspector-backdrop ${isOpen ? "is-open" : ""}`} onClick={onClose}/>
      <aside className={`inspector ${isOpen ? "is-open" : ""}`}>
        {payload && (payload.kind === "law" ? <LawInspector law={payload.law} onClose={onClose}/> : <EventInspector ev={payload} onClose={onClose}/>)}
      </aside>
    </>
  );
};

const EventInspector = ({ ev, onClose }) => (
  <>
    <div className="inspector__head">
      <span className="timeline__type" data-kind={ev.kind} style={{ width: 26, height: 26 }}>
        <span style={{ fontFamily: "var(--font-mono)", fontSize: 12 }}>{KIND_ICON_MAP[ev.kind]}</span>
      </span>
      <div style={{ display: "flex", flexDirection: "column", gap: 2, minWidth: 0 }}>
        <span className="inspector__title">{ev.title}</span>
        <span className="inspector__id">{ev.id} · {KIND_LABEL[ev.kind]}</span>
      </div>
      <button className="icon-btn" onClick={onClose} style={{ marginLeft: "auto" }}><Icon name="close" size={15}/></button>
    </div>
    <div className="inspector__body">
      <div className="inspector__section">
        <div className="inspector__section-label">Detail</div>
        <div style={{ fontSize: 12.5, color: "var(--fg-1)", textWrap: "pretty" }}>{ev.detail}</div>
      </div>
      <div className="inspector__section">
        <div className="inspector__section-label">Envelope</div>
        <dl className="kv-list">
          <dt>session</dt><dd className="mono">{ev.session}</dd>
          <dt>tool</dt><dd className="mono">claude-code</dd>
          <dt>model</dt><dd className="mono">{ev.model}</dd>
          <dt>repo</dt><dd className="mono">{ev.repo}</dd>
          <dt>occurred</dt><dd className="mono">{ev.t}</dd>
          <dt>duration</dt><dd className="mono">{ev.duration}</dd>
        </dl>
      </div>
      <div className="inspector__section">
        <div className="inspector__section-label">Payload (redacted)</div>
        <pre className="code-block">{`{
  "event_id": "${ev.id}",
  "kind": "${ev.kind}",
  "session_id": "${ev.session}",
  "tool": "claude-code",
  "model": "${ev.model}",
  "payload": {
    "title": "${ev.title}",
    "detail": "${ev.detail.slice(0, 60)}..."
  },
  "context": {
    "repo": "e:/HiveLLM/${ev.repo}",
    "branch": "main",
    "commit": "abc123"
  },
  "redactions": ["secret:.env"],
  "schema_version": "1"
}`}</pre>
      </div>
      <div className="inspector__section">
        <div className="inspector__section-label">Linked</div>
        <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
          <a className="violation-card" style={{ display: "block", textDecoration: "none", marginBottom: 0 }}>
            <div className="violation-card__head"><Tag tone="ok">DEC-2026-014</Tag><span style={{ fontSize: 11.5, color: "var(--fg-1)" }}>Symbol-level chunking via Tree-sitter</span></div>
            <div style={{ fontSize: 11, color: "var(--fg-3)" }}>Cited in pre-thinking bundle of this session</div>
          </a>
          <a className="violation-card" style={{ display: "block", textDecoration: "none", marginBottom: 0 }}>
            <div className="violation-card__head"><Tag>ANL-031</Tag><span style={{ fontSize: 11.5, color: "var(--fg-1)" }}>HNSW recall &gt; 1M vectors</span></div>
            <div style={{ fontSize: 11, color: "var(--fg-3)" }}>Source analysis</div>
          </a>
        </div>
      </div>
    </div>
  </>
);

const KIND_ICON_MAP = {
  turn: "→", tool_call: "⚙", agent_call: "↳", memory: "◆", decision: "✓", law_violation: "!"
};

const LawInspector = ({ law, onClose }) => {
  const violations = MOCK.violations.filter(v => v.lawId === law.id);
  return (
    <>
      <div className="inspector__head">
        <span style={{ width: 26, height: 26, display: "grid", placeItems: "center", borderRadius: 4, background: law.severity === "critical" ? "var(--critical-soft)" : "var(--warn-soft)", color: law.severity === "critical" ? "var(--critical)" : "var(--warn)", border: `1px solid oklch(from ${law.severity === "critical" ? "var(--critical)" : "var(--warn)"} l c h / 0.4)` }}>
          <Icon name={law.blocked ? "block" : "alert"} size={14}/>
        </span>
        <div style={{ display: "flex", flexDirection: "column", gap: 2 }}>
          <span className="inspector__title">{law.id}</span>
          <span className="inspector__id">{law.severity} · {law.blocked ? "blocking" : "observational"}</span>
        </div>
        <button className="icon-btn" onClick={onClose} style={{ marginLeft: "auto" }}><Icon name="close" size={15}/></button>
      </div>
      <div className="inspector__body">
        <div className="inspector__section">
          <div style={{ fontSize: 14, color: "var(--fg-0)", fontWeight: 600, marginBottom: 6, letterSpacing: "-0.01em" }}>{law.title}</div>
          <div style={{ display: "flex", gap: 6, flexWrap: "wrap" }}>
            <Tag tone={sevTone(law.severity)}>{law.severity}</Tag>
            {law.blocked ? <Tag tone="critical">tier 3 · block</Tag> : <Tag>tier 1 · annotate</Tag>}
            <Tag>{law.scope}</Tag>
          </div>
        </div>
        <div className="inspector__section">
          <div className="inspector__section-label">Definition (laws/{law.id}.md)</div>
          <pre className="code-block">{`---
id: ${law.id}
title: ${law.title}
severity: ${law.severity}
applies_to: [${law.scope.split(", ").map(s => `"${s}"`).join(", ")}]
detector: ${law.detector}
remediation: |
  ${law.remediation}
introduced: 2026-04-17
---
The model MUST follow this rule unless the user has
explicitly authorized an exception in this session.`}</pre>
        </div>
        <div className="inspector__section">
          <div className="inspector__section-label">7-day stats</div>
          <dl className="kv-list">
            <dt>applies</dt><dd className="mono tabular">{fmtNum(law.applies)} eligible events</dd>
            <dt>violations</dt><dd className="mono tabular">{law.violations7d}</dd>
            <dt>rate</dt><dd className="mono tabular">{law.rate.toFixed(2)} per 1k</dd>
            <dt>action</dt><dd className="mono">{law.blocked ? "PreToolUse block" : "PostToolUse annotate"}</dd>
          </dl>
        </div>
        {violations.length > 0 && (
          <div className="inspector__section">
            <div className="inspector__section-label">Recent violations</div>
            {violations.map(v => (
              <div key={v.id} className="violation-card">
                <div className="violation-card__head">
                  <span className="mono" style={{ fontSize: 11, color: "var(--accent)", fontWeight: 600 }}>{v.id}</span>
                  <Tag tone={v.action === "blocked" ? "critical" : "warn"}>{v.action}</Tag>
                  <span className="mono" style={{ fontSize: 10.5, color: "var(--fg-3)", marginLeft: "auto" }}>{v.at}</span>
                </div>
                <div style={{ fontSize: 11.5, color: "var(--fg-2)", marginBottom: 4 }}>
                  <span className="mono">{v.model}</span> · <span>{v.repo}</span> · <span className="mono">{v.session}</span>
                </div>
                <pre className="code-block" style={{ fontSize: 11, padding: "6px 8px", marginTop: 6 }}>{v.evidence}</pre>
                <div style={{ fontSize: 11, color: "var(--fg-3)", marginTop: 6 }}>{v.remediation}</div>
              </div>
            ))}
          </div>
        )}
      </div>
    </>
  );
};

/* ---------------------- Tweaks ---------------------- */
const TWEAK_DEFAULTS = /*EDITMODE-BEGIN*/{
  "theme": "dark",
  "accentHue": 75,
  "density": 7,
  "sidebarCollapsed": false,
  "liveSSE": true
}/*EDITMODE-END*/;

const TweaksFor = ({ tweaks, setTweak }) => (
  <>
    <TweakSection title="Theme">
      <TweakRadio label="Mode" value={tweaks.theme} options={[{ value: "dark", label: "Dark" }, { value: "light", label: "Light" }]} onChange={v => setTweak("theme", v)}/>
    </TweakSection>
    <TweakSection title="Color">
      <TweakSlider label="Accent hue" value={tweaks.accentHue} min={20} max={320} step={1} onChange={v => setTweak("accentHue", v)} unit="°"/>
      <div style={{ display: "flex", gap: 6, marginTop: 4 }}>
        {[
          { name: "Amber", h: 75 },
          { name: "Green", h: 155 },
          { name: "Blue", h: 230 },
          { name: "Purple", h: 290 },
          { name: "Red", h: 25 }
        ].map(p => (
          <button key={p.h} onClick={() => setTweak("accentHue", p.h)} title={p.name} style={{
            width: 24, height: 24, borderRadius: 4,
            background: `oklch(0.78 0.135 ${p.h})`,
            border: tweaks.accentHue === p.h ? "2px solid var(--fg-0)" : "1px solid var(--border)",
            cursor: "pointer"
          }}/>
        ))}
      </div>
    </TweakSection>
    <TweakSection title="Layout">
      <TweakToggle label="Sidebar collapsed" value={tweaks.sidebarCollapsed} onChange={v => setTweak("sidebarCollapsed", v)}/>
      <TweakSlider label="Data density" value={tweaks.density} min={1} max={10} step={1} onChange={v => setTweak("density", v)}/>
    </TweakSection>
    <TweakSection title="Stream">
      <TweakToggle label="Live SSE simulation" value={tweaks.liveSSE} onChange={v => setTweak("liveSSE", v)}/>
    </TweakSection>
  </>
);

/* ---------------------- App ---------------------- */
function App() {
  const [tweaks, setTweak] = useTweaks(TWEAK_DEFAULTS);
  const [view, setView] = React.useState("timeline");
  const [inspector, setInspector] = React.useState(null);
  const [tweaksOpen, setTweaksOpen] = React.useState(false);

  React.useEffect(() => {
    document.documentElement.dataset.theme = tweaks.theme;
    document.documentElement.style.setProperty("--accent-h", tweaks.accentHue);
    // Density: maps 1..10 to padding/row size adjustments via CSS variable
    const compact = (10 - tweaks.density) / 10;
    document.documentElement.style.setProperty("--header-h", `${52 - compact * 8}px`);
  }, [tweaks.theme, tweaks.accentHue, tweaks.density]);

  const collapsed = tweaks.sidebarCollapsed;

  const renderView = () => {
    switch (view) {
      case "timeline": return <TimelineView onSelect={setInspector}/>;
      case "memory": return <MemoryView/>;
      case "decisions": return <DecisionsView/>;
      case "laws": return <LawsView onSelect={setInspector}/>;
      case "analysis": return <AnalysisView/>;
      case "tools": return <ToolsView/>;
      case "graph": return <GraphView/>;
      default: return null;
    }
  };

  return (
    <div className={`app ${collapsed ? "collapsed" : ""}`}>
      <Header
        collapsed={collapsed}
        setCollapsed={(v) => setTweak("sidebarCollapsed", v)}
        live={tweaks.liveSSE}
        setLive={(v) => setTweak("liveSSE", v)}
        theme={tweaks.theme}
        setTheme={(v) => setTweak("theme", v)}
        onTweaks={() => setTweaksOpen(true)}
      />
      <Sidebar view={view} setView={setView} collapsed={collapsed}/>
      <main className="main">
        {renderView()}
        <Inspector payload={inspector} onClose={() => setInspector(null)}/>
      </main>
      {tweaksOpen && (
        <TweaksPanel title="Tweaks" onClose={() => setTweaksOpen(false)}>
          <TweaksFor tweaks={tweaks} setTweak={setTweak}/>
        </TweaksPanel>
      )}
    </div>
  );
}

const root = ReactDOM.createRoot(document.getElementById("app"));
root.render(<App/>);
