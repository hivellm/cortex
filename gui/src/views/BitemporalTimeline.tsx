import { useEffect, useState } from "react";
import { useQuery } from "@tanstack/react-query";

import { Tag } from "../atoms/Tag";
import {
  api,
  type BitemporalTimelineEvent,
  type TimelineFilters,
} from "../lib/api";
import { useConnKey } from "../lib/connections/useConnKey";

const STORAGE_KEY = "cortex.bitemporal-timeline.filters";

const TIMELINE_KINDS = [
  "session",
  "commit",
  "adr",
  "decision",
  "analysis",
  "memory",
  "release",
  "incident",
  "learning",
  "task_start",
  "task_archive",
  "branch_fork",
  "branch_merge",
  "branch_abandon",
  "cross_project_link",
] as const;

type StoredFilters = {
  project: string;
  as_of: string;
  branch: string;
  kinds: string[];
  from: string;
  to: string;
  limit: number;
};

const DEFAULT_STORED: StoredFilters = {
  project: "cortex",
  as_of: "",
  branch: "",
  kinds: [],
  from: "",
  to: "",
  limit: 50,
};

function loadStoredFilters(): StoredFilters {
  if (typeof window === "undefined") return DEFAULT_STORED;
  try {
    const raw = window.localStorage.getItem(STORAGE_KEY);
    if (!raw) return DEFAULT_STORED;
    const parsed = JSON.parse(raw) as Partial<StoredFilters>;
    return {
      project: typeof parsed.project === "string" ? parsed.project : "cortex",
      as_of: typeof parsed.as_of === "string" ? parsed.as_of : "",
      branch: typeof parsed.branch === "string" ? parsed.branch : "",
      kinds: Array.isArray(parsed.kinds) ? parsed.kinds : [],
      from: typeof parsed.from === "string" ? parsed.from : "",
      to: typeof parsed.to === "string" ? parsed.to : "",
      limit: typeof parsed.limit === "number" ? parsed.limit : 50,
    };
  } catch {
    return DEFAULT_STORED;
  }
}

function persistFilters(state: StoredFilters): void {
  if (typeof window === "undefined") return;
  window.localStorage.setItem(STORAGE_KEY, JSON.stringify(state));
}

function fmtUnix(unix: number | undefined): string {
  if (!unix) return "—";
  return new Date(unix * 1000).toISOString().slice(0, 10);
}

/** Emit a cross-view event to open the EntityHistory view for an entity. */
function openEntityHistory(id: string): void {
  window.dispatchEvent(
    new CustomEvent("cortex:open-entity-history", { detail: { id } }),
  );
}

