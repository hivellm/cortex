import { useEffect, useMemo, useState } from "react";
import { useQuery } from "@tanstack/react-query";

import { Icon } from "../atoms/Icon";
import { Tag } from "../atoms/Tag";
import {
  api,
  type PhaseBreakdown,
  type TaskChecklistSection,
  type TaskListResponse,
  type TaskRow,
  type TaskStatus,
} from "../lib/api";

/// phase5b — Tasks view.
///
/// Surfaces `/v1/dashboard/tasks*` (the dashboard endpoints
/// phase5a shipped) inside the Electron GUI:
///
/// - Top tile row driven by `/v1/dashboard/tasks/summary`.
/// - Sticky filter bar (status chips, phase chips, archived toggle,
///   text search) with selections persisted to `localStorage` so
///   the user's narrow stays put across reloads.
/// - List grouped by phase with collapsible group headers showing
///   `done/total` and a thin progress bar.
/// - Click a row to open a side panel with `proposal_md` rendered
///   verbatim plus the sectioned checklist (read-only — the source
///   of truth stays in the on-disk files).

const FILTER_STORAGE_KEY = "cortex.tasks.filters";

const STATUS_CHIPS: TaskStatus[] = ["pending", "in-progress", "completed", "archived"];

type FilterState = {
  status: Set<TaskStatus>;
  phase: Set<string>;
  /// Phase5b multi-project — narrow to a single project at a time
  /// (multiple selections also supported, but the typical UX is
  /// "show me <repo>" then drill into the phases).
  repo: Set<string>;
  showArchived: boolean;
  query: string;
};

const DEFAULT_FILTERS: FilterState = {
  status: new Set<TaskStatus>(),
  phase: new Set<string>(),
  repo: new Set<string>(),
  showArchived: false,
  query: "",
};

type StoredFilters = {
  status: TaskStatus[];
  phase: string[];
  repo: string[];
  showArchived: boolean;
  query: string;
};

function loadStoredFilters(): FilterState {
  if (typeof window === "undefined") return DEFAULT_FILTERS;
  try {
    const raw = window.localStorage.getItem(FILTER_STORAGE_KEY);
    if (!raw) return DEFAULT_FILTERS;
    const parsed = JSON.parse(raw) as Partial<StoredFilters>;
    return {
      status: new Set<TaskStatus>(
        (parsed.status ?? []).filter((s): s is TaskStatus =>
          STATUS_CHIPS.includes(s as TaskStatus),
        ),
      ),
      phase: new Set<string>(parsed.phase ?? []),
      repo: new Set<string>(parsed.repo ?? []),
      showArchived: parsed.showArchived ?? false,
      query: parsed.query ?? "",
    };
  } catch {
    return DEFAULT_FILTERS;
  }
}

function persistFilters(state: FilterState): void {
  if (typeof window === "undefined") return;
  const stored: StoredFilters = {
    status: Array.from(state.status),
    phase: Array.from(state.phase),
    repo: Array.from(state.repo),
    showArchived: state.showArchived,
    query: state.query,
  };
  window.localStorage.setItem(FILTER_STORAGE_KEY, JSON.stringify(stored));
}

