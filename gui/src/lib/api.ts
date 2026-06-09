/**
 * Fetchers for the cortex-api dashboard endpoints (spec 16 §0 MVP).
 *
 * Returns typed results matching the Rust response shapes in
 * `crates/cortex-api/src/dashboard.rs`. Errors bubble as thrown
 * `ApiError` so TanStack Query can surface them through its
 * `error` channel.
 *
 * Phase3 §3 — the static `BASE_URL` was promoted to a module-level
 * "active connection" register so every fetcher routes through the
 * current `Connection` (selected via the multi-connection switcher).
 * `ConnectionsProvider` registers itself on mount via
 * `setActiveConnectionResolver`; tests can install their own fixed
 * resolver. The default resolver preserves the legacy single-backend
 * behaviour so call sites that import api.ts without a provider
 * still hit `http://127.0.0.1:17000` over `auth=none`.
 */

/** Compact view of a Connection — only the fields the fetcher layer
 * needs. `auth` mirrors `gui/src/lib/connections/types.ts` shape but
 * is duplicated here so api.ts stays free of a circular dep with
 * the connections module. */
export type ApiConnection = {
  id: string;
  baseUrl: string;
  auth:
    | { kind: "none" }
    | { kind: "bearer"; token: string }
    | { kind: "basic"; username: string; password: string };
};

function legacyDefaultBaseUrl(): string {
  // In dev the renderer hits Vite's `/v1/*` proxy → 127.0.0.1:17000.
  // In production (built Electron) we hit the daemon directly.
  // Port matches `.env` `CORTEX_API_PORT` so a single supervisor
  // booting cortex-api from env settings stays in sync with the GUI.
  if (typeof window !== "undefined" && window.location.protocol === "file:") {
    return "http://127.0.0.1:17000";
  }
  return "";
}

let activeConnectionResolver: () => ApiConnection = () => ({
  id: "local",
  baseUrl: legacyDefaultBaseUrl(),
  auth: { kind: "none" },
});

/** Phase3 §3 — `ConnectionsProvider` calls this with a getter that
 * returns the current active connection. The store updates the
 * underlying state on every reducer action; the resolver function
 * stays the same (closes over the latest store value), so fetchers
 * always see the freshest baseUrl + auth without re-binding. */
export function setActiveConnectionResolver(
  resolver: () => ApiConnection,
): void {
  activeConnectionResolver = resolver;
}

/** Read the current active connection. Used by the few callers that
 * need both the base URL AND the connection id (React Query keys
 * scope on the id). */
export function getActiveConnection(): ApiConnection {
  return activeConnectionResolver();
}

/** Read the current base URL. Pure side-effect-free — the legacy
 * call sites that referenced `BASE_URL` as a const can swap to
 * `getBaseUrl()` when they need an absolute URL (e.g. SSE). */
export function getBaseUrl(): string {
  return activeConnectionResolver().baseUrl;
}

export class ApiError extends Error {
  status: number;
  constructor(status: number, message: string) {
    super(message);
    this.name = "ApiError";
    this.status = status;
  }
}

function authHeader(conn: ApiConnection): Record<string, string> {
  if (conn.auth.kind === "bearer" && conn.auth.token) {
    return { authorization: `Bearer ${conn.auth.token}` };
  }
  if (conn.auth.kind === "basic" && conn.auth.username) {
    // basic is rarely used in cortex-api today but the wire shape is
    // ready when the dashboard moves behind a reverse proxy that
    // requires it.
    const raw = `${conn.auth.username}:${conn.auth.password}`;
    const encoded =
      typeof btoa === "function" ? btoa(raw) : Buffer.from(raw).toString("base64");
    return { authorization: `Basic ${encoded}` };
  }
  return {};
}

