/**
 * Graph explorer — Cytoscape.js renderer.
 *
 * Replaces the hand-rolled SVG renderer with a force-directed
 * Cytoscape canvas that handles pan, zoom, layout, and selection
 * out of the box. Reads from `/v1/dashboard/graph`, which itself
 * runs a real Cypher MATCH against Nexus when configured (and
 * falls back to a synthetic Session→Turn→ToolCall graph otherwise).
 */

import { useEffect, useMemo, useRef, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import cytoscape, {
  type Core,
  type ElementDefinition,
  type EventObjectNode,
  type NodeSingular,
} from "cytoscape";
// @ts-expect-error — cose-bilkent ships its own .d.ts but the wrapper
// is registered via cytoscape.use() so we don't need a typed handle.
import coseBilkent from "cytoscape-cose-bilkent";

import { Icon } from "../atoms/Icon";
import { api, type GraphEdge, type GraphNode } from "../lib/api";
import { useFilters } from "../lib/filters";

// Register the layout once. cytoscape.use() is idempotent — calling it
// twice across HMR / strict-mode re-mounts is harmless.
cytoscape.use(coseBilkent);

// Map our canonical kinds to design-system colors. Single source of
// truth: the legend below + the Cytoscape stylesheet both read from
// this object so they can never drift.
//
// Cytoscape parses these strings through Canvas 2D, which does NOT
// resolve CSS custom properties (`var(--*)`) — it just sees an
// invalid color and silently paints black. We therefore feed it
// resolved values via `cssVar()` at render time so a theme switch
// still flows through the legend AND the canvas.
const KIND_COLOR_VARS: Record<string, { token: string; fallback: string }> = {
  session: { token: "--info", fallback: "#7aa2f7" },
  turn: { token: "--info", fallback: "#7aa2f7" },
  tool_call: { token: "--accent", fallback: "#9ece6a" },
  agent_call: { token: "", fallback: "#bb9af7" },
  artifact: { token: "--fg-2", fallback: "#a9b1d6" },
  decision: { token: "--ok", fallback: "#9ece6a" },
  law: { token: "--critical", fallback: "#f7768e" },
  violation: { token: "--critical", fallback: "#f7768e" },
  analysis: { token: "", fallback: "#bb9af7" },
};

/// Resolve a CSS custom property to its computed value (or a hex
/// fallback when the property is absent / the document isn't yet
/// laid out). Always returns something Canvas 2D can parse.
function cssVar(token: string, fallback: string): string {
  if (typeof window === "undefined" || !token) return fallback;
  const v = getComputedStyle(document.documentElement)
    .getPropertyValue(token)
    .trim();
  return v.length > 0 ? v : fallback;
}

function resolvedKindColors(): Record<string, string> {
  const out: Record<string, string> = {};
  for (const [kind, def] of Object.entries(KIND_COLOR_VARS)) {
    out[kind] = cssVar(def.token, def.fallback);
  }
  return out;
}

/// Legend palette — uses `var(--*)` because the legend lives in the
/// DOM where browser CSS resolution works fine. Same intent as
/// `KIND_COLOR_VARS`, different rendering target.
const KIND_COLOR: Record<string, string> = {
  session: "var(--info)",
  turn: "var(--info)",
  tool_call: "var(--accent)",
  agent_call: "oklch(0.75 0.13 290)",
  artifact: "var(--fg-2)",
  decision: "var(--ok)",
  law: "var(--critical)",
  violation: "var(--critical)",
  analysis: "oklch(0.75 0.13 290)",
};

const KIND_LABEL: Record<string, string> = {
  session: "Session",
  turn: "Turn",
  tool_call: "ToolCall",
  agent_call: "AgentCall",
  artifact: "Artifact",
  decision: "Decision",
  law: "Law",
  violation: "Violation",
  analysis: "Analysis",
};

type SelectedNode = {
  id: string;
  label: string;
  kind: string;
  neighbors: number;
};

export function GraphView() {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const cyRef = useRef<Core | null>(null);
  const [selected, setSelected] = useState<SelectedNode | null>(null);
  const { filters } = useFilters();

  const { data, isLoading, error } = useQuery({
    queryKey: ["graph", filters.session_id ?? ""],
    queryFn: () => api.graph(filters.session_id, 80),
    refetchInterval: 12_000,
    refetchIntervalInBackground: true,
  });

  const elements: ElementDefinition[] = useMemo(() => {
    if (!data) return [];
    return [
      ...data.nodes.map((n: GraphNode) => ({
        data: { id: n.id, label: n.label, kind: n.kind },
      })),
      ...data.edges.map((e: GraphEdge, i: number) => ({
        data: {
          id: `${e.from}|${e.label}|${e.to}|${i}`,
          source: e.from,
          target: e.to,
          label: e.label,
        },
      })),
    ];
  }, [data]);

  // Mount + tear down the Cytoscape instance once. Element updates go
  // through cy.json() so we never destroy the GPU canvas across polls.
  useEffect(() => {
    if (!containerRef.current) return;
    const cy = cytoscape({
      container: containerRef.current,
      elements: [],
      style: cytoscapeStyle(),
      layout: { name: "preset" },
      wheelSensitivity: 0.25,
      minZoom: 0.2,
      maxZoom: 3,
    });
    cyRef.current = cy;

    cy.on("tap", "node", (evt: EventObjectNode) => {
      const n = evt.target as NodeSingular;
      setSelected({
        id: n.id(),
        label: n.data("label") ?? n.id(),
        kind: n.data("kind") ?? "unknown",
        neighbors: n.neighborhood("node").length,
      });
    });
    cy.on("tap", (evt) => {
      if (evt.target === cy) setSelected(null);
    });

    return () => {
      cy.destroy();
      cyRef.current = null;
    };
  }, []);

  // Reflect new fetch results into the existing instance — diff
  // elements and re-run the layout only when the topology changed.
  useEffect(() => {
    const cy = cyRef.current;
    if (!cy) return;
    cy.batch(() => {
      cy.elements().remove();
      cy.add(elements);
    });
    if (elements.length > 0) {
      cy.layout({
        name: "cose-bilkent",
        animate: false,
        nodeDimensionsIncludeLabels: true,
        idealEdgeLength: 110,
        nodeRepulsion: 7500,
        gravity: 0.15,
        randomize: false,
      } as cytoscape.LayoutOptions).run();
      cy.fit(undefined, 32);
    }
  }, [elements]);

  const nodeCount = data?.nodes.length ?? 0;
  const edgeCount = data?.edges.length ?? 0;

  const onFit = () => cyRef.current?.fit(undefined, 32);
  const onZoomIn = () => {
    const cy = cyRef.current;
    if (!cy) return;
    cy.zoom({ level: cy.zoom() * 1.25, renderedPosition: viewportCenter(cy) });
  };
  const onZoomOut = () => {
    const cy = cyRef.current;
    if (!cy) return;
    cy.zoom({ level: cy.zoom() * 0.8, renderedPosition: viewportCenter(cy) });
  };

  return (
    <div className="view">
      <div className="view__head">
        <div>
          <h1 className="view__title">Graph explorer</h1>
          <p className="view__subtitle">
            Cypher MATCH against Nexus — Session → Turn → ToolCall today, with
            Decision / Law / Violation / Analysis joining as the spec-07
            payload-parsing round lands. When Nexus is unreachable the
            endpoint falls back to a synthetic graph derived from the keyword
            lane.
          </p>
        </div>
      </div>

      <div className="graph-wrap">
        <div className="graph-canvas">
          <div className="graph-toolbar">
            <button className="btn btn--sm" onClick={onFit} title="Fit to viewport">
              Fit
            </button>
            <button className="btn btn--sm" onClick={onZoomIn} title="Zoom in">
              +
            </button>
            <button className="btn btn--sm" onClick={onZoomOut} title="Zoom out">
              −
            </button>
          </div>
          <div className="graph-legend">
            {Object.entries(KIND_COLOR).map(([kind, color]) => (
              <span key={kind} className="legend-item">
                <span className="legend-dot" style={{ background: color }} />
                {KIND_LABEL[kind] ?? kind}
              </span>
            ))}
          </div>
          {error ? <CenterMsg msg="cortex-api unreachable." /> : null}
          {isLoading ? <CenterMsg msg="Loading graph…" /> : null}
          {!isLoading && !error && nodeCount === 0 ? (
            <CenterMsg msg="No graph data yet. Capture a Claude Code session with the Cortex plugin to populate it." />
          ) : null}
          <div
            ref={containerRef}
            className="graph-cy"
            style={{ position: "absolute", inset: 0 }}
          />
        </div>
        <div className="card">
          <div className="card__head">
            <span className="card__title">Selection</span>
          </div>
          <div className="card__body">
            {selected ? (
              <>
                <div
                  style={{
                    display: "flex",
                    alignItems: "center",
                    gap: 8,
                    marginBottom: 10,
                  }}
                >
                  <span
                    className="legend-dot"
                    style={{
                      background: KIND_COLOR[selected.kind] ?? "var(--fg-2)",
                      width: 12,
                      height: 12,
                    }}
                  />
                  <span style={{ fontWeight: 600, color: "var(--fg-0)" }}>
                    {KIND_LABEL[selected.kind] ?? selected.kind}
                  </span>
                </div>
                <dl className="kv-list">
                  <dt>id</dt>
                  <dd
                    className="mono"
                    style={{
                      wordBreak: "break-all",
                      whiteSpace: "normal",
                    }}
                  >
                    {selected.id}
                  </dd>
                  <dt>label</dt>
                  <dd>{selected.label}</dd>
                  <dt>kind</dt>
                  <dd className="mono">{selected.kind}</dd>
                  <dt>neighbors</dt>
                  <dd className="mono tabular">{selected.neighbors}</dd>
                </dl>
              </>
            ) : (
              <div
                style={{
                  fontSize: 11.5,
                  color: "var(--fg-3)",
                  marginBottom: 12,
                }}
              >
                <Icon name="graph" size={13} /> Click a node to inspect.
              </div>
            )}
            <div className="divider" />
            <dl className="kv-list">
              <dt>nodes</dt>
              <dd className="mono tabular">{nodeCount}</dd>
              <dt>edges</dt>
              <dd className="mono tabular">{edgeCount}</dd>
              <dt>filter</dt>
              <dd className="mono">
                {filters.session_id
                  ? filters.session_id.slice(0, 12) + "…"
                  : "all sessions"}
              </dd>
              <dt>refresh</dt>
              <dd className="mono">12 s</dd>
            </dl>
          </div>
        </div>
      </div>
    </div>
  );
}

function viewportCenter(cy: Core): { x: number; y: number } {
  const ext = cy.extent();
  return {
    x: (ext.x1 + ext.x2) / 2,
    y: (ext.y1 + ext.y2) / 2,
  };
}

function cytoscapeStyle(): cytoscape.StylesheetStyle[] {
  // Resolve every theme color up-front. Canvas 2D doesn't understand
  // `var(--*)` and silently paints black on parse failure, which is
  // why labels used to render as black-on-black rectangles. Numbers
  // are tuned per theme via design tokens; fallback hex covers the
  // case where the document isn't laid out yet.
  const fg1 = cssVar("--fg-1", "#a9b1d6");
  const fg2 = cssVar("--fg-2", "#9aa5ce");
  const fg3 = cssVar("--fg-3", "#737aa2");
  const bg1 = cssVar("--bg-1", "#1a1b26");
  const accent = cssVar("--accent", "#9ece6a");
  const borderStrong = cssVar("--border-strong", "#414868");
  const fontMono = cssVar(
    "--font-mono",
    "ui-monospace, SFMono-Regular, monospace",
  );

  // Map kinds to colors via attribute selectors — same palette the
  // legend renders, but with values Canvas can actually parse.
  const palette = resolvedKindColors();
  const nodeKindStyles: cytoscape.StylesheetStyle[] = Object.entries(palette).map(
    ([kind, color]) => ({
      selector: `node[kind = "${kind}"]`,
      style: {
        "background-color": color,
        "border-color": color,
      },
    }),
  );

  return [
    {
      selector: "node",
      style: {
        "background-color": fg2,
        "border-color": fg2,
        "border-width": 1.5,
        "border-opacity": 1,
        width: 20,
        height: 20,
        label: "data(label)",
        color: fg1,
        "font-family": fontMono,
        "font-size": 10,
        "text-valign": "bottom",
        "text-halign": "center",
        "text-margin-y": 6,
        "text-wrap": "ellipsis",
        "text-max-width": "160px",
        "text-background-color": bg1,
        "text-background-opacity": 0.85,
        "text-background-padding": "2px",
      },
    },
    ...nodeKindStyles,
    {
      selector: "node:selected",
      style: {
        "border-width": 3,
        "border-color": accent,
        width: 26,
        height: 26,
      },
    },
    {
      selector: "edge",
      style: {
        width: 1.4,
        "line-color": borderStrong,
        "target-arrow-color": fg3,
        "target-arrow-shape": "triangle",
        "curve-style": "bezier",
        label: "data(label)",
        "font-family": fontMono,
        "font-size": 8,
        color: fg3,
        "text-rotation": "autorotate",
        "text-margin-y": -4,
      },
    },
    {
      selector: "edge:selected",
      style: {
        "line-color": accent,
        "target-arrow-color": accent,
        width: 2,
      },
    },
  ];
}

function CenterMsg({ msg }: { msg: string }) {
  return (
    <div
      style={{
        position: "absolute",
        inset: 0,
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
        color: "var(--fg-3)",
        fontSize: 12,
        textAlign: "center",
        padding: 32,
        zIndex: 1,
        pointerEvents: "none",
      }}
    >
      {msg}
    </div>
  );
}