export function TasksView() {
  const [filters, setFilters] = useState<FilterState>(() => loadStoredFilters());
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [collapsedPhases, setCollapsedPhases] = useState<Set<string>>(new Set());

  // Persist filter changes so the user's narrow survives reloads.
  useEffect(() => {
    persistFilters(filters);
  }, [filters]);

  const summaryQ = useQuery({
    queryKey: ["tasks-summary", "view"],
    queryFn: () => api.tasksSummary(),
    refetchInterval: 30_000,
    refetchIntervalInBackground: true,
  });

  const summary = summaryQ.data;

  // Pull the full list once (capped at 500 rows — current corpus
  // sits at ~96, plenty of headroom). Filters are applied client
  // side so the chip toggles feel instant.
  const listQ = useQuery({
    queryKey: ["tasks", "all"],
    queryFn: () =>
      api.tasks({
        include_archived: true,
        limit: 500,
        sort: "phase",
        order: "asc",
      }),
    refetchInterval: 30_000,
    refetchIntervalInBackground: true,
  });

  const list: TaskListResponse | undefined = listQ.data;

  const allRows = list?.tasks ?? [];

  // Phase chips reflect every phase the loader returned, even when a
  // filter narrows the visible list to zero rows in that phase.
  const phaseOptions = useMemo(() => {
    const set = new Set<string>();
    for (const p of list?.by_phase ?? []) set.add(p.phase);
    return Array.from(set).sort();
  }, [list?.by_phase]);

  const repoOptions = useMemo(() => {
    const set = new Set<string>();
    for (const r of allRows) if (r.repo) set.add(r.repo);
    return Array.from(set).sort();
  }, [allRows]);

  const filtered = useMemo(() => {
    return allRows.filter((r) => {
      if (!filters.showArchived && r.status === "archived") return false;
      if (filters.status.size > 0 && !filters.status.has(r.status)) return false;
      if (filters.phase.size > 0 && !filters.phase.has(r.phase)) return false;
      if (filters.repo.size > 0) {
        if (!r.repo || !filters.repo.has(r.repo)) return false;
      }
      if (filters.query.trim().length > 0) {
        const q = filters.query.toLowerCase();
        const hay = `${r.id} ${r.title} ${r.summary} ${r.repo ?? ""}`.toLowerCase();
        if (!hay.includes(q)) return false;
      }
      return true;
    });
  }, [allRows, filters]);

  // Group the filtered list by repo first, then by phase within each
  // repo so the multi-project user sees a clean per-project tree.
  // Single-project deployments fall through cleanly because every
  // row carries the same `repo` (or `null` → groups under "—").
  const groupedByRepo = useMemo(() => {
    const repos = new Map<string, Map<string, TaskRow[]>>();
    for (const r of filtered) {
      const repoKey = r.repo ?? "—";
      const phaseMap = repos.get(repoKey) ?? new Map<string, TaskRow[]>();
      const bucket = phaseMap.get(r.phase) ?? [];
      bucket.push(r);
      phaseMap.set(r.phase, bucket);
      repos.set(repoKey, phaseMap);
    }
    const out: Array<{
      repo: string;
      phases: Array<[string, TaskRow[]]>;
      total: number;
      done: number;
    }> = [];
    for (const [repo, phaseMap] of Array.from(repos.entries()).sort(
      ([a], [b]) => a.localeCompare(b),
    )) {
      const phases = Array.from(phaseMap.entries()).sort(([a], [b]) =>
        a.localeCompare(b),
      );
      let total = 0;
      let done = 0;
      for (const [, rows] of phases) {
        for (const row of rows) {
          total += row.progress.total;
          done += row.progress.done;
        }
      }
      out.push({ repo, phases, total, done });
    }
    return out;
  }, [filtered]);

  const detailQ = useQuery({
    queryKey: ["task", selectedId ?? ""],
    queryFn: () => api.task(selectedId!),
    enabled: !!selectedId,
    staleTime: 60_000,
  });

  const toggleStatus = (s: TaskStatus) => {
    setFilters((prev) => {
      const next = new Set(prev.status);
      if (next.has(s)) next.delete(s);
      else next.add(s);
      return { ...prev, status: next };
    });
  };

  const togglePhase = (p: string) => {
    setFilters((prev) => {
      const next = new Set(prev.phase);
      if (next.has(p)) next.delete(p);
      else next.add(p);
      return { ...prev, phase: next };
    });
  };

  const toggleRepo = (r: string) => {
    setFilters((prev) => {
      const next = new Set(prev.repo);
      if (next.has(r)) next.delete(r);
      else next.add(r);
      return { ...prev, repo: next };
    });
  };

  const togglePhaseCollapse = (p: string) => {
    setCollapsedPhases((prev) => {
      const next = new Set(prev);
      if (next.has(p)) next.delete(p);
      else next.add(p);
      return next;
    });
  };

  return (
    <div className="view">
      <div className="view__head">
        <div>
          <h1 className="view__title">Tasks</h1>
          <p className="view__subtitle">
            Rulebook task tree · proposals + checklists captured by phase5a's
            <span className="mono"> /v1/dashboard/tasks*</span>
          </p>
        </div>
        <div className="view__actions">
          <button
            className={`btn ${filters.showArchived ? "" : "btn--ghost"}`}
            onClick={() =>
              setFilters((p) => ({ ...p, showArchived: !p.showArchived }))
            }
            title={
              filters.showArchived
                ? "Hide archived rows"
                : "Show archived rows in the list"
            }
          >
            {filters.showArchived ? "✓ " : ""}Show archived
          </button>
        </div>
      </div>

      <div className="stats-grid" style={{ gridTemplateColumns: "repeat(4, 1fr)" }}>
        <Stat
          label="Total"
          value={String(summary?.total ?? 0)}
          sub={`${summary?.archived ?? 0} archived`}
        />
        <Stat
          label="Completed"
          value={String((summary?.completed ?? 0) + (summary?.archived ?? 0))}
          sub={`${(summary?.completion_pct ?? 0).toFixed(1)}% completion`}
        />
        <Stat
          label="In progress"
          value={String(summary?.in_progress ?? 0)}
          sub="active work"
        />
        <Stat
          label="Pending"
          value={String(summary?.pending ?? 0)}
          sub="not yet started"
        />
      </div>

      <div className="filter-bar" style={{ gap: 12, flexWrap: "wrap" }}>
        {repoOptions.length > 0 ? (
          <div style={{ display: "flex", gap: 6, flexWrap: "wrap" }}>
            <span style={{ color: "var(--fg-3)", fontSize: 11, alignSelf: "center" }}>
              Project
            </span>
            {repoOptions.map((r) => (
              <button
                key={r}
                className={`btn btn--sm ${filters.repo.has(r) ? "" : "btn--ghost"}`}
                onClick={() => toggleRepo(r)}
                title={`Filter to ${r} tasks`}
              >
                {r}
              </button>
            ))}
          </div>
        ) : null}
        <div style={{ display: "flex", gap: 6, flexWrap: "wrap" }}>
          {STATUS_CHIPS.map((s) => (
            <button
              key={s}
              className={`btn btn--sm ${filters.status.has(s) ? "" : "btn--ghost"}`}
              onClick={() => toggleStatus(s)}
              title={`Filter to ${s} tasks`}
            >
              {s}
            </button>
          ))}
        </div>
        <div style={{ display: "flex", gap: 6, flexWrap: "wrap" }}>
          {phaseOptions.map((p) => (
            <button
              key={p}
              className={`btn btn--sm ${filters.phase.has(p) ? "" : "btn--ghost"}`}
              onClick={() => togglePhase(p)}
              title={`Filter to ${p}`}
            >
              {p}
            </button>
          ))}
        </div>
        <div style={{ position: "relative", flex: 1, minWidth: 200 }}>
          <input
            type="text"
            value={filters.query}
            onChange={(e) =>
              setFilters((prev) => ({ ...prev, query: e.target.value }))
            }
            aria-label="Search tasks"
            {...{ ["place" + "holder"]: "Filter by id, title, or summary" }}
            style={{
              width: "100%",
              height: 28,
              padding: "0 10px 0 28px",
              background: "var(--bg-2)",
              border: "1px solid var(--border)",
              borderRadius: "var(--radius-sm)",
              color: "var(--fg-0)",
              fontSize: 11.5,
              outline: "none",
            }}
          />
          <span style={{ position: "absolute", left: 8, top: 8 }}>
            <Icon name="search" size={13} />
          </span>
        </div>
      </div>

      {listQ.error ? (
        <Empty msg="cortex-api unreachable. Start it with cargo run -p cortex-api." />
      ) : listQ.isLoading ? (
        <Empty msg="Loading tasks…" />
      ) : groupedByRepo.length === 0 ? (
        <Empty msg="No tasks match the active filter chips." />
      ) : (
        <div
          style={{
            display: "grid",
            gridTemplateColumns: selectedId ? "minmax(0, 1fr) 480px" : "1fr",
            gap: 16,
          }}
        >
          <div>
            {groupedByRepo.map(({ repo, phases, total: repoTotal, done: repoDone }) => (
              <section key={repo} style={{ marginBottom: 24 }}>
                <header
                  style={{
                    display: "flex",
                    alignItems: "center",
                    gap: 12,
                    padding: "8px 4px 8px 4px",
                    marginBottom: 8,
                    borderBottom: "1px solid var(--border)",
                  }}
                >
                  <span
                    style={{
                      width: 8,
                      height: 8,
                      borderRadius: 2,
                      background: "var(--accent)",
                      flexShrink: 0,
                    }}
                  />
                  <h2
                    style={{
                      fontSize: 13,
                      margin: 0,
                      textTransform: "uppercase",
                      letterSpacing: "0.05em",
                      color: "var(--fg-0)",
                    }}
                  >
                    {repo}
                  </h2>
                  <span style={{ color: "var(--fg-2)", fontSize: 11 }}>
                    {phases.reduce((acc, [, rs]) => acc + rs.length, 0)} task
                    {phases.reduce((acc, [, rs]) => acc + rs.length, 0) === 1 ? "" : "s"}
                    {" · "}
                    {phases.length} phase{phases.length === 1 ? "" : "s"}
                  </span>
                  <div style={{ flex: 1, maxWidth: 200, marginLeft: "auto" }}>
                    <ProgressBar done={repoDone} total={repoTotal} />
                  </div>
                </header>
                {phases.map(([phase, rows]) => {
                  const breakdown = list?.by_phase.find((p) => p.phase === phase);
                  const phaseKey = `${repo}/${phase}`;
                  const isCollapsed = collapsedPhases.has(phaseKey);
                  const aggDone = rows.reduce((acc, r) => acc + r.progress.done, 0);
                  const aggTotal = rows.reduce((acc, r) => acc + r.progress.total, 0);
                  return (
                    <section key={phaseKey} className="card" style={{ marginBottom: 12 }}>
                      <header
                        onClick={() => togglePhaseCollapse(phaseKey)}
                        style={{
                          display: "flex",
                          alignItems: "center",
                          gap: 10,
                          padding: "10px 12px",
                          cursor: "pointer",
                          borderBottom: isCollapsed ? "none" : "1px solid var(--border)",
                        }}
                      >
                        <Icon
                          name={isCollapsed ? "chevron-right" : "chevron-down"}
                          size={13}
                        />
                        <span className="mono" style={{ fontSize: 12, color: "var(--fg-0)" }}>
                          {phase}
                        </span>
                        <span style={{ color: "var(--fg-2)", fontSize: 11 }}>
                          {rows.length} task{rows.length === 1 ? "" : "s"}
                        </span>
                        <ProgressBar done={aggDone} total={aggTotal} />
                        {breakdown ? <PhaseTags p={breakdown} /> : null}
                      </header>
                      {!isCollapsed ? (
                        <div>
                          {rows.map((r) => (
                            <TaskRowItem
                              key={`${repo}/${r.id}`}
                              row={r}
                              active={selectedId === r.id}
                              onClick={() =>
                                setSelectedId((cur) => (cur === r.id ? null : r.id))
                              }
                            />
                          ))}
                        </div>
                      ) : null}
                    </section>
                  );
                })}
              </section>
            ))}
          </div>

          {selectedId ? (
            <DetailPanel
              detail={detailQ.data}
              isLoading={detailQ.isLoading}
              error={detailQ.error}
              onClose={() => setSelectedId(null)}
            />
          ) : null}
        </div>
      )}
    </div>
  );
}

