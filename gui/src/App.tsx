import { useCallback, useMemo, useState } from "react";

import { Header } from "./shell/Header";
import { Sidebar, type ViewId } from "./shell/Sidebar";
import { Tweaks } from "./shell/Tweaks";
import { WindowChrome } from "./shell/WindowChrome";
import { TweaksProvider } from "./lib/useTweaks";
import { ConnectionsProvider } from "./lib/connections";
import { ApiResolverBinding } from "./lib/connections/registerApi";
import { ApiKeyPromptHost } from "./shell/ApiKeyPrompt";
import { ConnectionsView } from "./views/Connections";
import { TimelineView } from "./views/Timeline";
import { MemoryView } from "./views/Memory";
import { DecisionsView } from "./views/Decisions";
import { LawsView } from "./views/Laws";
import { AnalysisView } from "./views/Analysis";
import { ConsolidationsView } from "./views/Consolidations";
import { TasksView } from "./views/Tasks";
import { ToolsView } from "./views/Tools";
import { GraphView } from "./views/Graph";
import { ConversationsView } from "./views/Conversations";
import { HandoffsView } from "./views/Handoffs";
import { ClassificationsView } from "./views/Classifications";
import { HealthView } from "./views/Health";
import { BitemporalTimelineView } from "./views/BitemporalTimeline";
import { BranchExplorerView } from "./views/BranchExplorer";
import { EntityHistoryView } from "./views/EntityHistory";
import { CanaryView } from "./views/Canary";
import { EMPTY_FILTERS, FiltersContext, type FiltersContextValue } from "./lib/filters";
import type { Filters } from "./lib/api";
import { useDashboardStream } from "./lib/useDashboardStream";

export function App() {
  return (
    <ConnectionsProvider>
      <ApiResolverBinding>
        <TweaksProvider>
          <AppShell />
        </TweaksProvider>
      </ApiResolverBinding>
    </ConnectionsProvider>
  );
}

function AppShell() {
  const [view, setView] = useState<ViewId>("timeline");
  const [tweaksOpen, setTweaksOpen] = useState(false);
  const [filters, setFiltersState] = useState<Filters>(EMPTY_FILTERS);
  // Spec 21 — single dashboard SSE stream. Subscribing here once at the
  // app shell guarantees every view's react-query cache gets push
  // invalidations regardless of which view is mounted. The status
  // snapshot drives the connection pill in the header.
  const dashboardStream = useDashboardStream();

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
      case "consolidations":
        return <ConsolidationsView />;
      case "tasks":
        return <TasksView />;
      case "tools":
        return <ToolsView />;
      case "graph":
        return <GraphView />;
      case "health":
        return <HealthView />;
      case "canary":
        return <CanaryView />;
      case "bitemporal-timeline":
        return <BitemporalTimelineView />;
      case "branch-explorer":
        return <BranchExplorerView />;
      case "entity-history":
        return <EntityHistoryView />;
      case "connections":
        return <ConnectionsView />;
      default:
        return null;
    }
  };

  return (
    <FiltersContext.Provider value={filtersValue}>
      <div className="app-shell">
        <WindowChrome />
        <div className="app">
          <Header
            onOpenTweaks={() => setTweaksOpen(true)}
            onJumpToHealth={() => setView("health")}
            onJumpToConnections={() => setView("connections")}
            dashboardStream={dashboardStream}
          />
          <Sidebar view={view} setView={setView} />
          <main className="main">{renderView()}</main>
          <Tweaks open={tweaksOpen} onClose={() => setTweaksOpen(false)} />
          <ApiKeyPromptHost />
        </div>
      </div>
    </FiltersContext.Provider>
  );
}
