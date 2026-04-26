import { Icon, type IconName } from "../atoms/Icon";

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
      <div className="sidebar__footer">
        <div className="repo-pill">
          <span className="repo-dot" />
          <span className="repo-name">cortex-api</span>
          <span className="repo-meta">live</span>
        </div>
      </div>
    </aside>
  );
}