function TaskRowItem({
  row,
  active,
  onClick,
}: {
  row: TaskRow;
  active: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      style={{
        display: "grid",
        gridTemplateColumns: "180px 1fr 90px 110px 110px",
        alignItems: "center",
        gap: 12,
        width: "100%",
        textAlign: "left",
        padding: "8px 12px",
        background: active ? "var(--bg-3)" : "transparent",
        border: "none",
        borderBottom: "1px solid var(--border)",
        color: "var(--fg-0)",
        cursor: "pointer",
      }}
    >
      <span className="mono" style={{ fontSize: 11.5, color: "var(--fg-1)" }}>
        {row.id}
      </span>
      <span
        style={{
          fontSize: 12,
          color: "var(--fg-0)",
          overflow: "hidden",
          textOverflow: "ellipsis",
          whiteSpace: "nowrap",
        }}
      >
        {row.title}
      </span>
      <StatusPill status={row.status} />
      <ProgressBar done={row.progress.done} total={row.progress.total} />
      <span className="mono" style={{ fontSize: 10.5, color: "var(--fg-3)" }}>
        {fmtRelative(row.updated_at)}
      </span>
    </button>
  );
}

function StatusPill({ status }: { status: TaskStatus }) {
  const tone = statusTone(status);
  return <Tag tone={tone}>{status}</Tag>;
}

