import { useEffect, useState } from "react";

import { Header } from "./shell/Header";
import { Sidebar, type ViewId } from "./shell/Sidebar";
import { TimelineView } from "./views/Timeline";
import { MemoryView } from "./views/Memory";
import { DecisionsView } from "./views/Decisions";
import { LawsView } from "./views/Laws";
import { AnalysisView } from "./views/Analysis";
import { ToolsView } from "./views/Tools";
import { GraphView } from "./views/Graph";

export function App() {
  const [view, setView] = useState<ViewId>("timeline");
  const [collapsed, setCollapsed] = useState(false);
  const [theme, setTheme] = useState<"dark" | "light">("dark");

  useEffect(() => {
    document.documentElement.dataset.theme = theme;
  }, [theme]);

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
  );
}
