import { useQuery } from "@tanstack/react-query";

import { Icon, type IconName } from "../atoms/Icon";
import { Sparkline } from "../atoms/Sparkline";
import { api, type RepoCount, type SessionRow } from "../lib/api";
import { fmtNum } from "../lib/format";
import { useFilters } from "../lib/filters";

export type ViewId =
  | "timeline"
  | "conversations"
  | "memory"
  | "retention"
  | "decisions"
  | "handoffs"
  | "classifications"
  | "laws"
  | "analysis"
  | "tools"
  | "graph"
  | "health";

type NavItem = { id: ViewId; label: string; icon: IconName; countSource?: CountKey };
type CountKey =
  | "events"
  | "decisions"
  | "laws"
  | "analyses"
  | "tools"
  | "sessions"
  | "conversations"
  | "handoffs"
  | "classifications";

const NAV: NavItem[] = [
  { id: "timeline", label: "Live timeline", icon: "timeline" },
  { id: "conversations", label: "Conversations", icon: "memory", countSource: "conversations" },
  { id: "memory", label: "Memory", icon: "memory", countSource: "events" },
  { id: "retention", label: "Retention", icon: "archive" },
  { id: "decisions", label: "Decisions", icon: "decision", countSource: "decisions" },
  { id: "handoffs", label: "Handoffs", icon: "memory", countSource: "handoffs" },
  { id: "classifications", label: "Classifications", icon: "analysis", countSource: "classifications" },
  { id: "laws", label: "Laws", icon: "law", countSource: "laws" },
  { id: "analysis", label: "Analysis", icon: "analysis", countSource: "analyses" },
  { id: "tools", label: "Tool analytics", icon: "tools", countSource: "tools" },
  { id: "graph", label: "Graph explorer", icon: "graph" },
  { id: "health", label: "Health", icon: "tools" },
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

  const overviewQ = useQuery({
    queryKey: ["overview"],
    queryFn: () => api.overview(),
    refetchInterval: 10_000,
    refetchIntervalInBackground: true,
  });
  const overview = overviewQ.data;

  // Counts pulled from the same TanStack caches the views populate —
  // re-using the keyed query result avoids a second round of fetches.
  const decisionsQ = useQuery({
    queryKey: ["decisions"],
    queryFn: () => api.decisions(),
    refetchInterval: 30_000,
  });
  const lawsQ = useQuery({
    queryKey: ["laws"],
    queryFn: () => api.laws(),
    refetchInterval: 60_000,
  });
  const analysesQ = useQuery({
    queryKey: ["analyses"],
    queryFn: () => api.analyses(),
    refetchInterval: 30_000,
  });
  const toolsQ = useQuery({
    queryKey: ["tools-stats"],
    queryFn: () => api.toolsStats(),
    refetchInterval: 15_000,
  });
  const conversationsQ = useQuery({
    queryKey: ["conversations"],
    queryFn: () => api.conversations(),
    refetchInterval: 15_000,
  });
  const handoffsQ = useQuery({
    queryKey: ["handoffs", "all"],
    queryFn: () => api.handoffs(),
    refetchInterval: 30_000,
  });
  const classificationsQ = useQuery({
    queryKey: ["classifications", "sidebar"],
    queryFn: () => api.classifications({ limit: 1 }),
    refetchInterval: 30_000,
  });

  const counts: Record<CountKey, number | undefined> = {
    events: overview?.events_total,
    decisions: decisionsQ.data?.length,
    laws: lawsQ.data?.length,
    analyses: analysesQ.data?.length,
    tools: toolsQ.data?.tools.length,
    sessions: sessions.length,
    conversations: conversationsQ.data?.length,
    handoffs: handoffsQ.data?.length,
    classifications: classificationsQ.data?.stats.total,
  };

  const onSessionClick = (sid: string) => {
    if (filters.session_id === sid) {
      setFilter("session_id", undefined);
    } else {
      setFilter("session_id", sid);
      setView("timeline");
    }
  };

  /// Toggle this repo in the active filter list. Clicking a repo
  /// row that's already selected removes it; clicking a new one
  /// adds it; clearing the last selection drops the filter
  /// entirely so the timeline shows everything again.
  const onRepoClick = (repo: string) => {
    const current = filters.repo ?? [];
    const next = current.includes(repo)
      ? current.filter((r) => r !== repo)
      : [...current, repo];
    setFilter("repo", next.length === 0 ? undefined : next);
  };

  // 20-bucket events-per-min sparkline drawn under the Workspace
  // label — gives the sidebar a live pulse cue without claiming a
  // full stat tile (those land on the Timeline view's stats grid).
  // Honest empty when the overview hasn't returned yet.
  const epm = overview?.series.events_per_min ?? [];
  const sparkVisible = !collapsed && epm.some((v) => v > 0);

  return (
    <aside className="sidebar">
      <div className="sidebar__group-label">Workspace</div>
      {sparkVisible ? (
        <div
          className="sidebar__spark"
          title={`Events per minute · last ${epm.length}m · current ${fmtNum(
            epm[epm.length - 1] ?? 0,
          )}`}
          style={{ padding: "0 12px 6px", color: "var(--accent)" }}
        >
          <Sparkline data={epm} height={20} />
        </div>
      ) : null}
      {NAV.map((item) => {
        const c = item.countSource ? counts[item.countSource] : undefined;
        return (
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
            {typeof c === "number" && c > 0 ? (
              <span className="nav-count">{fmtNum(c)}</span>
            ) : null}
          </button>
        );
      })}

      {overview && overview.recent_repos.length > 0 ? (
        <>
          <div
            className="sidebar__group-label"
            style={{
              marginTop: 14,
              display: "flex",
              alignItems: "center",
              justifyContent: "space-between",
            }}
          >
            <span>Repos · {overview.repos_indexed}</span>
            {filters.repo && filters.repo.length > 0 ? (
              <button
                onClick={() => setFilter("repo", undefined)}
                title="Clear repo filter"
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
          {overview.recent_repos.map((r: RepoCount) => (
            <RepoItem
              key={r.repo}
              repo={r}
              active={(filters.repo ?? []).includes(r.repo)}
              onClick={() => onRepoClick(r.repo)}
            />
          ))}
        </>
      ) : null}

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

function RepoItem({
  repo,
  active,
  onClick,
}: {
  repo: RepoCount;
  active: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      className={`nav-item ${active ? "is-active" : ""}`}
      onClick={onClick}
      title={`${repo.repo} · ${repo.count} events`}
    >
      <span
        className="nav-icon"
        style={{
          width: 8,
          height: 8,
          borderRadius: 2,
          background: "var(--accent-dim)",
          display: "inline-block",
        }}
      />
      <span className="nav-label mono" style={{ fontSize: 11.5 }}>
        {repo.repo}
      </span>
      <span className="nav-count">{fmtNum(repo.count)}</span>
    </button>
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