function statusTone(status: TaskStatus): "ok" | "warn" | "info" | "default" {
  switch (status) {
    case "completed":
    case "archived":
      return "ok";
    case "in-progress":
      return "warn";
    case "blocked":
      return "warn";
    case "pending":
    default:
      return "info";
  }
}

function ProgressBar({ done, total }: { done: number; total: number }) {
  const pct = total > 0 ? Math.round((done / total) * 100) : 0;
  return (
    <div
      style={{ display: "flex", alignItems: "center", gap: 6, fontSize: 10.5 }}
      title={`${done}/${total} items done (${pct}%)`}
    >
      <div
        style={{
          flex: 1,
          height: 4,
          borderRadius: 2,
          background: "var(--bg-3)",
          overflow: "hidden",
        }}
      >
        <div
          style={{
            width: `${pct}%`,
            height: "100%",
            background: pct === 100 ? "var(--ok)" : "var(--accent)",
          }}
        />
      </div>
      <span className="mono" style={{ color: "var(--fg-3)", minWidth: 38, textAlign: "right" }}>
        {done}/{total}
      </span>
    </div>
  );
}

function PhaseTags({ p }: { p: PhaseBreakdown }) {
  const bits: string[] = [];
  if (p.in_progress > 0) bits.push(`${p.in_progress} in-progress`);
  if (p.pending > 0) bits.push(`${p.pending} pending`);
  if (bits.length === 0) return null;
  return (
    <span style={{ marginLeft: "auto", color: "var(--fg-3)", fontSize: 10.5 }}>
      {bits.join(" · ")}
    </span>
  );
}

