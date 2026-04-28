/**
 * Fetchers for the cortex-api dashboard endpoints (spec 16 §0 MVP).
 *
 * Returns typed results matching the Rust response shapes in
 * `crates/cortex-api/src/dashboard.rs`. Errors bubble as thrown
 * `ApiError` so TanStack Query can surface them through its
 * `error` channel.
 */

const BASE_URL = (() => {
  // In dev the renderer hits Vite's `/v1/*` proxy → 127.0.0.1:15000.
  // In production (built Electron) we hit the daemon directly.
  // Port matches `.env` `CORTEX_API_PORT` so a single supervisor
  // booting cortex-api from env settings stays in sync with the GUI.
  if (typeof window !== "undefined" && window.location.protocol === "file:") {
    return "http://127.0.0.1:15000";
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

/// Time-bucketed series block. `events_per_min` carries 20 minute
/// buckets; `violations_7d_daily` carries 7 day buckets. Both are
/// always present — empty buckets are zero so the renderer can draw
/// a gap-free Sparkline.
export type SeriesBlock = {
  events_per_min: number[];
  violations_7d_daily: number[];
};

export type Overview = {
  events_total: number;
  repos_indexed: number;
  kind_breakdown: KindCount[];
  recent_repos: RepoCount[];
  series: SeriesBlock;
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

/// Active filter set. `repo` is a list because the user often
/// observes more than one repo at once (compare staging-vs-prod,
/// watch a refactor that spans `core` + `gui`, etc.). The wire
/// contract: each repo is sent as its own `repo=<name>` query
/// parameter so the server-side filter stays a simple "any-of"
/// match without a delimiter convention.
export type Filters = {
  session_id?: string;
  repo?: string[];
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

export type DecisionChainNode = {
  id: string;
  title: string;
  state: "current" | "old";
};

export type DecisionRow = {
  id: string;
  /// Repo this decision belongs to (project that owns the
  /// `.rulebook/decisions/*.md`). Multi-project Hive workspace —
  /// without this the dashboard can't tell whose ADR it is.
  repo?: string;
  title: string;
  status: string;
  author: string | null;
  source_analysis: string | null;
  rationale: string | null;
  tags: string[];
  cites: string[];
  supersedes: string | null;
  /// Optional supersession chain. Populated by the backend's
  /// phase2h work; until that ships, the field is absent and the
  /// renderer skips the chain element.
  chain?: DecisionChainNode[];
  /// Optional id of the decision that superseded this one (the
  /// reverse pointer of `supersedes`). Phase2h-gated like `chain`.
  superseded_by?: string;
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

export type HeatmapBlock = {
  tz: string;
  days: string[];
  /// `[7][24]` matrix of tool-call counts. Rows are weekdays
  /// (Mon..Sun), columns are hour-of-day in UTC.
  cells: number[][];
};

export type ToolsStatsBody = {
  tools: ToolStat[];
  heatmap: HeatmapBlock;
};

/// Trust matrix — model × repo cells with `[0, 1]` scores. Empty
/// arrays / map until spec-14 lands the actual computation; the GUI
/// shows an empty state in that case.
export type TrustMatrix = {
  models: string[];
  repos: string[];
  scores: Record<string, Record<string, number>>;
};

export type DecisionDetail = DecisionRow & {
  body_markdown: string;
};

/// Per-session summary the Conversations list view renders. Each row
/// is one chat thread the user can drill into via `api.conversation`.
export type ConversationSummary = {
  session_id: string;
  title: string;
  repos: string[];
  turn_count: number;
  started_at_ms: number;
  last_at_ms: number;
};

/// One paired turn in a conversation transcript — user prompt +
/// (optional) assistant reply. Surfaces both halves the
/// UserPromptSubmit and Stop hooks now capture.
export type ConversationTurn = {
  turn_id: string;
  user_message: string;
  assistant_message: string | null;
  started_at_ms: number;
  completed_at_ms: number | null;
};

export type ConversationDetail = {
  session_id: string;
  repos: string[];
  turns: ConversationTurn[];
};

/// Sonnet-generated structured summary of one chat session. The
/// analyzer endpoint backs this — Conversations view fetches it on
/// demand and renders it above the raw transcript.
export type SessionSummary = {
  summary: string;
  key_actions: string[];
  references: string[];
  topics: string[];
  repos: string[];
};

/// One hand-off snapshot (`.rulebook/handoff/*.md`). Used by the
/// per-project Handoffs view so a user resuming work can pull the
/// last hand-off without grepping every repo by hand.
export type HandoffRow = {
  repo: string | null;
  path: string | null;
  filename: string;
  excerpt: string;
  updated: string;
  updated_ms: number;
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

/// Render a `Filters` set onto a `URLSearchParams`. Multi-valued
/// fields (today: `repo`) get one `key=value` pair per entry so the
/// server reads them via `Vec<String>` axum extractors without
/// needing a custom delimiter.
function applyFilters(params: URLSearchParams, filters?: Filters) {
  if (!filters) return;
  if (filters.session_id) params.set("session_id", filters.session_id);
  if (filters.repo && filters.repo.length > 0) {
    for (const r of filters.repo) {
      params.append("repo", r);
    }
  }
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
  decisions: (repo?: string) => {
    const params = new URLSearchParams();
    if (repo) params.set("repo", repo);
    const q = params.toString();
    return getJson<DecisionRow[]>(
      `/v1/dashboard/decisions${q ? `?${q}` : ""}`,
    );
  },
  conversations: () =>
    getJson<ConversationSummary[]>("/v1/dashboard/conversations"),
  conversation: (sessionId: string) =>
    getJson<ConversationDetail>(
      `/v1/dashboard/conversations/${encodeURIComponent(sessionId)}`,
    ),
  conversationSummary: (sessionId: string) =>
    getJson<SessionSummary>(
      `/v1/dashboard/conversations/${encodeURIComponent(sessionId)}/summary`,
    ),
  handoffs: (repo?: string) => {
    const params = new URLSearchParams();
    if (repo) params.set("repo", repo);
    const q = params.toString();
    return getJson<HandoffRow[]>(
      `/v1/dashboard/handoffs${q ? `?${q}` : ""}`,
    );
  },
  laws: () => getJson<LawRow[]>("/v1/dashboard/laws"),
  violations: () => getJson<ViolationRow[]>("/v1/dashboard/violations"),
  analyses: () => getJson<AnalysisRow[]>("/v1/dashboard/analyses"),
  toolsStats: () => getJson<ToolsStatsBody>("/v1/dashboard/tools/stats"),
  trust: () => getJson<TrustMatrix>("/v1/dashboard/trust"),
  decisionDetail: (id: string) =>
    getJson<DecisionDetail>(`/v1/dashboard/decisions/${encodeURIComponent(id)}`),
  graph: (sessionId?: string, limit = 60) => {
    const params = new URLSearchParams();
    if (sessionId) params.set("session_id", sessionId);
    params.set("limit", String(limit));
    return getJson<GraphPayload>(`/v1/dashboard/graph?${params.toString()}`);
  },
  status: () => getJson<StatusBody>("/v1/status"),
};

export type StatusBody = {
  service: string;
  version: string;
  pid: number;
  uptime_ms: number;
};