export function BitemporalTimelineView() {
  const [stored, setStored] = useState<StoredFilters>(() => loadStoredFilters());

  useEffect(() => {
    persistFilters(stored);
  }, [stored]);

  const connKey = useConnKey();

  const branchesQ = useQuery({
    queryKey: [connKey, "branches", stored.project],
    queryFn: () => api.branches(stored.project),
    refetchInterval: 300_000,
    enabled: stored.project.trim().length > 0,
  });

  const filters: TimelineFilters = {};
  if (stored.branch) filters.branch = stored.branch;
  if (stored.kinds.length > 0) filters.kind = stored.kinds.join(",");
  if (stored.from) filters.from = stored.from;
  if (stored.to) filters.to = stored.to;
  if (stored.as_of) filters.as_of = stored.as_of;
  if (stored.limit !== 50) filters.limit = stored.limit;

  const { data, isLoading, error } = useQuery({
    queryKey: [connKey, "bitemporal-timeline", stored.project, stored],
    queryFn: () => api.bitemporalTimeline(stored.project, filters),
    refetchInterval: 300_000,
    enabled: stored.project.trim().length > 0,
  });

  const events: BitemporalTimelineEvent[] = data?.events ?? [];

  const toggleKind = (kind: string) => {
    setStored((prev) => {
      const next = prev.kinds.includes(kind)
        ? prev.kinds.filter((k) => k !== kind)
        : [...prev.kinds, kind];
      return { ...prev, kinds: next };
    });
  };

  const branchOptions = branchesQ.data?.branches ?? [];

  return (
    <div className="view">
      <div className="view__head">
        <div>
          <h1 className="view__title">Bitemporal Timeline</h1>
          <p className="view__subtitle">
            Project event history ·{" "}
            <span className="mono">/v1/timeline/{"{project}"}</span>
          </p>
        </div>
      </div>

      <div
        className="filter-bar"
        style={{
          gap: 12,
          flexWrap: "wrap",
          position: "sticky",
          top: 0,
          zIndex: 5,
          background: "var(--bg-1)",
          paddingTop: 8,
          paddingBottom: 8,
          borderBottom: "1px solid var(--border)",
        }}
      >
        <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
          <span style={{ color: "var(--fg-3)", fontSize: 11 }}>Project</span>
          <input
            type="text"
            value={stored.project}
            onChange={(e) => setStored((p) => ({ ...p, project: e.target.value }))}
            style={{
              height: 28,
              padding: "0 10px",
              background: "var(--bg-2)",
              border: "1px solid var(--border)",
              borderRadius: "var(--radius-sm)",
              color: "var(--fg-0)",
              fontSize: 11.5,
              width: 120,
            }}
            aria-label="Project name"
          />
          <button
            type="button"
            className={`btn btn--sm ${stored.project === "_all" ? "" : "btn--ghost"}`}
            onClick={() =>
              setStored((p) => ({
                ...p,
                project: p.project === "_all" ? "cortex" : "_all",
                branch: "",
              }))
            }
            title="Union every project's events into one cross-project progress feed"
          >
            All projects
          </button>
        </div>

        <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
          <span style={{ color: "var(--fg-3)", fontSize: 11 }}>As of</span>
          <input
            type="date"
            value={stored.as_of}
            onChange={(e) => setStored((p) => ({ ...p, as_of: e.target.value }))}
            style={{
              height: 28,
              padding: "0 8px",
              background: "var(--bg-2)",
              border: "1px solid var(--border)",
              borderRadius: "var(--radius-sm)",
              color: "var(--fg-0)",
              fontSize: 11.5,
            }}
            aria-label="As of date"
          />
        </div>

        <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
          <span style={{ color: "var(--fg-3)", fontSize: 11 }}>Branch</span>
          <select
            value={stored.branch}
            onChange={(e) => setStored((p) => ({ ...p, branch: e.target.value }))}
            className="btn btn--ghost"
            style={{ fontFamily: "var(--font-mono)", fontSize: 12, padding: "5px 10px" }}
            aria-label="Branch filter"
          >
            <option value="">main (default)</option>
            {branchOptions.map((b) => (
              <option key={b.id ?? b.name} value={b.name ?? b.id ?? ""}>
                {b.name ?? b.id}
              </option>
            ))}
          </select>
        </div>

        <div style={{ display: "flex", alignItems: "center", gap: 6, flexWrap: "wrap" }}>
          <span style={{ color: "var(--fg-3)", fontSize: 11 }}>Kind</span>
          {TIMELINE_KINDS.map((k) => (
            <button
              key={k}
              className={`btn btn--sm ${stored.kinds.includes(k) ? "" : "btn--ghost"}`}
              onClick={() => toggleKind(k)}
              title={`Filter to kind: ${k}`}
            >
              {k}
            </button>
          ))}
        </div>

        <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
          <span style={{ color: "var(--fg-3)", fontSize: 11 }}>From</span>
          <input
            type="date"
            value={stored.from}
            onChange={(e) => setStored((p) => ({ ...p, from: e.target.value }))}
            style={{
              height: 28,
              padding: "0 8px",
              background: "var(--bg-2)",
              border: "1px solid var(--border)",
              borderRadius: "var(--radius-sm)",
              color: "var(--fg-0)",
              fontSize: 11.5,
            }}
            aria-label="From date"
          />
          <span style={{ color: "var(--fg-3)", fontSize: 11 }}>To</span>
          <input
            type="date"
            value={stored.to}
            onChange={(e) => setStored((p) => ({ ...p, to: e.target.value }))}
            style={{
              height: 28,
              padding: "0 8px",
              background: "var(--bg-2)",
              border: "1px solid var(--border)",
              borderRadius: "var(--radius-sm)",
              color: "var(--fg-0)",
              fontSize: 11.5,
            }}
            aria-label="To date"
          />
        </div>

        <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
          <span style={{ color: "var(--fg-3)", fontSize: 11 }}>Limit</span>
          <input
            type="number"
            value={stored.limit}
            min={1}
            max={500}
            onChange={(e) =>
              setStored((p) => ({ ...p, limit: Number(e.target.value) || 50 }))
            }
            style={{
              height: 28,
              padding: "0 8px",
              background: "var(--bg-2)",
              border: "1px solid var(--border)",
              borderRadius: "var(--radius-sm)",
              color: "var(--fg-0)",
              fontSize: 11.5,
              width: 72,
            }}
            aria-label="Event limit"
          />
        </div>
      </div>

      {error ? (
        <Empty msg="cortex-api unreachable. Start it with cargo run -p cortex-api." />
      ) : isLoading ? (
        <Empty msg="Loading timeline events…" />
      ) : events.length === 0 ? (
        <Empty msg="No bitemporal events found for the active filters." />
      ) : (
        <div className="decision-list">
          {events.map((ev, i) => (
            <EventRow key={ev.id ?? String(i)} ev={ev} />
          ))}
        </div>
      )}
    </div>
  );
}

function EventRow({ ev }: { ev: BitemporalTimelineEvent }) {
  return (
    <article
      className="decision"
      style={{ display: "flex", flexDirection: "column", gap: 4 }}
    >
      <div className="decision__head">
        {ev.project_id ? <Tag tone="accent">{ev.project_id}</Tag> : null}
        {ev.kind ? <Tag tone="info">{ev.kind}</Tag> : null}
        <span className="decision__title">{ev.title ?? "(no title)"}</span>
        <span
          className="muted mono"
          style={{ marginLeft: "auto", fontSize: 10.5 }}
        >
          {fmtUnix(ev.valid_from_unix)}
        </span>
      </div>
      {ev.summary ? (
        <p style={{ fontSize: 11.5, color: "var(--fg-2)", margin: 0 }}>
          {ev.summary}
        </p>
      ) : null}
      {ev.entity_id ? (
        <div style={{ marginTop: 2 }}>
          <button
            type="button"
            className="link-btn"
            style={{
              fontFamily: "var(--font-mono)",
              fontSize: 10.5,
              color: "var(--accent)",
              background: "transparent",
              border: 0,
              cursor: "pointer",
              padding: 0,
            }}
            title="Open entity history"
            onClick={() => openEntityHistory(ev.entity_id!)}
          >
            {ev.entity_id}
          </button>
        </div>
      ) : null}
    </article>
  );
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