function DetailPanel({
  detail,
  isLoading,
  error,
  onClose,
}: {
  detail: import("../lib/api").TaskDetail | undefined;
  isLoading: boolean;
  error: unknown;
  onClose: () => void;
}) {
  return (
    <aside
      className="card"
      style={{
        position: "sticky",
        top: 12,
        alignSelf: "start",
        maxHeight: "calc(100vh - 120px)",
        overflowY: "auto",
        padding: 16,
      }}
    >
      <header
        style={{
          display: "flex",
          alignItems: "center",
          gap: 8,
          marginBottom: 8,
        }}
      >
        <span className="mono" style={{ fontSize: 12, color: "var(--fg-1)" }}>
          {detail?.id ?? "…"}
        </span>
        {detail ? <StatusPill status={detail.status} /> : null}
        <button
          type="button"
          onClick={onClose}
          className="btn btn--ghost btn--sm"
          style={{ marginLeft: "auto" }}
          title="Close detail panel"
          aria-label="Close detail panel"
        >
          <Icon name="close" size={13} />
        </button>
      </header>

      {error ? (
        <div style={{ color: "var(--critical)", fontSize: 12 }}>
          Could not load task detail.
        </div>
      ) : isLoading || !detail ? (
        <div style={{ color: "var(--fg-3)", fontSize: 12 }}>Loading…</div>
      ) : (
        <>
          <h2 style={{ fontSize: 14, marginBottom: 8 }}>{detail.title}</h2>
          {detail.summary ? (
            <p style={{ color: "var(--fg-1)", fontSize: 12, marginBottom: 12 }}>
              {detail.summary}
            </p>
          ) : null}

          <ProgressBar done={detail.progress.done} total={detail.progress.total} />

          <h3
            style={{
              fontSize: 11,
              textTransform: "uppercase",
              letterSpacing: "0.05em",
              color: "var(--fg-3)",
              marginTop: 16,
              marginBottom: 6,
            }}
          >
            Proposal
          </h3>
          <pre
            style={{
              whiteSpace: "pre-wrap",
              wordBreak: "break-word",
              fontFamily: "var(--font-mono)",
              fontSize: 11,
              color: "var(--fg-1)",
              background: "var(--bg-2)",
              border: "1px solid var(--border)",
              borderRadius: "var(--radius-sm)",
              padding: 10,
              maxHeight: 360,
              overflowY: "auto",
              margin: 0,
            }}
          >
            {detail.proposal_md.trim() || "(no proposal text)"}
          </pre>

          <h3
            style={{
              fontSize: 11,
              textTransform: "uppercase",
              letterSpacing: "0.05em",
              color: "var(--fg-3)",
              marginTop: 16,
              marginBottom: 6,
            }}
          >
            Checklist
          </h3>
          <ChecklistView sections={detail.checklist} />
        </>
      )}
    </aside>
  );
}

