/**
 * Graph explorer — Sigma.js + graphology renderer.
 *
 * Replaces the previous Cytoscape canvas with a WebGL renderer that ships
 * pan / zoom / arrowheads / edge labels / hover-neighborhood highlighting
 * natively. Reads from `/v1/dashboard/graph`, which itself runs a real
 * Cypher MATCH against Nexus when configured (and falls back to a
 * synthetic Session → Turn → ToolCall graph otherwise).
 */

import { useEffect, useMemo, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import {
  SigmaContainer,
  useLoadGraph,
  useRegisterEvents,
  useSigma,
  ControlsContainer,
  ZoomControl,
  FullScreenControl,
} from "@react-sigma/core";
import "@react-sigma/core/lib/style.css";
import Graph from "graphology";
import forceAtlas2 from "graphology-layout-forceatlas2";

import { Icon } from "../atoms/Icon";
import { api, type GraphPayload } from "../lib/api";
import { useFilters } from "../lib/filters";

// Single source of truth for node colors. Same hex table is used
// by the loader's `KIND_HEX` so the legend dots match the canvas.
const KIND_FALLBACK: Record<string, string> = {
  repo: "#e0af68",       // amber — project anchors
  session: "#7aa2f7",    // blue — entry points
  turn: "#7dcfff",       // cyan — conversation steps
  decision: "#9ece6a",   // green — accepted decisions
  law: "#f7768e",        // rose — rules
  violation: "#ff9e64",  // orange — rule breaks
  memory: "#bb9af7",     // purple — pinned context
  analysis: "#c0caf5",   // lavender — analyses
  agent_call: "#73daca", // mint — sub-agent invocations
};

const KIND_LABEL: Record<string, string> = {
  repo: "Repo",
  session: "Session",
  turn: "Turn",
  decision: "Decision",
  law: "Law",
  violation: "Violation",
  memory: "Memory",
  analysis: "Analysis",
  agent_call: "AgentCall",
};

function cssVar(token: string, fallback: string): string {
  if (typeof window === "undefined" || !token) return fallback;
  const v = getComputedStyle(document.documentElement)
    .getPropertyValue(token)
    .trim();
  return v.length > 0 ? v : fallback;
}

function kindColor(kind: string): string {
  // Sigma's WebGL renderer parses node colors with a strict regex
  // that accepts hex / rgb() / rgba() but not `oklch()` — and every
  // CSS variable in this theme is an `oklch(...)` string, so feeding
  // `cssVar("--info")` straight in renders every node black. We
  // resolve to the hex fallback table directly; the theme switcher
  // doesn't recolor the canvas, but at least nodes are *visible*.
  return KIND_FALLBACK[kind] ?? "#9aa5ce";
}

type SelectedNode = {
  id: string;
  label: string;
  kind: string;
  neighbors: number;
};

function GraphLoader({
  data,
  onSelect,
}: {
  data: GraphPayload | undefined;
  onSelect: (s: SelectedNode | null) => void;
}) {
  const sigma = useSigma();
  const loadGraph = useLoadGraph();
  const registerEvents = useRegisterEvents();
  const [hovered, setHovered] = useState<string | null>(null);

  useEffect(() => {
    if (!data) return;
    const g = new Graph({ multi: true, type: "directed" });
    const ids = new Set<string>();

    // Step 0 — collapse the long tail before drawing anything.
    //
    // The full panorama (~23k nodes) is dominated by Artifacts and
    // ToolCalls whose only meaningful relationship is a single
    // IN_REPO / HAS_TOOL_CALL hop into their owning repo or session.
    // Rendering them turns the canvas into a black mass with no
    // discoverable structure — the explorer only earns its keep when
    // it surfaces *interconnections*. We keep one tier of nodes:
    //
    //   - Repos: the project anchors.
    //   - Sessions: the conversation entry points.
    //   - Decisions / Laws / Analyses / LawViolations: the
    //     institutional knowledge that crosses projects.
    //   - Memories: long-lived context the user pinned.
    //   - Turns: the bridge between Sessions and the rest of the
    //     session tree.
    //   - AgentCalls: rare cross-cutting events.
    //
    // Artifacts and ToolCalls are summarised into per-repo /
    // per-session degree counts (used below to size the surviving
    // hub) but never rendered as their own nodes. The result is a
    // ~100-node graph that reads like the spec-07 schema diagram
    // instead of a hairball.
    const STRUCTURAL_KINDS = new Set([
      "repo",
      "session",
      "turn",
      "decision",
      "law",
      "violation",
      "memory",
      "analysis",
      "agent_call",
    ]);
    const inboundCount = new Map<string, number>();
    for (const e of data.edges) {
      inboundCount.set(e.to, (inboundCount.get(e.to) ?? 0) + 1);
      inboundCount.set(e.from, (inboundCount.get(e.from) ?? 0) + 1);
    }
    const survivingIds = new Set<string>();
    for (const n of data.nodes) {
      if (STRUCTURAL_KINDS.has(n.kind)) {
        survivingIds.add(n.id);
      }
    }


    // Step 0.5 — promote Session labels to "first turn message".
    //
    // The mapper writes `name = "Session <ulid12>"` on Session nodes,
    // which is barely better than the raw ULID. But the conversation
    // itself has a great natural title: the first user prompt. We
    // walk the HAS_TURN edges, pick the earliest Turn id (lexicographic
    // order works because turn ids are ULIDs — they sort by creation
    // time), and promote that Turn's label onto its owning Session.
    const sessionFirstTurn = new Map<string, string>();
    const turnLabelById = new Map<string, string>();
    for (const n of data.nodes) {
      if (n.kind === "turn") turnLabelById.set(n.id, n.label);
    }
    for (const e of data.edges) {
      if (e.label !== "HAS_TURN") continue;
      const cur = sessionFirstTurn.get(e.from);
      if (!cur || e.to < cur) sessionFirstTurn.set(e.from, e.to);
    }
    const promotedSessionLabel = new Map<string, string>();
    for (const [sessionId, firstTurnId] of sessionFirstTurn) {
      const turnLabel = turnLabelById.get(firstTurnId);
      if (turnLabel && !turnLabel.startsWith("Turn ")) {
        promotedSessionLabel.set(sessionId, turnLabel);
      }
    }

    // Step 1 — palette per kind (Sigma WebGL requires hex; CSS
    // `oklch(...)` vars render as black). One stable hue per kind
    // makes the legend match the canvas.
    const KIND_HEX: Record<string, string> = {
      repo: "#e0af68",      // amber — project anchors
      session: "#7aa2f7",   // blue — entry points
      turn: "#7dcfff",      // cyan — conversation steps
      decision: "#9ece6a",  // green — accepted decisions
      law: "#f7768e",       // rose — rules
      violation: "#ff9e64", // orange — rule breaks
      memory: "#bb9af7",    // purple — pinned context
      analysis: "#c0caf5",  // lavender — analyses
      agent_call: "#73daca",// mint — sub-agent invocations
    };

    // Step 2 — per-node degree (using the *full* edge set, not just
    // the surviving slice) so a Session that orchestrated 200 tool
    // calls renders bigger than a Session with 5 turns. The visual
    // hierarchy mirrors importance.
    const degree = inboundCount;
    const sizeFor = (id: string, kind: string): number => {
      const d = degree.get(id) ?? 0;
      // log-scale so a 1000-tool-call session doesn't dwarf the
      // canvas — the eye still ranks the order.
      const base = 4 + Math.log2(1 + d) * 2.5;
      // Repos and Decisions are anchors regardless of degree.
      if (kind === "repo") return Math.max(base, 18);
      if (kind === "decision" || kind === "law") return Math.max(base, 12);
      if (kind === "session") return Math.max(base, 10);
      return Math.max(base, 5);
    };

    for (const n of data.nodes) {
      if (!survivingIds.has(n.id)) continue;
      if (g.hasNode(n.id)) continue;
      ids.add(n.id);
      // Use the promoted "first turn" label on Sessions when we have
      // one; otherwise show whatever the backend gave us. Strip
      // pure-ULID labels so only the real-name strings reach the
      // canvas (a 26-char ULID renders as visual noise).
      let display = n.label ?? n.id;
      if (n.kind === "session" && promotedSessionLabel.has(n.id)) {
        display = promotedSessionLabel.get(n.id)!;
      }
      // Hide labels that are still raw ULIDs (failed to resolve to
      // a useful `name` upstream) so the canvas reads cleanly.
      const isRawUlid =
        /^[0-9A-HJKMNP-TV-Z]{26}$/i.test(display) ||
        /^Memory [0-9A-Z]{12}$/i.test(display) ||
        /^Session [0-9A-Z]{12}$/i.test(display) ||
        /^Turn [0-9A-Z]{12}$/i.test(display) ||
        /^Decision [0-9A-Z]{12,}$/i.test(display);
      const labelToShow = isRawUlid ? "" : display;
      g.addNode(n.id, {
        label: labelToShow,
        kind: n.kind,
        size: sizeFor(n.id, n.kind),
        color: KIND_HEX[n.kind] ?? "#9aa5ce",
        // Force-show labels for the headline nodes — Repos / Sessions /
        // Decisions / Laws — so the canvas reads as a structured map
        // instead of an anonymous cloud. Smaller nodes (Turns,
        // Memories, Violations) reveal labels on hover and zoom-in.
        forceLabel:
          n.kind === "repo" ||
          n.kind === "session" ||
          n.kind === "decision" ||
          n.kind === "law",
      });
    }

    // Step 3 — surviving edges only. Drop everything that touches a
    // filtered-out node; what remains is the institutional skeleton.
    const edgeBaseColor = "#3d4868";
    const edgeBridgeColor = "#e0af68";
    let i = 0;
    for (const e of data.edges) {
      if (!ids.has(e.from) || !ids.has(e.to)) continue;
      // SUPERSEDES, OBSERVED_IN, OF, REMEMBERS are the cross-cutting
      // links that move knowledge between projects / sessions /
      // domains. Highlight them with a warmer hue so they pop against
      // the structural HAS_TURN / HAS_TOOL_CALL backbone.
      const isBridge =
        e.label === "SUPERSEDES" ||
        e.label === "OBSERVED_IN" ||
        e.label === "OF" ||
        e.label === "LINKED_TO";
      g.addEdgeWithKey(`e-${i++}`, e.from, e.to, {
        label: e.label,
        size: isBridge ? 1.6 : 0.9,
        type: "arrow",
        color: isBridge ? edgeBridgeColor : edgeBaseColor,
      });
    }

    // Step 4 — circular pre-seed + ForceAtlas2 refinement. The
    // example reference (Star Wars character network) is a classic
    // force-directed graph: nodes seeded on a circle, then attracted
    // toward connected neighbours so clusters self-organise. With
    // ~100 surviving nodes the layout converges in well under a
    // second and produces the readable hub-and-spoke shape the user
    // pointed at.
    if (g.order > 0) {
      const radius = 600;
      const orderArr = Array.from(g.nodes());
      orderArr.forEach((nodeId, idx) => {
        const angle = (idx * 2 * Math.PI) / orderArr.length;
        g.setNodeAttribute(nodeId, "x", radius * Math.cos(angle));
        g.setNodeAttribute(nodeId, "y", radius * Math.sin(angle));
      });
      forceAtlas2.assign(g, {
        iterations: 400,
        settings: {
          gravity: 1.2,
          scalingRatio: 12,
          slowDown: 4,
          barnesHutOptimize: g.order > 100,
          adjustSizes: true,
          outboundAttractionDistribution: false,
          strongGravityMode: false,
        },
      });
    }

    // Sigma settings — labels at smaller sizes than default, edge
    // labels off (with this many edges they overlap).
    sigma.setSetting("renderEdgeLabels", false);
    sigma.setSetting("labelRenderedSizeThreshold", 6);
    sigma.setSetting("defaultNodeColor", "#9aa5ce");

    loadGraph(g);
  }, [data, loadGraph, sigma]);

  useEffect(() => {
    registerEvents({
      enterNode: ({ node }) => setHovered(node),
      leaveNode: () => setHovered(null),
      clickNode: ({ node }) => {
        const g = sigma.getGraph();
        if (!g.hasNode(node)) return;
        onSelect({
          id: node,
          label: g.getNodeAttribute(node, "label") ?? node,
          kind: g.getNodeAttribute(node, "kind") ?? "unknown",
          neighbors: g.degree(node),
        });
      },
      clickStage: () => onSelect(null),
    });
  }, [registerEvents, sigma, onSelect]);

  useEffect(() => {
    const dimColor = "#2a2e3a";
    sigma.setSetting("nodeReducer", (node, attrs) => {
      if (!hovered) return attrs;
      const g = sigma.getGraph();
      if (node === hovered || g.areNeighbors(hovered, node)) return attrs;
      return { ...attrs, color: dimColor, label: "" };
    });
    sigma.setSetting("edgeReducer", (edge, attrs) => {
      if (!hovered) return attrs;
      const g = sigma.getGraph();
      const [s, t] = g.extremities(edge);
      if (s === hovered || t === hovered) return attrs;
      return { ...attrs, hidden: true };
    });
    sigma.refresh();
  }, [sigma, hovered]);

  return null;
}

export function GraphView() {
  const { filters } = useFilters();
  const [selected, setSelected] = useState<SelectedNode | null>(null);

  const { data, isLoading, error } = useQuery({
    queryKey: ["graph", filters.session_id ?? ""],
    queryFn: () => api.graph(filters.session_id, 30_000),
    // The full panorama is multiple MB on the wire and a few seconds
    // of FA2 layout — refetching every 12 s would peg the CPU on a
    // laptop. 60 s is the closest cadence the dashboard actually
    // benefits from (live capture lands new turns at human speed).
    refetchInterval: 60_000,
    refetchIntervalInBackground: false,
  });

  // Count what survives the structural filter the loader applies, not
  // the full payload — the user's mental model is "how many shapes
  // does the canvas show", not "how many rows did the backend send".
  const STRUCTURAL_KINDS = useMemo(
    () =>
      new Set([
        "repo",
        "session",
        "turn",
        "decision",
        "law",
        "violation",
        "memory",
        "analysis",
        "agent_call",
      ]),
    [],
  );
  const renderedNodes = useMemo(
    () =>
      data?.nodes.filter((n) => STRUCTURAL_KINDS.has(n.kind)).length ?? 0,
    [data, STRUCTURAL_KINDS],
  );
  const renderedEdges = useMemo(() => {
    if (!data) return 0;
    const survivors = new Set(
      data.nodes.filter((n) => STRUCTURAL_KINDS.has(n.kind)).map((n) => n.id),
    );
    let n = 0;
    for (const e of data.edges) {
      if (survivors.has(e.from) && survivors.has(e.to)) n++;
    }
    return n;
  }, [data, STRUCTURAL_KINDS]);
  const nodeCount = renderedNodes;
  const edgeCount = renderedEdges;
  const totalNodes = data?.nodes.length ?? 0;

  const sigmaSettings = useMemo(
    () => ({
      renderEdgeLabels: true,
      defaultEdgeType: "arrow",
      labelFont: cssVar(
        "--font-mono",
        "ui-monospace, SFMono-Regular, monospace",
      ),
      labelSize: 11,
      labelWeight: "500",
      labelColor: { color: cssVar("--fg-1", "#a9b1d6") },
      edgeLabelFont: cssVar(
        "--font-mono",
        "ui-monospace, SFMono-Regular, monospace",
      ),
      edgeLabelSize: 9,
      edgeLabelColor: { color: cssVar("--fg-3", "#737aa2") },
      labelDensity: 0.7,
      labelGridCellSize: 80,
      labelRenderedSizeThreshold: 6,
      minCameraRatio: 0.1,
      maxCameraRatio: 4,
    }),
    [],
  );

  return (
    <div className="view">
      <div className="view__head">
        <div>
          <h1 className="view__title">Graph explorer</h1>
          <p className="view__subtitle">
            Cypher MATCH against Nexus — Session → Turn → ToolCall today, with
            Decision / Law / Violation / Analysis joining as the spec-07
            payload-parsing round lands. When Nexus is unreachable the endpoint
            falls back to a synthetic graph derived from the keyword lane.
          </p>
        </div>
      </div>

      <div className="graph-wrap">
        <div className="graph-canvas">
          <div className="graph-legend">
            {Object.entries(KIND_LABEL).map(([kind, label]) => (
              <span key={kind} className="legend-item">
                <span
                  className="legend-dot"
                  style={{ background: kindColor(kind) }}
                />
                {label}
              </span>
            ))}
          </div>
          {error ? <CenterMsg msg="cortex-api unreachable." /> : null}
          {isLoading ? <CenterMsg msg="Loading graph…" /> : null}
          {!isLoading && !error && nodeCount === 0 ? (
            <CenterMsg msg="No graph data yet. Capture a Claude Code session with the Cortex plugin to populate it." />
          ) : null}
          <SigmaContainer
            style={{
              position: "absolute",
              inset: 0,
              background: "transparent",
            }}
            settings={sigmaSettings}
          >
            <GraphLoader data={data} onSelect={setSelected} />
            <ControlsContainer position="top-left">
              <ZoomControl />
              <FullScreenControl />
            </ControlsContainer>
          </SigmaContainer>
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
                      background: kindColor(selected.kind),
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
                    style={{ wordBreak: "break-all", whiteSpace: "normal" }}
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
                <Icon name="graph" size={13} /> Hover to highlight, click to
                inspect.
              </div>
            )}
            <div className="divider" />
            <dl className="kv-list">
              <dt>shown</dt>
              <dd className="mono tabular">{nodeCount} nodes</dd>
              <dt>edges</dt>
              <dd className="mono tabular">{edgeCount}</dd>
              <dt>hidden</dt>
              <dd className="mono tabular">
                {(totalNodes - nodeCount).toLocaleString()} leaf
              </dd>
              <dt>filter</dt>
              <dd className="mono">
                {filters.session_id
                  ? filters.session_id.slice(0, 12) + "..."
                  : "all sessions"}
              </dd>
              <dt>refresh</dt>
              <dd className="mono">60 s</dd>
            </dl>
          </div>
        </div>
      </div>
    </div>
  );
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
