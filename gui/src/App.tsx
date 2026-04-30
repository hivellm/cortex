import { useCallback, useMemo, useState } from "react";

import { Header } from "./shell/Header";
import { Sidebar, type ViewId } from "./shell/Sidebar";
import { Tweaks } from "./shell/Tweaks";
import { TimelineView } from "./views/Timeline";
import { MemoryView } from "./views/Memory";
import { RetentionView } from "./views/Retention";
import { DecisionsView } from "./views/Decisions";
import { LawsView } from "./views/Laws";
import { AnalysisView } from "./views/Analysis";
import { ToolsView } from "./views/Tools";
import { GraphView } from "./views/Graph";
import { ConversationsView } from "./views/Conversations";
import { HandoffsView } from "./views/Handoffs";
import { ClassificationsView } from "./views/Classifications";
import { HealthView } from "./views/Health";
import { EMPTY_FILTERS, FiltersContext, type FiltersContextValue } from "./lib/filters";
import { TweaksProvider, useTweaks } from "./lib/useTweaks";
import type { Filters } from "./lib/api";

export function App() {
  return (
    <TweaksProvider>
      <AppShell />
    </TweaksProvider>
  );
}

function AppShell() {
  const { tweaks, setTweak } = useTweaks();
  const collapsed = tweaks.sidebarCollapsed;
  const onToggleSidebar = useCallback(
    () => setTweak("sidebarCollapsed", !collapsed),
    [collapsed, setTweak],
  );

  const [view, setView] = useState<ViewId>("timeline");
  const [tweaksOpen, setTweaksOpen] = useState(false);
  const [filters, setFiltersState] = useState<Filters>(EMPTY_FILTERS);

  const setFilter = useCallback(
    <K extends keyof Filters>(key: K, value: Filters[K] | undefined) => {
      setFiltersState((prev) => {
        const next = { ...prev };
        if (value === undefined || value === "") delete next[key];
        else next[key] = value;
        return next;
      });
    },
    [],
  );

  const filtersValue: FiltersContextValue = useMemo(
    () => ({
      filters,
      setFilters: setFiltersState,
      setFilter,
      clearFilters: () => setFiltersState(EMPTY_FILTERS),
    }),
    [filters, setFilter],
  );

  const renderView = () => {
    switch (view) {
      case "timeline":
        return <TimelineView />;
      case "conversations":
        return <ConversationsView />;
      case "memory":
        return <MemoryView />;
      case "retention":
        return <RetentionView />;
      case "decisions":
        return <DecisionsView />;
      case "handoffs":
        return <HandoffsView />;
      case "classifications":
        return <ClassificationsView />;
      case "laws":
        return <LawsView />;
      case "analysis":
        return <AnalysisView />;
      case "tools":
        return <ToolsView />;
      case "graph":
        return <GraphView />;
      case "health":
        return <HealthView />;
      default:
        return null;
    }
  };

  return (
    <FiltersContext.Provider value={filtersValue}>
      <div className={`app ${collapsed ? "collapsed" : ""}`}>
        <Header
          collapsed={collapsed}
          onToggleSidebar={onToggleSidebar}
          onOpenTweaks={() => setTweaksOpen(true)}
          onJumpToHealth={() => setView("health")}
        />
        <Sidebar view={view} setView={setView} collapsed={collapsed} />
        <main className="main">{renderView()}</main>
        <Tweaks open={tweaksOpen} onClose={() => setTweaksOpen(false)} />
      </div>
    </FiltersContext.Provider>
  );
}
