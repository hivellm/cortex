/* Cortex — Timeline view (hero) */

const KIND_ICON = {
  turn: "→",
  tool_call: "⚙",
  agent_call: "↳",
  memory: "◆",
  decision: "✓",
  law_violation: "!"
};

const KIND_LABEL = {
  turn: "Turn",
  tool_call: "Tool",
  agent_call: "Agent",
  memory: "Memory",
  decision: "Decision",
  law_violation: "Violation"
};

const TimelineRow = ({ ev, isNew, onSelect }) => (
  <button className={`timeline__row ${isNew ? "is-new" : ""}`} onClick={() => onSelect(ev)} style={{ width: "100%", textAlign: "left", border: 0 }}>
    <span className="timeline__time">{ev.t}</span>
    <span className="timeline__type" data-kind={ev.kind} title={KIND_LABEL[ev.kind]}>
      <span style={{ fontFamily: "var(--font-mono)", fontSize: 11 }}>{KIND_ICON[ev.kind]}</span>
    </span>
    <span className="timeline__main">
      <span className="timeline__title">
        <span>{ev.title}</span>
        {ev.kind === "tool_call" && <span className="mono">· {ev.detail.split(" ")[0]}</span>}
      </span>
      <span className="timeline__detail">
        <span className="muted">{ev.kind === "tool_call" ? ev.detail.split(" ").slice(1).join(" ") || ev.detail : ev.detail}</span>
      </span>
    </span>
    <span className="timeline__meta">
      <span>{ev.repo} · <span className="muted">{ev.session}</span></span>
      <span>{ev.model} · <span className="muted">{ev.duration}</span></span>
    </span>
  </button>
);

const TimelineFilters = ({ filters, setFilters, query, setQuery, live, setLive }) => {
  const kinds = ["turn", "tool_call", "agent_call", "memory", "decision", "law_violation"];
  const repos = ["Vectorizer", "Nexus", "Synap", "Rulebook", "Cortex"];
  const toggleKind = (k) => {
    const next = filters.kinds.includes(k) ? filters.kinds.filter(x => x !== k) : [...filters.kinds, k];
    setFilters({ ...filters, kinds: next });
  };
  const toggleRepo = (r) => {
    const next = filters.repos.includes(r) ? filters.repos.filter(x => x !== r) : [...filters.repos, r];
    setFilters({ ...filters, repos: next });
  };
  return (
    <div className="filter-bar">
      <span className="filter-bar__label">Kind</span>
      <span className="chip-group">
        {kinds.map(k => (
          <button key={k} className={`chip ${filters.kinds.includes(k) ? "is-active" : ""}`} onClick={() => toggleKind(k)}>
            <span className="chip-dot"/>{KIND_LABEL[k]}
          </button>
        ))}
      </span>
      <span className="filter-divider"/>
      <span className="filter-bar__label">Repo</span>
      <span className="chip-group">
        {repos.map(r => (
          <button key={r} className={`chip ${filters.repos.includes(r) ? "is-active" : ""}`} onClick={() => toggleRepo(r)}>{r}</button>
        ))}
      </span>
      <span className="filter-divider"/>
      <div style={{ position: "relative", flex: 1, minWidth: 200 }}>
        <Icon name="search" size={13}/>
        <input
          type="text"
          placeholder="Search events, sessions, models…"
          value={query}
          onChange={e => setQuery(e.target.value)}
          style={{
            width: "100%",
            height: 26,
            padding: "0 10px 0 28px",
            background: "var(--bg-2)",
            border: "1px solid var(--border)",
            borderRadius: "var(--radius-sm)",
            color: "var(--fg-0)",
            fontSize: 11.5,
            outline: "none"
          }}
        />
        <span style={{ position: "absolute", left: 8, top: "50%", transform: "translateY(-50%)", color: "var(--fg-3)", pointerEvents: "none" }}>
          <Icon name="search" size={13}/>
        </span>
      </div>
      <button className={`btn btn--sm ${live ? "" : "btn--ghost"}`} onClick={() => setLive(!live)}>
        <Icon name={live ? "pause" : "play"} size={12}/> {live ? "Pause stream" : "Resume"}
      </button>
    </div>
  );
};