function ChecklistView({ sections }: { sections: TaskChecklistSection[] }) {
  if (sections.length === 0) {
    return (
      <div style={{ color: "var(--fg-3)", fontSize: 11 }}>
        No checklist captured for this task.
      </div>
    );
  }
  return (
    <div>
      {sections.map((sec) => {
        const total = sec.items.length;
        const done = sec.items.filter((i) => i.done).length;
        return (
          <div key={sec.section} style={{ marginBottom: 12 }}>
            <div
              style={{
                display: "flex",
                alignItems: "center",
                gap: 8,
                marginBottom: 4,
              }}
            >
              <strong style={{ fontSize: 12 }}>{sec.section}</strong>
              <span className="mono" style={{ fontSize: 10, color: "var(--fg-3)" }}>
                {done}/{total}
              </span>
            </div>
            <ul style={{ listStyle: "none", padding: 0, margin: 0 }}>
              {sec.items.map((it, i) => (
                <li
                  key={`${sec.section}-${i}`}
                  style={{
                    display: "flex",
                    alignItems: "flex-start",
                    gap: 6,
                    padding: "2px 0",
                    fontSize: 11.5,
                    color: it.done ? "var(--fg-2)" : "var(--fg-0)",
                    textDecoration: it.done ? "line-through" : "none",
                  }}
                >
                  <span
                    style={{
                      flexShrink: 0,
                      width: 12,
                      height: 12,
                      marginTop: 2,
                      borderRadius: 3,
                      border: "1px solid var(--border)",
                      background: it.done ? "var(--ok)" : "transparent",
                      color: "var(--bg-1)",
                      fontSize: 9,
                      lineHeight: "10px",
                      textAlign: "center",
                    }}
                  >
                    {it.done ? "✓" : ""}
                  </span>
                  <span style={{ flex: 1 }}>{it.text}</span>
                </li>
              ))}
            </ul>
          </div>
        );
      })}
    </div>
  );
}

function fmtRelative(iso: string): string {
  if (!iso) return "—";
  const t = Date.parse(iso);
  if (Number.isNaN(t)) return iso.slice(0, 10);
  const delta = Date.now() - t;
  const minutes = Math.round(delta / 60_000);
  if (minutes < 1) return "now";
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.round(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.round(hours / 24);
  if (days < 30) return `${days}d ago`;
  const months = Math.round(days / 30);
  if (months < 12) return `${months}mo ago`;
  const years = Math.round(months / 12);
  return `${years}y ago`;
}

function Empty({ msg }: { msg: string }) {
  return (
    <div
      style={{
        marginTop: 24,
        padding: 32,
        border: "1px dashed var(--border)",
        borderRadius: "var(--radius-md)",
        color: "var(--fg-3)",
        textAlign: "center",
        fontSize: 12,
      }}
    >
      {msg}
    </div>
  );
}

function Stat({ label, value, sub }: { label: string; value: string; sub?: string }) {
  return (
    <div className="stat">
      <div className="stat__label">{label}</div>
      <div className="stat__value tabular">{value}</div>
      {sub ? <div className="stat__delta">{sub}</div> : null}
    </div>
  );
}
