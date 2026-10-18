import { useEffect, useRef, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Icon } from "../atoms/Icon";
import { HeaderFrame } from "../shell/Header";
import { NAV, SidebarFrame } from "../shell/Sidebar";
import { Tweaks } from "../shell/Tweaks";
import { fmtNum } from "../lib/format";
import { initialRepository, localRequest } from "./api";
import { LocalWorkspace } from "./LocalWorkspace";
import type { LocalView } from "./types";
import "./shell.css";

const supported = new Set<string>(["timeline", "memory", "decisions", "classifications", "graph", "health"]);
const navigation = [
  ...NAV.filter((item) => supported.has(item.id)),
  { id: "audit", label: "Delivery audit", icon: "timeline" as const },
  { id: "feedback", label: "Feedback", icon: "decision" as const },
];
type Repository = { repository: string; records: number; memories: number; events: number; links: number };

export function LocalApp() {
  const [repository, setRepository] = useState(initialRepository);
  const [view, setView] = useState<LocalView>("timeline");
  const [searchText, setSearchText] = useState("");
  const [draft, setDraft] = useState("");
  const [tweaksOpen, setTweaksOpen] = useState(false);
  const searchInput = useRef<HTMLInputElement>(null);
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "k") {
        event.preventDefault();
        searchInput.current?.focus();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);
  const repositories = useQuery({
    queryKey: ["local", "repositories"],
    queryFn: ({ signal }) => localRequest<{ repositories: Repository[] }>("repositories", {}, signal),
    refetchInterval: 30_000, retry: false,
  });
  const status = useQuery({
    queryKey: ["local", repository, "status", "shell"],
    queryFn: ({ signal }) => localRequest<Record<string, unknown>>("status", { repository }, signal),
    enabled: repository.length > 0, refetchInterval: 5000, retry: false,
  });
  const live = Boolean(status.data) && !status.isError;
  const selectRepository = (next: string) => {
    setRepository(next); setSearchText(""); setDraft("");
  };
  const updateSearch = (text: string) => { setDraft(text); setSearchText(text); };
  const search = (text: string) => { updateSearch(text); setView("memory"); };

  return (
    <div className="app-shell local-dashboard">
      <div className="app">
        <HeaderFrame version="local" search={
          <div className="header__search">
            <form onSubmit={(event) => { event.preventDefault(); search(draft); }}>
              <Icon name="search" size={15} />
              <input ref={searchInput} aria-label="Search current repository" placeholder="Search current repository..." value={draft} onChange={(event) => setDraft(event.target.value)} />
              <button type="submit" className="scope-chip" disabled={!repository}>Search</button>
            </form>
          </div>
        }>
          <span className="mono local-runtime">Local files</span>
          <button type="button" className={`status-pill ${live ? "" : "is-paused"}`} onClick={() => setView("health")} title="Local store status; polled every 5 seconds">
            <span className="dot" /><span className="mono">{!repository ? "select repo" : status.isPending ? "connecting..." : live ? "local" : "offline"}</span>
          </button>
          <span className="status-pill local-polling" title="Polling every 5 seconds; no push stream"><span className="mono">polling</span></span>
          <button className="icon-btn" onClick={() => setTweaksOpen(true)} title="Open tweaks (theme · accent · density)" aria-label="Open tweaks panel"><Icon name="settings" size={15} /></button>
        </HeaderFrame>
        <SidebarFrame>
          <div className="sidebar__group-label">Workspace</div>
          {navigation.map((item) => <button type="button" key={item.id} aria-label={item.label} title={item.label} className={`nav-item ${view === item.id ? "is-active" : ""}`} onClick={() => setView(item.id as LocalView)}><span className="nav-icon"><Icon name={item.icon} size={15} /></span><span className="nav-label">{item.label}</span></button>)}
          <div className="local-desktop-repositories">
          <div className="sidebar__group-label" style={{ marginTop: 14 }}>Repos · {repositories.data?.repositories.length ?? "..."}</div>
          <input className="local-repository" aria-label="Exact repository" placeholder="Exact repository" value={repository} onChange={(event) => selectRepository(event.target.value)} />
          {repositories.isError && <p className="muted local-shell-message" role="alert">{repositories.error.message}</p>}
          {repositories.data?.repositories.map((entry) => <button type="button" className={`nav-item ${entry.repository === repository ? "is-active" : ""}`} key={entry.repository} onClick={() => selectRepository(entry.repository)} title={`${entry.repository} · ${entry.records} records`}><span className="nav-icon"><Icon name="memory" size={15} /></span><span className="nav-label mono">{entry.repository}</span><span className="nav-count">{fmtNum(entry.records)}</span></button>)}
          </div>
          <div className="sidebar__footer"><div className="repo-pill"><span className="repo-dot" style={{ background: live ? "var(--ok)" : "var(--fg-3)" }} /><span className="repo-name">cortex-local</span><span className="repo-meta">{live ? "connected" : "not connected"}</span></div></div>
        </SidebarFrame>
        <main className="main">
          <label className="local-mobile-repository">Repository
            <select aria-label="Select repository" value={repository} onChange={event => selectRepository(event.target.value)}>
              {!repository && <option value="">Choose repository</option>}
              {repository && !repositories.data?.repositories.some(entry => entry.repository === repository) && <option value={repository}>{repository}</option>}
              {repositories.data?.repositories.map(entry => <option key={entry.repository} value={entry.repository}>{entry.repository}</option>)}
            </select>
          </label>
          {repository ? <LocalWorkspace key={repository} view={view} repository={repository} searchText={searchText} onSearch={updateSearch} onNavigate={setView} /> : <div className="panel"><div className="panel__header"><h2>Select a repository</h2></div><p className="muted local-shell-message">Choose an exact repository from the sidebar to inspect local history.</p></div>}
        </main>
        <Tweaks local open={tweaksOpen} onClose={() => setTweaksOpen(false)} />
      </div>
    </div>
  );
}