async function getJson<T>(path: string): Promise<T> {
  const conn = activeConnectionResolver();
  const headers: Record<string, string> = {
    accept: "application/json",
    ...authHeader(conn),
  };
  const resp = await fetch(`${conn.baseUrl}${path}`, { headers });
  if (!resp.ok) {
    throw new ApiError(resp.status, `${resp.status} ${resp.statusText}`);
  }
  return (await resp.json()) as T;
}

/// Phase6a — `/v1/query` request body shape, matching the daemon's
/// `QueryRequest`. Only the fields the GUI actually emits are
/// declared here; the daemon's spec-11 schema is the source of truth.
export type QueryRequestBody = {
  intent: string;
  query: string;
  scope?: { repo?: string; files?: string[]; topics?: string[]; since?: string };
  limit?: number;
  k?: number;
  include?: string[];
  budget_ms?: number;
};

/// Phase6a — POST `/v1/query` with the spec-6a scope-resolution
/// headers wired in. When the caller knows the active repo (from
/// the sidebar filter), it MUST pass `repo` so the daemon sees the
/// same value as `x-cortex-repo`. The body's `scope.repo` still
/// wins when set; this helper lets callers route through the
/// header path without restructuring the request body.
export async function postQuery<T = unknown>(
  body: QueryRequestBody,
  opts: { repo?: string } = {},
): Promise<T> {
  const conn = activeConnectionResolver();
  const headers: Record<string, string> = {
    accept: "application/json",
    "content-type": "application/json",
    ...authHeader(conn),
  };
  if (opts.repo && opts.repo.length > 0) {
    headers["x-cortex-repo"] = opts.repo;
  }
  const resp = await fetch(`${conn.baseUrl}/v1/query`, {
    method: "POST",
    headers,
    body: JSON.stringify(body),
  });
  if (!resp.ok) {
    // 422 carries the spec-6a `scope_repo_required` reason —
    // surface the structured body so callers can branch on it.
    let payload: unknown = null;
    try {
      payload = await resp.json();
    } catch {
      payload = null;
    }
    const reason =
      typeof payload === "object" && payload !== null && "reason" in payload
        ? (payload as { reason: unknown }).reason
        : null;
    throw new ApiError(
      resp.status,
      typeof reason === "string"
        ? `${resp.status} ${reason}`
        : `${resp.status} ${resp.statusText}`,
    );
  }
  return (await resp.json()) as T;
}

export type KindCount = { kind: string; count: number };
export type RepoCount = { repo: string; count: number };

/// Time-bucketed series block. `events_per_min` carries 20 minute
/// buckets; `violations_7d_daily` carries 7 day buckets — both have
/// no nulls so the Sparkline draws a gap-free line.
/// `pre_thinking_p95_ms` carries 20 minute buckets too but each slot
/// is `number | null`; the renderer leaves a gap on null buckets so
/// the user sees missing-data honestly rather than an implied zero.
/// `classifier_cost_usd_today` is a 24-slot hourly ribbon — all
/// zeros until spec-05 lands cost stamping (the parent body's
/// `classifier_cost_unavailable_until_spec05` flag confirms that).
export type SeriesBlock = {
  events_per_min: number[];
  pre_thinking_p95_ms: (number | null)[];
  violations_7d_daily: number[];
  classifier_cost_usd_today: number[];
};

