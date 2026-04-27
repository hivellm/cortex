import { useQuery } from "@tanstack/react-query";

import { Icon, type IconName } from "../atoms/Icon";
import { api, type SessionRow } from "../lib/api";
import { useFilters } from "../lib/filters";

export type ViewId = "timeline" | "memory" | "decisions" | "laws" | "analysis" | "tools" | "graph";

type NavItem = { id: ViewId; label: string; icon: IconName };

const NAV: NavItem[] = [
  { id: "timeline", label: "Live timeline", icon: "timeline" },
  { id: "memory", label: "Memory", icon: "memory" },
  { id: "decisions", label: "Decisions", icon: "decision" },
  { id: "laws", label: "Laws", icon: "law" },
  { id: "analysis", label: "Analysis", icon: "analysis" },
  { id: "tools", label: "Tool analytics", icon: "tools" },
  { id: "graph", label: "Graph explorer", icon: "graph" },
];

type SidebarProps = {
  view: ViewId;
  setView: (v: ViewId) => void;
  collapsed: boolean;
};

export function Sidebar({ view, setView, collapsed }: SidebarProps) {
  const { filters, setFilter, clearFilters } = useFilters();

  const sessionsQ = useQuery({
    queryKey: ["sessions"],
    queryFn: () => api.sessions(),
    refetchInterval: 8000,
    refetchIntervalInBackground: true,
  });
  const sessions = sessionsQ.data ?? [];

  const onSessionClick = (sid: string) => {
    if (filters.session_id === sid) {
      setFilter("session_id", undefined);
    } else {
      setFilter("session_id", sid);
      setView("timeline");
    }
  };

  return (
    <aside className="sidebar">
      <div className="sidebar__group-label">Workspace</div>
      {NAV.map((item) => (
        <button
          key={item.id}
          className={`nav-item ${view === item.id ? "is-active" : ""}`}
          onClick={() => setView(item.id)}
          title={collapsed ? item.label : undefined}
        >
          <span className="nav-icon">
            <Icon name={item.icon} size={15} />
          </span>
          <span className="nav-label">{item.label}</span>
        </button>
      ))}

      <div className="sidebar__group-label" style={{ marginTop: 14, display: "flex", alignItems: "center", justifyContent: "space-between" }}>
        <span>Sessions · {sessions.length}</span>
        {filters.session_id ? (
          <button
            className="sidebar__clear"
            onClick={() => clearFilters()}
            title="Clear filters"
            style={{
              background: "transparent",
              border: 0,
              color: "var(--accent)",
              fontSize: 10,
              fontFamily: "var(--font-mono)",
              cursor: "pointer",
            }}
          >
            clear
          </button>
        ) : null}
      </div>

      {sessionsQ.isLoading ? (
        <div className="muted" style={{ padding: "4px 12px", fontSize: 11 }}>
          loading…
        </div>
      ) : sessions.length === 0 ? (
        <div className="muted" style={{ padding: "4px 12px", fontSize: 11 }}>
          no sessions captured yet
        </div>
      ) : (
        <div className="session-list">
          {sessions.map((s) => (
            <SessionItem
              key={s.session_id}
              session={s}
              active={filters.session_id === s.session_id}
              onClick={() => onSessionClick(s.session_id)}
            />
          ))}
        </div>
      )}

      <SidebarFooter />
    </aside>
  );
}

function SidebarFooter() {
  // Mirror the header's status query so the footer pill agrees with
  // the header's connection indicator without duplicating fetches —
  // TanStack Query dedupes by key.
  const statusQ = useQuery({
    queryKey: ["status"],
    queryFn: () => api.status(),
    refetchInterval: 5000,
    refetchIntervalInBackground: true,
    retry: 0,
  });
  const live = !statusQ.isError && !!statusQ.data;
  return (
    <div className="sidebar__footer">
      <div className="repo-pill">
        <span
          className="repo-dot"
          style={{ background: live ? "var(--ok)" : "var(--fg-3)" }}
        />
        <span className="repo-name">cortex-api</span>
        <span className="repo-meta">
          {statusQ.isLoading ? "…" : live ? `v${statusQ.data!.version}` : "offline"}
        </span>
      </div>
    </div>
  );
}

function SessionItem({
  session,
  active,
  onClick,
}: {
  session: SessionRow;
  active: boolean;
  onClick: () => void;
}) {
  const short = session.session_id.slice(0, 8);
  const title = session.title || "(no turn captured)";
  const repo = session.repos[0] ?? "—";
  const turnCount =
    session.kind_breakdown.find((k) => k.kind === "turn")?.count ?? 0;
  const toolCount =
    session.kind_breakdown.find((k) => k.kind === "tool_call")?.count ?? 0;
  return (
    <button
      type="button"
      className={`session-item ${active ? "is-active" : ""}`}
      onClick={onClick}
      title={`${session.session_id}\n${session.event_count} events`}
    >
      <span className="session-item__title">{title}</span>
      <span className="session-item__meta">
        <span className="mono">{short}</span>
        <span>·</span>
        <span>{repo}</span>
        <span>·</span>
        <span>
          {turnCount}t / {toolCount}tc
        </span>
      </span>
    </button>
  );
}
