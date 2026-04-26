import { useCallback, useEffect, useMemo, useState } from "react";

import { Header } from "./shell/Header";
import { Sidebar, type ViewId } from "./shell/Sidebar";
import { TimelineView } from "./views/Timeline";
import { MemoryView } from "./views/Memory";
import { DecisionsView } from "./views/Decisions";
import { LawsView } from "./views/Laws";
import { AnalysisView } from "./views/Analysis";
import { ToolsView } from "./views/Tools";
import { GraphView } from "./views/Graph";
import { EMPTY_FILTERS, FiltersContext, type FiltersContextValue } from "./lib/filters";
import type { Filters } from "./lib/api";

export function App() {
  const [view, setView] = useState<ViewId>("timeline");
  const [collapsed, setCollapsed] = useState(false);
  const [theme, setTheme] = useState<"dark" | "light">("dark");
  const [filters, setFiltersState] = useState<Filters>(EMPTY_FILTERS);

  useEffect(() => {
    document.documentElement.dataset.theme = theme;
  }, [theme]);

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
      case "memory":
        return <MemoryView />;
      case "decisions":
        return <DecisionsView />;
      case "laws":
        return <LawsView />;
      case "analysis":
        return <AnalysisView />;
      case "tools":
        return <ToolsView />;
      case "graph":
        return <GraphView />;
      default:
        return null;
    }
  };

  return (
    <FiltersContext.Provider value={filtersValue}>
      <div className={`app ${collapsed ? "collapsed" : ""}`}>
        <Header
          collapsed={collapsed}
          onToggleSidebar={() => setCollapsed((c) => !c)}
          theme={theme}
          onToggleTheme={() => setTheme((t) => (t === "dark" ? "light" : "dark"))}
        />
        <Sidebar view={view} setView={setView} collapsed={collapsed} />
        <main className="main">{renderView()}</main>
      </div>
    </FiltersContext.Provider>
  );
}