export type Overview = {
  events_total: number;
  repos_indexed: number;
  kind_breakdown: KindCount[];
  recent_repos: RepoCount[];
  series: SeriesBlock;
  classifier_cost_unavailable_until_spec05: boolean;
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
  // Phase3 — sha256 fingerprint stamped by the spec-18 plugin on every
  // captured envelope. `sha256:<64hex>`. Dropped for redacted lane hits.
  content_hash?: string;
  // Phase3 — un-clipped tool-call body for the Inspector. Capped at
  // 8 KiB server-side; rows larger than that get `preview_truncated`.
  preview?: string;
  preview_truncated?: boolean;
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
  // Phase3 — full sha256 hash to filter the timeline by. The
  // Inspector wires this when the user clicks the `content_hash` row,
  // collapsing the view to every call sharing the same fingerprint
  // (the dedupe / replay-detection workflow).
  content_hash?: string;
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
  /** Owning repo. Present for phase4e bootstrap-imported audits and
   * for spec-15 envelopes that anchor to a project. */
  repo?: string;
  /** Repo-rooted markdown path for bootstrap-imported audits — lets
   * the GUI deep-link the source file. Absent for spec-15 envelopes. */
  source_path?: string;
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
/// shows an empty state in that case. `source` switches from
/// `"stub_until_spec14"` to `"derived"` when the real signal lands.
export type TrustMatrix = {
  models: string[];
  repos: string[];
  scores: Record<string, Record<string, number>>;
  source: "stub_until_spec14" | "derived";
};

export type DecisionDetail = DecisionRow & {
  body_markdown: string;
};

export type AnalysisDetail = AnalysisRow & {
  body_markdown: string;
};

export type ConsolidationRow = {
  id: string;
  consolidation_id?: string;
  title: string;
  /** `session` | `topic` | `decision_trace` | `""` for legacy. */
  grain: string;
  /** `shallow` | `deep` | `""` for legacy. */
  depth: string;
  model?: string;
  source_event_count: number;
  repo?: string;
  topics: string[];
  excerpt: string;
  occurred_at: string;
};

export type ConsolidationDetail = ConsolidationRow & {
  body_markdown: string;
};

/** Echo of the filter the dashboard applied to produce a view. */
export type ConsolidationFilter = {
  /** Normalised repo allow-list. Empty array = no repo filter. */
  repos: string[];
  /** `session` | `topic` | `decision_trace` | `undefined` when no grain filter. */
  grain?: string;
};

/**
 * Wire envelope returned by `/v1/dashboard/consolidations` (ADR-014
 * / phase13f §2.2). The GUI consumes `rows` verbatim; `total` mirrors
 * `rows.length`; `filter` is the server's echo of the filter that
 * produced the list.
 */
export type ConsolidationReportView = {
  rows: ConsolidationRow[];
  total: number;
  filter: ConsolidationFilter;
};

/** One per-backend entry in [`CoverageReportView.backends`]. */
export type BackendCoverageEntry = {
  /** `nexus` | `vectorizer` | `meili` | `archive`. */
  backend: string;
  missing: number;
  sample_event_ids: string[];
};

/**
 * Wire envelope returned by `/v1/dashboard/coverage` (ADR-014 /
 * phase13f §2.3). `metadata_db` is the empty string when no metadata
 * store is wired; non-empty when the scan ran.
 */
export type CoverageReportView = {
  metadata_db: string;
  rows_total: number;
  backends: BackendCoverageEntry[];
  is_healthy: boolean;
  latency_ms: number;
};

/** One row in [`ProducerCheckpointsReportView.rows`]. */
export type ProducerCheckpointView = {
  producer_name: string;
  scope: string;
  /** Last envelope id; `null` when the row carried an empty marker. */
  last_event_id: string | null;
  last_occurred_at: string;
  accumulated_at: string;
  has_progress: boolean;
};

/**
 * Wire envelope returned by `/v1/dashboard/producers` (ADR-014 /
 * phase13f §3.4). One row per `(producer_name, scope)` pair, sorted
 * by `producer_name` then `scope`.
 */
export type ProducerCheckpointsReportView = {
  rows: ProducerCheckpointView[];
  total: number;
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

/// One classified-event row surfaced on the Classifications view.
/// Mirrors `ClassificationRow` server-side.
export type ClassificationRow = {
  event_id: string;
  kind: string;
  repo: string | null;
  path: string | null;
  topics: string[];
  severity: string | null;
  pii_risk: string | null;
  summary: string;
  ts: number;
  at: string;
};

/// Aggregate counts the Classifications view renders alongside the rows.
export type ClassificationStats = {
  total: number;
  top_topics: { topic: string; count: number }[];
  by_severity: { kind: string; count: number }[];
  by_pii_risk: { kind: string; count: number }[];
  by_repo: { repo: string; count: number }[];
};

export type ClassificationsBody = {
  stats: ClassificationStats;
  rows: ClassificationRow[];
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
  if (filters.content_hash) params.set("content_hash", filters.content_hash);
}

// ── Phase18 §7.7 — bitemporal timeline / branch / entity-history wire
// types. Mirror the §4.4 HTTP route payloads (see docs/specs/33-timeline-api.md).
// Distinct from the session-stream `TimelineEvent` above — these scope to a
// project's bitemporal timeline. Nexus returns rows as JSON maps, so every
// field is optional to tolerate absent columns.

/// One row from `GET /v1/timeline/{project}` / `/v1/entity/{id}/history`.
export type BitemporalTimelineEvent = {
  id?: string;
  kind?: string;
  project_id?: string;
  branch_id?: string;
  entity_id?: string;
  valid_from_unix?: number;
  recorded_at_unix?: number;
  title?: string;
  summary?: string;
};

/// `GET /v1/timeline/{project}` response envelope.
export type BitemporalTimelineResponse = {
  project: string;
  branch: string;
  filters?: Record<string, unknown>;
  events: BitemporalTimelineEvent[];
};

/// Optional filters for `GET /v1/timeline/{project}`.
export type TimelineFilters = {
  branch?: string;
  kind?: string;
  from?: string;
  to?: string;
  as_of?: string;
  limit?: number;
};

/// One branch from `GET /v1/branch/{project}`.
export type BranchMetadata = {
  id?: string;
  name?: string;
  parent_branch_id?: string;
  status?: string;
  merge_strategy?: string;
  merge_point_event_id?: string;
  abandonment_reason?: string;
  fork_valid_time?: string;
  created_at?: string;
  created_by?: string;
};

/// `GET /v1/branch/{project}` response envelope.
export type BranchListResponse = {
  project: string;
  branches: BranchMetadata[];
};

/// `GET /v1/entity/{id}/history` response envelope.
export type EntityHistoryResponse = {
  entity_id: string;
  as_of_unix?: number | null;
  events: BitemporalTimelineEvent[];
};

/// One node in a supersession chain walk.
export type SupersessionChainNode = {
  id?: string;
  kind?: string;
};

/// `GET /v1/entity/{id}/supersession` lineage payload.
export type SupersessionLineage = {
  id?: string;
  label?: string;
  predecessors?: SupersessionChainNode[];
  successors?: SupersessionChainNode[];
};

/// `GET /v1/entity/{id}/supersession` response envelope.
export type SupersessionResponse = {
  entity_id: string;
  lineage?: SupersessionLineage;
};

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
  classifications: (filters?: {
    repo?: string;
    topic?: string;
    severity?: string;
    kind?: string;
    limit?: number;
  }) => {
    const params = new URLSearchParams();
    if (filters?.repo) params.set("repo", filters.repo);
    if (filters?.topic) params.set("topic", filters.topic);
    if (filters?.severity) params.set("severity", filters.severity);
    if (filters?.kind) params.set("kind", filters.kind);
    if (filters?.limit) params.set("limit", String(filters.limit));
    const q = params.toString();
    return getJson<ClassificationsBody>(
      `/v1/dashboard/classifications${q ? `?${q}` : ""}`,
    );
  },
  laws: () => getJson<LawRow[]>("/v1/dashboard/laws"),
  violations: () => getJson<ViolationRow[]>("/v1/dashboard/violations"),
  analyses: (repo?: string) => {
    const params = new URLSearchParams();
    if (repo) params.set("repo", repo);
    const q = params.toString();
    return getJson<AnalysisRow[]>(
      `/v1/dashboard/analyses${q ? `?${q}` : ""}`,
    );
  },
  toolsStats: () => getJson<ToolsStatsBody>("/v1/dashboard/tools/stats"),
  trust: () => getJson<TrustMatrix>("/v1/dashboard/trust"),
  decisionDetail: (id: string) =>
    getJson<DecisionDetail>(`/v1/dashboard/decisions/${encodeURIComponent(id)}`),
  analysisDetail: (id: string) =>
    getJson<AnalysisDetail>(`/v1/dashboard/analyses/${encodeURIComponent(id)}`),
  consolidations: (params?: { repo?: string; grain?: string }) => {
    const qs = new URLSearchParams();
    if (params?.repo) qs.set("repo", params.repo);
    if (params?.grain) qs.set("grain", params.grain);
    const q = qs.toString();
    return getJson<ConsolidationReportView>(
      `/v1/dashboard/consolidations${q ? `?${q}` : ""}`,
    );
  },
  consolidationDetail: (id: string) =>
    getJson<ConsolidationDetail>(
      `/v1/dashboard/consolidations/${encodeURIComponent(id)}`,
    ),
  coverage: () => getJson<CoverageReportView>("/v1/dashboard/coverage"),
  producers: () =>
    getJson<ProducerCheckpointsReportView>("/v1/dashboard/producers"),
  dashboardCanary: () => getJson<CanaryReport>("/v1/dashboard/canary"),
  /// Pull the graph panorama. `repos` narrows the canvas to the
  /// listed projects' artifacts (sessions / decisions / memories
  /// stay regardless so the cross-project knowledge spine remains
  /// visible). Each repo is sent as its own `repo=<name>` query
  /// parameter — same wire shape `applyFilters` uses for the other
  /// dashboard endpoints.
  graph: (sessionId?: string, limit = 60, repos?: string[]) => {
    const params = new URLSearchParams();
    if (sessionId) params.set("session_id", sessionId);
    params.set("limit", String(limit));
    if (repos && repos.length > 0) {
      for (const r of repos) params.append("repo", r);
    }
    return getJson<GraphPayload>(`/v1/dashboard/graph?${params.toString()}`);
  },
  status: () => getJson<StatusBody>("/v1/status"),
  // Phase8g — Health view clients. Each method returns the JSON
  // shape the matching /v1/health/* endpoint emits; types are
  // mirror-images of the Rust structs. The `freshness`,
  // `divergence`, and `versions` endpoints are mounted on the
  // dashboard router so they share the same BASE_URL.
  healthOverview: () => getJson<HealthOverview>("/v1/health"),
  healthFreshness: () => getJson<FreshnessRow[]>("/v1/health/freshness"),
  healthDivergence: () => getJson<DivergenceRow[]>("/v1/health/divergence"),
  healthVersions: () => getJson<VersionsReport>("/v1/health/versions"),
  healthConfig: () => getJson<ConfigAudit>("/v1/health/config"),
  // Phase11e — collection / index inventory diff. Renders under
  // the existing extras column in the Health view.
  healthCoverage: () => getJson<CoverageReport>("/v1/health/coverage"),
  // phase14a §4.2 — consolidator daemon health. Surfaces per-grain
  // last_run + last_status that the Consolidations view renders as
  // a daemon-status banner above the consolidation list.
  healthConsolidator: () =>
    getJson<ConsolidatorHealthReport>("/v1/health/consolidator"),
  // phase14f — pre-thinking quality. Reuses the
  // `/v1/health/pre-thinking` endpoint which carries breaker state
  // plus per-intent bundle-size quantiles + helpful-rate counters
  // (added in phase14f §4.1/§4.2).
  preThinkingQuality: () =>
    getJson<PreThinkingHealthReport>("/v1/health/pre-thinking"),

  // Phase9i — retention dashboard endpoints. `sweeps` is paginated
  // by `limit`; `state` is a compact snapshot. Both are cached via
  // TanStack Query at the call site.
  retentionSweeps: (limit = 50, since?: string) => {
    const params = new URLSearchParams();
    params.set("limit", String(limit));
    if (since) params.set("since", since);
    return getJson<RetentionSweepRow[]>(`/v1/retention/sweeps?${params.toString()}`);
  },
  retentionState: () => getJson<RetentionStateBody>("/v1/retention/state"),

  // phase5b — Tasks view fetchers. Mirror the backend contracts
  // shipped by phase5a's /v1/dashboard/tasks* endpoints.
  tasks: (params?: TaskListParams) => {
    const qs = new URLSearchParams();
    if (params?.status?.length) {
      for (const s of params.status) qs.append("status", s);
    }
    if (params?.phase?.length) {
      for (const p of params.phase) qs.append("phase", p);
    }
    if (params?.repo?.length) {
      for (const r of params.repo) qs.append("repo", r);
    }
    if (params?.include_archived !== undefined) {
      qs.set("include_archived", String(params.include_archived));
    }
    if (params?.limit !== undefined) qs.set("limit", String(params.limit));
    if (params?.offset !== undefined) qs.set("offset", String(params.offset));
    if (params?.sort) qs.set("sort", params.sort);
    if (params?.order) qs.set("order", params.order);
    const tail = qs.toString();
    return getJson<TaskListResponse>(
      `/v1/dashboard/tasks${tail ? `?${tail}` : ""}`,
    );
  },
  task: (id: string) =>
    getJson<TaskDetail>(`/v1/dashboard/tasks/${encodeURIComponent(id)}`),
  tasksSummary: () => getJson<TaskSummary>("/v1/dashboard/tasks/summary"),

  // ── Phase18 §7.7 — bitemporal timeline / branch / entity-history
  // fetchers over the §4.4 routes.
  bitemporalTimeline: (project: string, filters?: TimelineFilters) => {
    const qs = new URLSearchParams();
    if (filters?.branch) qs.set("branch", filters.branch);
    if (filters?.kind) qs.set("kind", filters.kind);
    if (filters?.from) qs.set("from", filters.from);
    if (filters?.to) qs.set("to", filters.to);
    if (filters?.as_of) qs.set("as_of", filters.as_of);
    if (filters?.limit !== undefined) qs.set("limit", String(filters.limit));
    const tail = qs.toString();
    return getJson<BitemporalTimelineResponse>(
      `/v1/timeline/${encodeURIComponent(project)}${tail ? `?${tail}` : ""}`,
    );
  },
  branches: (project: string) =>
    getJson<BranchListResponse>(`/v1/branch/${encodeURIComponent(project)}`),
  branchDetail: (project: string, branch: string) =>
    getJson<{ branch_id: string; branch?: BranchMetadata }>(
      `/v1/branch/${encodeURIComponent(project)}/${encodeURIComponent(branch)}`,
    ),
  entityHistory: (id: string, asOf?: string) => {
    const qs = new URLSearchParams();
    if (asOf) qs.set("as_of", asOf);
    const tail = qs.toString();
    return getJson<EntityHistoryResponse>(
      `/v1/entity/${encodeURIComponent(id)}/history${tail ? `?${tail}` : ""}`,
    );
  },
  entitySupersession: (id: string) =>
    getJson<SupersessionResponse>(
      `/v1/entity/${encodeURIComponent(id)}/supersession`,
    ),
};

// phase5b — Tasks view types (mirror cortex-api's tasks_loader).
export type TaskStatus =
  | "pending"
  | "in-progress"
  | "completed"
  | "blocked"
  | "archived";

export type TaskProgress = {
  done: number;
  total: number;
};

export type TaskRow = {
  id: string;
  title: string;
  phase: string;
  phase_num: number;
  phase_letter: string;
  status: TaskStatus;
  created_at: string;
  updated_at: string;
  archived_at?: string | null;
  progress: TaskProgress;
  summary: string;
  /// Phase5b multi-project — project slug the task came from
  /// (lowercase, derived from the `.rulebook/` parent directory).
  repo?: string | null;
};

export type PhaseBreakdown = {
  phase: string;
  phase_num: number;
  phase_letter: string;
  total: number;
  done: number;
  in_progress: number;
  pending: number;
};

export type TaskListResponse = {
  tasks: TaskRow[];
  total: number;
  by_phase: PhaseBreakdown[];
  by_status: Record<string, number>;
};

export type TaskChecklistItem = {
  text: string;
  done: boolean;
};

export type TaskChecklistSection = {
  section: string;
  items: TaskChecklistItem[];
};

export type TaskDetail = TaskRow & {
  proposal_md: string;
  checklist: TaskChecklistSection[];
};

export type TaskSummary = {
  total: number;
  completed: number;
  in_progress: number;
  pending: number;
  archived: number;
  completion_pct: number;
};

export type TaskListParams = {
  status?: string[];
  phase?: string[];
  /// Phase5b multi-project — restrict to these repo slugs.
  repo?: string[];
  include_archived?: boolean;
  limit?: number;
  offset?: number;
  sort?: string;
  order?: "asc" | "desc";
};

// Phase9i — retention dashboard types. Mirror-images of the Rust
// `RetentionSweepBody` / `RetentionStateBody` structs.
export type RetentionSweepRow = {
  sweep_id: string;
  started_at: string;
  finished_at: string | null;
  status: string;
  records_demoted: number;
  records_dropped: number;
  /// Per-stage counters parsed from `tier_transitions_json`. Keys
  /// are stage names; values carry the stage's own JSON shape.
  stages: Record<string, unknown>;
};

export type RetentionArchiveBuckets = {
  le_30d: number;
  "30d_to_365d": number;
  gt_365d: number;
  total: number;
  root: string;
  available: boolean;
};

export type RetentionCasTotals = {
  rows: number;
  bytes: number;
  available: boolean;
  path: string;
};

export type RetentionScheduledRun = {
  sweep: string;
  /// RFC-3339 timestamp of the next scheduled run, `"disabled"`
  /// when the cron row exists but `enabled = 0`, or `undefined` /
  /// `null` when the cron row is missing from the metadata store
  /// (Phase13a §4.3 — the handler no longer emits a missing-state
  /// string sentinel).
  next_run?: string | null;
  /// RFC-3339 timestamp of the most recent run. Absent when the
  /// sweep has never executed (phase11v §1.2).
  last_run?: string;
  /// `success` / `failed` / `lock_held`. Absent when the sweep has
  /// never executed (phase11v §1.2).
  last_status?: string;
  /// Consecutive failure count; omitted when zero (phase11v §1.2).
  failure_streak?: number;
};

export type RetentionStateBody = {
  collections: unknown[];
  archive_bytes: RetentionArchiveBuckets;
  meili_indexes: unknown[];
  cas: RetentionCasTotals;
  next_runs: RetentionScheduledRun[];
};

export type StatusBody = {
  service: string;
  version: string;
  pid: number;
  uptime_ms: number;
};

// ---- Phase8g health view types -------------------------------------

export type HealthState = "ok" | "degraded" | "down";

export type SubsystemStatus = {
  name: string;
  state: HealthState;
  latency_ms: number;
  last_error?: string;
  version?: string;
  since?: string;
  extras?: Record<string, unknown>;
};

export type HealthOverview = {
  overall: HealthState;
  subsystems: SubsystemStatus[];
  checked_at: string;
};

export type FreshnessSeverity = "ok" | "warn" | "critical";

export type FreshnessRow = {
  key: string;
  last_event_ts_ms: number;
  gap_seconds: number;
  severity: FreshnessSeverity;
};

export type DivergenceRow = {
  pair: string;
  upstream: number;
  downstream: number;
  delta: number;
  delta_growth: number;
  severity: FreshnessSeverity;
};

export type VersionRow = {
  name: string;
  git_sha: string;
  git_sha_short: string;
  build_ts: string;
  git_dirty: boolean;
  profile: string;
  crate_version: string;
  matches_head: boolean;
};

export type DriftRow = {
  binary: string;
  running_sha: string;
  expected_sha: string;
  behind_by_commits: number | null;
  note?: string;
};

export type VersionsReport = {
  head_sha: string;
  head_sha_short: string;
  running_binaries: VersionRow[];
  drift: DriftRow[];
  all_in_sync: boolean;
};

export type ConfigFinding = {
  severity: FreshnessSeverity;
  source: string;
  message: string;
};

export type ConfigAudit = {
  findings: ConfigFinding[];
  surfaces_read: number;
};

/// Phase11e — coverage audit response shape from
/// `/v1/health/coverage`. Mirrors `cortex_api::coverage::CoverageResponse`
/// + `BackendCoverage` exactly so an upstream rename / field add
/// blows up at compile time on the GUI side.
export type CoverageBackend = {
  backend: string;
  base_url: string | null;
  severity: "ok" | "warn" | "critical";
  expected_count: number;
  present_count: number;
  missing_count: number;
  unexpected_count: number;
  present: string[];
  missing: string[];
  unexpected: string[];
  error: string | null;
};

export type CoverageReport = {
  slugs: string[];
  families: string[];
  backends: CoverageBackend[];
  overall_severity: "ok" | "warn" | "critical";
};

/// Phase15f — one canary tick row from `GET /v1/dashboard/canary`.
export type CanaryRunView = {
  ts: string;
  status: "ok" | "error";
  latency_ms: number | null;
  error_message: string | null;
};

/// Phase15f — response body from `GET /v1/dashboard/canary`.
export type CanaryReport = {
  runs: CanaryRunView[];
  last_failure: CanaryRunView | null;
};

// phase14a §4.1 wire shape of `/v1/health/consolidator` —
// generated from Rust via ts-rs (phase14d). Re-exported so
// callers can keep `import { ConsolidatorHealthReport, ... }
// from "./api"`; `ConsolidatorHealthReport` is also imported
// locally because the `api.healthConsolidator()` helper above
// uses it as a `getJson<T>` type parameter.
import type {
  ConsolidatorHealthReport,
  PreThinkingHealthReport,
} from "./api.generated";
export type {
  ConsolidatorHealthReport,
  GrainHealth,
  IntentByteQuantilesView,
  IntentHelpfulRateView,
  IntentMismatchView,
  PreThinkingHealthReport,
} from "./api.generated";

export type HealthSnapshot = {
  generated_at: string;
  overall: HealthState;
  freshness: FreshnessRow[];
  divergence: DivergenceRow[];
  truncated: boolean;
};

/// Phase8g — `/v1/health/stream` SSE URL the GUI's `useSSE` hook
/// subscribes to. Same base URL the polling clients use so dev /
/// prod swaps behave identically.
///
/// Phase3 §3 — `EventSource` cannot carry custom headers, so when
/// the active connection has a bearer token we tack it on as a
/// `?api_key=…` query-param the daemon's `require_api_key`
/// middleware honours (mirrors the contract phase3 §7 spec ships).
/// Caller passes the URL straight to `new EventSource(...)`.
export function healthStreamUrl(): string {
  const conn = activeConnectionResolver();
  if (conn.auth.kind === "bearer" && conn.auth.token) {
    const sep = "?";
    return `${conn.baseUrl}/v1/health/stream${sep}api_key=${encodeURIComponent(conn.auth.token)}`;
  }
  return `${conn.baseUrl}/v1/health/stream`;
}

/// Spec 21 — `/v1/dashboard/stream` SSE endpoint URL with the same
/// `?api_key=…` escape hatch healthStreamUrl uses.
export function dashboardStreamUrl(): string {
  const conn = activeConnectionResolver();
  if (conn.auth.kind === "bearer" && conn.auth.token) {
    return `${conn.baseUrl}/v1/dashboard/stream?api_key=${encodeURIComponent(conn.auth.token)}`;
  }
  return `${conn.baseUrl}/v1/dashboard/stream`;
}