const TimelineView = ({ onSelect }) => {
  const [filters, setFilters] = React.useState({ kinds: [], repos: [] });
  const [query, setQuery] = React.useState("");
  const [live, setLive] = React.useState(true);
  const [events, setEvents] = React.useState(MOCK.events);
  const [newIds, setNewIds] = React.useState(new Set());

  // Simulated SSE: every ~5s prepend a synthetic event
  React.useEffect(() => {
    if (!live) return;
    const tick = () => {
      const samples = [
        { kind: "tool_call", title: "Read", detail: "src/lib/embedder/chunker.rs · 4.2 KB", duration: "6 ms" },
        { kind: "tool_call", title: "Grep", detail: "pattern: 'tokio::spawn' · 22 matches", duration: "29 ms" },
        { kind: "turn", title: "Pre-thinking bundle injected", detail: "2 decisions · 1 analysis · 5 similar turns", duration: "54 ms" },
        { kind: "tool_call", title: "Edit", detail: "cortex-api/src/query.rs · +9 / −3", duration: "76 ms" },
        { kind: "memory", title: "Reference memory captured", detail: "Bootstrap of 17 repos completed in 6h 12m", duration: "—" }
      ];
      const s = samples[Math.floor(Math.random() * samples.length)];
      const id = "01HXY8" + Math.random().toString(36).slice(2, 6).toUpperCase();
      const now = new Date().toTimeString().slice(0, 8);
      const ev = { ...s, id, t: now, session: "ses_8a3f", model: "claude-opus-4-7", repo: "Cortex" };
      setEvents(prev => [ev, ...prev].slice(0, 80));
      setNewIds(new Set([id]));
      setTimeout(() => setNewIds(new Set()), 700);
    };
    const i = setInterval(tick, 5200);
    return () => clearInterval(i);
  }, [live]);

  const filtered = events.filter(e => {
    if (filters.kinds.length && !filters.kinds.includes(e.kind)) return false;
    if (filters.repos.length && !filters.repos.includes(e.repo)) return false;
    if (query) {
      const q = query.toLowerCase();
      if (!(e.title + " " + e.detail + " " + e.session + " " + e.model + " " + e.repo).toLowerCase().includes(q)) return false;
    }
    return true;
  });

  // Stats
  const counts = filtered.reduce((acc, e) => ((acc[e.kind] = (acc[e.kind] || 0) + 1), acc), {});
  const sparks = {
    turns: [12, 18, 14, 22, 19, 28, 24, 31, 28, 34, 30, 38, 42, 39, 44, 41, 48, 46, 52, 50],
    tools: [40, 55, 48, 62, 70, 65, 78, 85, 80, 92, 88, 95, 102, 98, 110, 108, 118, 122, 130, 128],
    violations: [2, 1, 3, 1, 0, 2, 1, 4, 2, 1, 3, 2, 1, 0, 2, 3, 1, 2, 1, 1],
    classifier: [1.8, 2.1, 1.9, 2.4, 2.0, 2.6, 2.2, 2.3, 2.1, 2.5, 2.4, 2.7, 2.3, 2.2, 2.4, 2.1, 2.0, 2.3, 2.2, 2.4]
  };

  return (
    <div className="view">
      <div className="view__head">
        <div>
          <h1 className="view__title">Live timeline</h1>
          <p className="view__subtitle">Sessions · turns · tool calls · decisions — streamed from <span className="mono">cortex.events.enriched</span></p>
        </div>
        <div className="view__actions">
          <button className="btn btn--ghost"><Icon name="external" size={13}/> Export NDJSON</button>
          <button className="btn"><Icon name="filter" size={13}/> Saved views</button>
        </div>
      </div>

      <div className="stats-grid">
        <div className="stat">
          <div className="stat__label">Events / minute</div>
          <div className="stat__value tabular">312<span className="stat__unit">eps</span></div>
          <div className="stat__delta is-up">▲ 18% vs 1h avg</div>
          <div className="stat__spark"><Sparkline data={sparks.tools} color="var(--accent)"/></div>
        </div>
        <div className="stat">
          <div className="stat__label">Pre-thinking P95</div>
          <div className="stat__value tabular">142<span className="stat__unit">ms</span></div>
          <div className="stat__delta is-up">▲ within target (&lt;150 ms)</div>
          <div className="stat__spark"><Sparkline data={sparks.turns} color="var(--info)"/></div>
        </div>
        <div className="stat">
          <div className="stat__label">Active violations · 7d</div>
          <div className="stat__value tabular">23<span className="stat__unit">obs</span></div>
          <div className="stat__delta is-down">▼ 4 critical, 1 blocked just now</div>
          <div className="stat__spark"><Sparkline data={sparks.violations} color="var(--critical)"/></div>
        </div>
        <div className="stat">
          <div className="stat__label">Classifier spend · today</div>
          <div className="stat__value tabular">$2.41<span className="stat__unit">/ $40 cap</span></div>
          <div className="stat__delta">cache-hit 64%</div>
          <div className="stat__spark"><Sparkline data={sparks.classifier} color="var(--ok)"/></div>
        </div>
      </div>

      <TimelineFilters filters={filters} setFilters={setFilters} query={query} setQuery={setQuery} live={live} setLive={setLive}/>

      <div className="timeline">
        {filtered.length === 0 ? (
          <div style={{ padding: 40, textAlign: "center", color: "var(--fg-3)" }}>
            No events match these filters.
          </div>
        ) : filtered.map(ev => (
          <TimelineRow key={ev.id} ev={ev} isNew={newIds.has(ev.id)} onSelect={onSelect}/>
        ))}
      </div>

      <div style={{ marginTop: 12, display: "flex", justifyContent: "space-between", fontSize: 11.5, color: "var(--fg-3)", fontFamily: "var(--font-mono)" }}>
        <span>{filtered.length} events shown · {events.length} total in buffer</span>
        <span>SSE channel: <span style={{ color: live ? "var(--ok)" : "var(--fg-3)" }}>{live ? "● connected" : "○ paused"}</span></span>
      </div>
    </div>
  );
};

window.TimelineView = TimelineView;
window.KIND_LABEL = KIND_LABEL;
