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
import { RepoMultiSelect } from "../atoms/RepoMultiSelect";
import { api, type GraphPayload } from "../lib/api";
import { useFilters } from "../lib/filters";
import { useConnKey } from "../lib/connections/useConnKey";

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

type LoaderMode = "panorama" | "drilldown" | "correlation";

type SynthEdge = { from: string; to: string; label: string };

function GraphLoader({
  data,
  onSelect,
  onRepoClick,
  mode,
  selectedRepos,
  sessionPrimaryRepo,
  syntheticEdges,
}: {
  data: GraphPayload | undefined;
  onSelect: (s: SelectedNode | null) => void;
  onRepoClick: (repoId: string) => void;
  mode: LoaderMode;
  selectedRepos: string[];
  sessionPrimaryRepo: Map<string, string>;
  syntheticEdges: SynthEdge[];
}) {
  const sigma = useSigma();
  const loadGraph = useLoadGraph();
  const registerEvents = useRegisterEvents();
  const [hovered, setHovered] = useState<string | null>(null);

  useEffect(() => {
    if (!data) return;
    const g = new Graph({ multi: true, type: "directed" });
    const ids = new Set<string>();

    // Step 0 — pick what survives, mode by mode.
    //
    // Three modes, three different stories the canvas should tell:
    //
    //   * panorama (no filter): the institutional spine across ALL
    //     repos. Just Repos + Decisions + Laws + Analyses. Sessions /
    //     Turns / Memories / Violations / AgentCalls are dropped here
    //     because at 18 repos they explode into a 4k-node hairball
    //     with no readable structure. The user picks a project (or
    //     two) to bring those back in.
    //
    //   * drilldown (exactly 1 repo selected): the full subtree of
    //     that repo. Sessions, Turns, Memories, Decisions, Analyses,
    //     Violations, AgentCalls, plus the long-tail Artifacts /
    //     ToolCalls reachable from them. This is the "I want to see
    //     everything in `cortex`" view.
    //
    //   * correlation (2+ repos selected): the selected Repos PLUS
    //     only the nodes that bridge them — Decisions / Memories /
    //     Analyses / Sessions that touch two or more of the selected
    //     repos via SUPERSEDES / OBSERVED_IN / REMEMBERS / OF /
    //     LINKED_TO / IN_REPO edges. Single-repo nodes are dropped so
    //     the canvas reads as the literal cross-project link diagram.

    // Build a lowercase→canonical map for repo nodes so synthesized
    // edges (which target the lowercase slug) can resolve to whatever
    // casing Nexus actually stored on the node id.
    const repoIdByLower = new Map<string, string>();
    for (const n of data.nodes) {
      if (n.kind === "repo") repoIdByLower.set(n.id.toLowerCase(), n.id);
    }
    // Synthesize Repo nodes for any selected slug missing from the
    // payload (covers projects with zero indexed events but a known
    // ADR/analysis row). Without this, synthesized IN_REPO edges
    // would dangle.
    const synthRepoNodes: { id: string; label: string; kind: string }[] = [];
    const ensureRepoNode = (lowerSlug: string) => {
      if (repoIdByLower.has(lowerSlug)) return repoIdByLower.get(lowerSlug)!;
      repoIdByLower.set(lowerSlug, lowerSlug);
      synthRepoNodes.push({ id: lowerSlug, label: lowerSlug, kind: "repo" });
      return lowerSlug;
    };
    // Resolve each synthesized edge's target onto the canonical repo
    // node id (and add a Repo node if necessary).
    const resolvedSyntheticEdges: SynthEdge[] = syntheticEdges.map((e) => ({
      from: e.from,
      to: ensureRepoNode(e.to.toLowerCase()),
      label: e.label,
    }));
    // From here on the loader works against the union of payload
    // edges + synthesized edges. Same shape, same semantics.
    const allEdges: { from: string; to: string; label: string }[] = [
      ...data.edges,
      ...resolvedSyntheticEdges,
    ];
    const allNodes = [...data.nodes, ...synthRepoNodes];

    const inboundCount = new Map<string, number>();
    for (const e of allEdges) {
      inboundCount.set(e.to, (inboundCount.get(e.to) ?? 0) + 1);
      inboundCount.set(e.from, (inboundCount.get(e.from) ?? 0) + 1);
    }

    // Lowercase the selection set once — Nexus can return mixed-case
    // repo ids (e.g. "Cortex") while the multi-select stores
    // lowercase, and the user clicked a node whose id we want to
    // match either way.
    const selectedSet = new Set(selectedRepos.map((r) => r.toLowerCase()));

    // Walk the IN_REPO edges to learn which repo each artifact /
    // memory / decision / analysis / session belongs to. Used both
    // to filter for the correlation mode and to colour-tint nodes
    // by their owning project.
    const nodeRepos = new Map<string, Set<string>>();
    const noteRepo = (nodeId: string, repoId: string) => {
      const lower = repoId.toLowerCase();
      let s = nodeRepos.get(nodeId);
      if (!s) {
        s = new Set();
        nodeRepos.set(nodeId, s);
      }
      s.add(lower);
    };
    for (const e of allEdges) {
      if (e.label === "IN_REPO") noteRepo(e.from, e.to);
      if (e.label === "OBSERVED_IN") noteRepo(e.from, e.to);
      if (e.label === "OF") noteRepo(e.from, e.to);
    }
    // Sessions inherit their primary repo via the dashboard
    // /sessions aggregate (no IN_REPO edge on the writer side yet).
    for (const [sid, repo] of sessionPrimaryRepo) {
      noteRepo(sid, repo);
    }

    const PANORAMA_KINDS = new Set(["repo", "decision", "law", "analysis"]);
    const DRILLDOWN_KINDS = new Set([
      "repo",
      "session",
      "turn",
      "decision",
      "law",
      "violation",
      "memory",
      "analysis",
      "agent_call",
      "artifact",
      "tool_call",
    ]);
    const CORRELATION_KINDS = new Set([
      "repo",
      "decision",
      "law",
      "analysis",
      "memory",
      "session",
    ]);

    const survivingIds = new Set<string>();
    for (const n of allNodes) {
      if (mode === "panorama") {
        if (!PANORAMA_KINDS.has(n.kind)) continue;
        survivingIds.add(n.id);
        continue;
      }
      if (mode === "drilldown") {
        if (!DRILLDOWN_KINDS.has(n.kind)) continue;
        survivingIds.add(n.id);
        continue;
      }
      // correlation
      if (!CORRELATION_KINDS.has(n.kind)) continue;
      if (n.kind === "repo") {
        // Only keep the explicitly selected repo nodes — every other
        // repo in the payload is irrelevant to a 2-repo correlation.
        if (selectedSet.has(n.id.toLowerCase())) survivingIds.add(n.id);
        continue;
      }
      const repos = nodeRepos.get(n.id);
      if (!repos) continue;
      // Bridge condition: the node touches ≥2 of the selected repos.
      let hits = 0;
      for (const r of repos) if (selectedSet.has(r)) hits++;
      if (hits >= 2) survivingIds.add(n.id);
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
    for (const n of allNodes) {
      if (n.kind === "turn") turnLabelById.set(n.id, n.label);
    }
    for (const e of allEdges) {
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

    // Per-repo accent palette. Sessions / Turns inherit their
    // owning project's hue when known via `sessionPrimaryRepo`,
    // so the canvas reads as project-coloured clusters at first
    // glance instead of a sea of cyan turns. The palette stays
    // distinct from KIND_HEX so the legend's "Session" / "Turn"
    // dots still read true on session-tree spines whose repo is
    // unknown.
    const REPO_PALETTE: readonly string[] = [
      "#f7768e", // rose
      "#9ece6a", // green
      "#7aa2f7", // blue
      "#bb9af7", // purple
      "#e0af68", // amber
      "#73daca", // mint
      "#ff9e64", // orange
      "#7dcfff", // cyan
      "#c0caf5", // lavender
      "#f5a97f", // peach
      "#a6da95", // pistachio
      "#cba6f7", // mauve
    ];
    const knownRepos = Array.from(
      new Set([
        ...allNodes
          .filter((n) => n.kind === "repo")
          .map((n) => n.id.toLowerCase()),
        ...sessionPrimaryRepo.values(),
      ]),
    ).sort();
    const repoColor = new Map<string, string>();
    knownRepos.forEach((r, idx) => {
      repoColor.set(r, REPO_PALETTE[idx % REPO_PALETTE.length]);
    });

    // Resolve each Turn's owning Session (HAS_TURN edge `from`)
    // so Turns also pick up the parent Session's repo hue.
    const turnSession = new Map<string, string>();
    for (const e of allEdges) {
      if (e.label === "HAS_TURN") turnSession.set(e.to, e.from);
    }
    const colorFor = (id: string, kind: string): string => {
      if (kind === "repo")
        return repoColor.get(id.toLowerCase()) ?? KIND_HEX.repo;
      if (kind === "session") {
        const r = sessionPrimaryRepo.get(id);
        if (r && repoColor.has(r)) return repoColor.get(r)!;
      }
      if (kind === "turn") {
        const sid = turnSession.get(id);
        const r = sid ? sessionPrimaryRepo.get(sid) : undefined;
        if (r && repoColor.has(r)) return repoColor.get(r)!;
      }
      return KIND_HEX[kind] ?? "#9aa5ce";
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

    for (const n of allNodes) {
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
        color: colorFor(n.id, n.kind),
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
    for (const e of allEdges) {
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
  }, [
    data,
    loadGraph,
    sigma,
    mode,
    selectedRepos,
    sessionPrimaryRepo,
    syntheticEdges,
  ]);

  useEffect(() => {
    registerEvents({
      enterNode: ({ node }) => setHovered(node),
      leaveNode: () => setHovered(null),
      clickNode: ({ node }) => {
        const g = sigma.getGraph();
        if (!g.hasNode(node)) return;
        const kind = g.getNodeAttribute(node, "kind") ?? "unknown";
        // Repo nodes are drill-down anchors — clicking one narrows
        // the backend filter to that project (single-repo mode).
        // The next render will pull the full subtree (artifacts +
        // tool calls) instead of the structural skeleton.
        if (kind === "repo") {
          // Lowercase before bubbling — Nexus stores raw casing
          // ("Cortex") while the multi-select stores normalised
          // lowercase, and we want toggle membership to round-trip.
          onRepoClick(node.toLowerCase());
          return;
        }
        onSelect({
          id: node,
          label: g.getNodeAttribute(node, "label") ?? node,
          kind,
          neighbors: g.degree(node),
        });
      },
      clickStage: () => onSelect(null),
    });
  }, [registerEvents, sigma, onSelect, onRepoClick]);

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
  const { filters, setFilter } = useFilters();
  const [selected, setSelected] = useState<SelectedNode | null>(null);

  // Lowercase whatever is currently in `filters.repo` so the canvas
  // filter, the dropdown, and the click-toggle all share one identity
  // for each project regardless of the casing the originating source
  // (Nexus / overview / sessions / sidebar) used.
  const selectedRepos = useMemo(
    () => (filters.repo ?? []).map((r) => r.toLowerCase()),
    [filters.repo],
  );

  const loaderMode: LoaderMode =
    !filters.session_id && selectedRepos.length === 1
      ? "drilldown"
      : !filters.session_id && selectedRepos.length >= 2
        ? "correlation"
        : "panorama";
  const isRepoDrilldown = loaderMode === "drilldown";
  const isCorrelationView = loaderMode === "correlation";

  const connKey = useConnKey();
  const { data, isLoading, error } = useQuery({
    queryKey: [connKey, "graph",
      filters.session_id ?? "",
      selectedRepos.slice().sort().join("|"),
    ],
    queryFn: () => api.graph(filters.session_id, 30_000, selectedRepos),
    // The full panorama is multiple MB on the wire and a few seconds
    // of FA2 layout — refetching every 12 s would peg the CPU on a
    // laptop. 60 s is the closest cadence the dashboard actually
    // benefits from (live capture lands new turns at human speed).
    refetchInterval: 60_000,
    refetchIntervalInBackground: false,
  });

  // Sessions don't connect to Repos in the graph schema — there's
  // no `:USED_IN`/`:SESSION_REPO` edge on the writer side today.
  // The dashboard `/sessions` endpoint already aggregates per-
  // session repo touches; we pull it in parallel and join into a
  // `sessionId -> primaryRepo` map the GraphLoader uses to colour
  // each Session node by its dominant project. Refetches happen
  // far less often than the graph because the per-session repo
  // mapping is stable once a session ends.
  const { data: sessionsRows } = useQuery({
    queryKey: [connKey, "graph-sessions-for-repo-tint"],
    queryFn: () => api.sessions(),
    refetchInterval: 5 * 60_000,
    refetchIntervalInBackground: false,
  });

  // Overview gives us the canonical `recent_repos` list — used to
  // populate the project selector regardless of the current graph
  // filter (so a user who narrowed to one repo still sees every
  // other repo as a switchable option). Same surface Timeline /
  // Memory use.
  const { data: overview } = useQuery({
    queryKey: [connKey, "graph-overview-for-repo-options"],
    queryFn: () => api.overview(),
    refetchInterval: 5 * 60_000,
    refetchIntervalInBackground: false,
  });

  // Decisions and Analyses carry their owning repo as a row field
  // (the dashboard `/decisions` and `/analyses` endpoints surface
  // it). Nexus does NOT yet store a `Decision -[:IN_REPO]-> Repo`
  // edge — the writer only emits IN_REPO for Artifacts — so the
  // graph payload we receive has Decisions and Analyses dangling
  // with zero outbound edges in panorama mode (the symptom: a
  // ring of 481 disconnected green nodes).
  //
  // We pull the same rows the Decisions / Analyses tabs use and
  // synthesize the missing `IN_REPO` edges client-side at render
  // time. Side benefit: the synthesized edges work in correlation
  // mode too — a Decision tagged repo=cortex AND repo=rulebook
  // would already need separate sources, but with the per-row
  // `repo` field we at least get the single-repo membership the
  // backend never wrote into Nexus.
  const { data: decisionsRows } = useQuery({
    queryKey: [connKey, "graph-decisions-for-repo-edges"],
    queryFn: () => api.decisions(),
    refetchInterval: 5 * 60_000,
    refetchIntervalInBackground: false,
  });
  const { data: analysesRows } = useQuery({
    queryKey: [connKey, "graph-analyses-for-repo-edges"],
    queryFn: () => api.analyses(),
    refetchInterval: 5 * 60_000,
    refetchIntervalInBackground: false,
  });
  const synthesizedRepoEdges = useMemo(() => {
    const edges: { from: string; to: string; label: string }[] = [];
    for (const d of decisionsRows ?? []) {
      if (d.repo) edges.push({ from: d.id, to: d.repo.toLowerCase(), label: "IN_REPO" });
    }
    for (const a of analysesRows ?? []) {
      if (a.repo) edges.push({ from: a.id, to: a.repo.toLowerCase(), label: "IN_REPO" });
    }
    return edges;
  }, [decisionsRows, analysesRows]);
  // Repo slugs come in with inconsistent casing across sources
  // (Nexus stores whatever the writer received; overview /
  // sessions normalise differently). Backend `normalize_repo`
  // lowercases on the filter side, so any mixed-case slug we
  // store here would never match the canvas / dropdown twice.
  // We lowercase at every ingest point so the GUI's own
  // identity check (`Set`, `includes`, dropdown dedupe) lines up.
  const sessionPrimaryRepo = useMemo(() => {
    const m = new Map<string, string>();
    for (const row of sessionsRows ?? []) {
      if (row.repos && row.repos.length > 0) {
        m.set(row.session_id, row.repos[0].toLowerCase());
      }
    }
    return m;
  }, [sessionsRows]);

  // Union of every repo we know about so the picker is stable even
  // as the graph filter narrows. Pulls from three sources so we
  // don't depend on any single one being populated:
  //   - Overview's `recent_repos` (canonical, server-side aggregate).
  //   - Repo nodes the current graph payload returned (whatever the
  //     filter let through).
  //   - Sessions' aggregated repos (covers projects with sessions but
  //     no live activity in the recent overview window).
  //   - The currently selected `filters.repo` itself (so a stale slug
  //     still shows up in the dropdown for the user to clear).
  const repoOptions = useMemo(() => {
    const set = new Set<string>();
    for (const r of overview?.recent_repos ?? []) set.add(r.repo.toLowerCase());
    for (const n of data?.nodes ?? []) {
      if (n.kind === "repo") set.add(n.id.toLowerCase());
    }
    for (const row of sessionsRows ?? []) {
      for (const r of row.repos ?? []) set.add(r.toLowerCase());
    }
    for (const r of selectedRepos) set.add(r);
    return Array.from(set).sort((a, b) => a.localeCompare(b));
  }, [overview?.recent_repos, data?.nodes, sessionsRows, selectedRepos]);

  // Mirror the loader's mode-aware filter so the "shown" / "edges"
  // figures in the side panel match what the canvas actually drew.
  // Three rule sets:
  //   - panorama: just the institutional spine (repo/decision/law/analysis).
  //   - drilldown: the full subtree.
  //   - correlation: spine + memories + sessions, but only nodes that
  //     bridge ≥2 selected repos.
  const { renderedNodes, renderedEdges } = useMemo(() => {
    if (!data) return { renderedNodes: 0, renderedEdges: 0 };
    const PANORAMA = new Set(["repo", "decision", "law", "analysis"]);
    const DRILL = new Set([
      "repo",
      "session",
      "turn",
      "decision",
      "law",
      "violation",
      "memory",
      "analysis",
      "agent_call",
      "artifact",
      "tool_call",
    ]);
    const CORR = new Set([
      "repo",
      "decision",
      "law",
      "analysis",
      "memory",
      "session",
    ]);
    // Walk IN_REPO / OBSERVED_IN / OF to find what each node belongs
    // to — needed for correlation's "touches ≥2 selected repos" gate.
    // Same logic as the loader's; we mirror it here so the panel
    // counters match the canvas exactly.
    const repoIdByLower = new Map<string, string>();
    for (const n of data.nodes) {
      if (n.kind === "repo") repoIdByLower.set(n.id.toLowerCase(), n.id);
    }
    const synthRepoNodes: { id: string; label: string; kind: string }[] = [];
    const ensureRepoNode = (lower: string) => {
      if (repoIdByLower.has(lower)) return repoIdByLower.get(lower)!;
      repoIdByLower.set(lower, lower);
      synthRepoNodes.push({ id: lower, label: lower, kind: "repo" });
      return lower;
    };
    const allEdges = [
      ...data.edges,
      ...synthesizedRepoEdges.map((e) => ({
        from: e.from,
        to: ensureRepoNode(e.to.toLowerCase()),
        label: e.label,
      })),
    ];
    const allNodes = [...data.nodes, ...synthRepoNodes];

    const nodeRepos = new Map<string, Set<string>>();
    const note = (node: string, repo: string) => {
      const lower = repo.toLowerCase();
      let s = nodeRepos.get(node);
      if (!s) {
        s = new Set();
        nodeRepos.set(node, s);
      }
      s.add(lower);
    };
    for (const e of allEdges) {
      if (e.label === "IN_REPO") note(e.from, e.to);
      if (e.label === "OBSERVED_IN") note(e.from, e.to);
      if (e.label === "OF") note(e.from, e.to);
    }
    for (const row of sessionsRows ?? []) {
      if (row.repos?.[0]) note(row.session_id, row.repos[0]);
    }
    const sel = new Set(selectedRepos);
    const survivors = new Set<string>();
    for (const n of allNodes) {
      if (loaderMode === "panorama") {
        if (PANORAMA.has(n.kind)) survivors.add(n.id);
        continue;
      }
      if (loaderMode === "drilldown") {
        if (DRILL.has(n.kind)) survivors.add(n.id);
        continue;
      }
      if (!CORR.has(n.kind)) continue;
      if (n.kind === "repo") {
        if (sel.has(n.id.toLowerCase())) survivors.add(n.id);
        continue;
      }
      const repos = nodeRepos.get(n.id);
      if (!repos) continue;
      let hits = 0;
      for (const r of repos) if (sel.has(r)) hits++;
      if (hits >= 2) survivors.add(n.id);
    }
    let edges = 0;
    for (const e of allEdges) {
      if (survivors.has(e.from) && survivors.has(e.to)) edges++;
    }
    return { renderedNodes: survivors.size, renderedEdges: edges };
  }, [data, sessionsRows, loaderMode, selectedRepos, synthesizedRepoEdges]);
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
            Cypher MATCH against Nexus — pick a project to drill into a single
            repo's subtree, or select two or more to surface the cross-project
            correlations (SUPERSEDES / OBSERVED_IN / REMEMBERS bridges between
            them). Empty selection shows the full panorama. Clicking a Repo
            node in the canvas toggles its membership.
          </p>
        </div>
        <div className="view__actions">
          {repoOptions.length > 0 ? (
            <RepoMultiSelect
              label="Project"
              prominent
              options={repoOptions}
              selected={selectedRepos}
              onChange={(next) =>
                setFilter(
                  "repo",
                  next.length === 0
                    ? undefined
                    : next.map((r) => r.toLowerCase()),
                )
              }
            />
          ) : null}
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
            <GraphLoader
              data={data}
              onSelect={setSelected}
              onRepoClick={(repoId) => {
                // Toggle membership in the multi-select array.
                // Casing was already normalised on the way in (loader
                // lowercases before calling this) so includes() is a
                // direct match.
                if (selectedRepos.includes(repoId)) {
                  const next = selectedRepos.filter((r) => r !== repoId);
                  setFilter("repo", next.length === 0 ? undefined : next);
                } else {
                  setFilter("repo", [...selectedRepos, repoId]);
                }
              }}
              mode={loaderMode}
              selectedRepos={selectedRepos}
              sessionPrimaryRepo={sessionPrimaryRepo}
              syntheticEdges={synthesizedRepoEdges}
            />
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
              <dt>mode</dt>
              <dd className="mono">
                {filters.session_id
                  ? "session"
                  : isCorrelationView
                    ? `correlation (${(filters.repo ?? []).length} projects)`
                    : isRepoDrilldown
                      ? "drilldown (1 project)"
                      : "panorama"}
              </dd>
              <dt>filter</dt>
              <dd
                className="mono"
                style={{ wordBreak: "break-all", whiteSpace: "normal" }}
              >
                {filters.session_id
                  ? filters.session_id.slice(0, 12) + "..."
                  : filters.repo && filters.repo.length > 0
                    ? filters.repo.join(", ")
                    : "all projects"}
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
