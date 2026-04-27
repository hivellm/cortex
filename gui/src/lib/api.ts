/**
 * Fetchers for the cortex-api dashboard endpoints (spec 16 §0 MVP).
 *
 * Returns typed results matching the Rust response shapes in
 * `crates/cortex-api/src/dashboard.rs`. Errors bubble as thrown
 * `ApiError` so TanStack Query can surface them through its
 * `error` channel.
 */

const BASE_URL = (() => {
  // In dev the renderer hits Vite's `/v1/*` proxy → 127.0.0.1:15011.
  // In production (built Electron) we hit the daemon directly.
  if (typeof window !== "undefined" && window.location.protocol === "file:") {
    return "http://127.0.0.1:15011";
  }
  return "";
})();

export class ApiError extends Error {
  status: number;
  constructor(status: number, message: string) {
    super(message);
    this.name = "ApiError";
    this.status = status;
  }
}

async function getJson<T>(path: string): Promise<T> {
  const resp = await fetch(`${BASE_URL}${path}`, {
    headers: { accept: "application/json" },
  });
  if (!resp.ok) {
    throw new ApiError(resp.status, `${resp.status} ${resp.statusText}`);
  }
  return (await resp.json()) as T;
}

export type KindCount = { kind: string; count: number };
export type RepoCount = { repo: string; count: number };
export type Overview = {
  events_total: number;
  repos_indexed: number;
  kind_breakdown: KindCount[];
  recent_repos: RepoCount[];
};

export type TimelineEvent = {
  id: string;
  t: string;
  kind: string;
  title: string;
  detail: string;
  repo: string | null;
  session_id?: string;
  model: string;
};

export type SessionRow = {
  session_id: string;
  event_count: number;
  kind_breakdown: KindCount[];
  started_at_ms: number;
  last_event_ms: number;
  duration_ms: number;
  repos: string[];
  title: string;
};

export type Filters = {
  session_id?: string;
  repo?: string;
  kind?: string;
};

export type MemoryEntry = {
  title: string;
  excerpt: string;
  kind: string;
  repo: string | null;
  topics: string[];
  updated: string;
};

export type DecisionRow = {
  id: string;
  title: string;
  status: string;
  author: string | null;
  source_analysis: string | null;
  rationale: string | null;
  tags: string[];
  cites: string[];
  supersedes: string | null;
  occurred_at: string;
};

export type LawRow = {
  id: string;
  title: string;
  severity: string;
  blocked: boolean;
  scope: string;
  applies: number;
  violations_7d: number;
  rate: number;
  detector: string;
  remediation: string;
};

export type ViolationRow = {
  id: string;
  law_id: string | null;
  at: string;
  repo: string | null;
  action: string;
  evidence: string;
  remediation: string | null;
};

export type AnalysisRow = {
  id: string;
  title: string;
  status: string;
  panel: string[];
  judge: string;
  rounds: number;
  duration_s: number;
  verdict: string;
  decision_id: string | null;
  occurred_at: string;
};

export type ToolStat = {
  tool: string;
  calls: number;
  avg_ms: number;
  err_rate: number;
  share: number;
};

export type GraphNode = {
  id: string;
  label: string;
  x: number;
  y: number;
  kind: string;
};

export type GraphEdge = { from: string; to: string; label: string };
export type GraphPayload = { nodes: GraphNode[]; edges: GraphEdge[] };

function applyFilters(params: URLSearchParams, filters?: Filters) {
  if (!filters) return;
  if (filters.session_id) params.set("session_id", filters.session_id);
  if (filters.repo) params.set("repo", filters.repo);
  if (filters.kind) params.set("kind", filters.kind);
}

export const api = {
  overview: () => getJson<Overview>("/v1/dashboard/overview"),
  timelineRecent: (limit = 200, filters?: Filters) => {
    const params = new URLSearchParams();
    params.set("limit", String(limit));
    applyFilters(params, filters);
    return getJson<TimelineEvent[]>(`/v1/dashboard/timeline/recent?${params.toString()}`);
  },
  memory: (q: string, limit = 80, filters?: Filters) => {
    const params = new URLSearchParams();
    if (q) params.set("q", q);
    params.set("limit", String(limit));
    applyFilters(params, filters);
    return getJson<MemoryEntry[]>(`/v1/dashboard/memory?${params.toString()}`);
  },
  sessions: () => getJson<SessionRow[]>("/v1/dashboard/sessions"),
  decisions: () => getJson<DecisionRow[]>("/v1/dashboard/decisions"),
  laws: () => getJson<LawRow[]>("/v1/dashboard/laws"),
  violations: () => getJson<ViolationRow[]>("/v1/dashboard/violations"),
  analyses: () => getJson<AnalysisRow[]>("/v1/dashboard/analyses"),
  toolsStats: () => getJson<ToolStat[]>("/v1/dashboard/tools/stats"),
  graph: () => getJson<GraphPayload>("/v1/dashboard/graph"),
  status: () => getJson<StatusBody>("/v1/status"),
};

export type StatusBody = {
  service: string;
  version: string;
  pid: number;
  uptime_ms: number;
};
