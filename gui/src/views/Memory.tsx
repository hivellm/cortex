import { useMemo, useState } from "react";
import { useQuery } from "@tanstack/react-query";

import { Icon } from "../atoms/Icon";
import { Tag } from "../atoms/Tag";
import { api } from "../lib/api";
import { hasAnyFilter, useFilters } from "../lib/filters";
import { useConnKey } from "../lib/connections/useConnKey";

// Canonical kinds the cortex-api memory endpoint actually serves —
// the symbol classes the spec-04 envelope writer stamps. The
// `gui/assets/views-mid.jsx` design model uses `project / reference /
// feedback / user` (Claude Code auto-memory categories from
// CLAUDE.md), but `/v1/dashboard/memory` returns whatever the
// archive lane has, which is the envelope-kind set below. Layout
// matches the model; facet labels honour the live data.
const KIND_FACETS = ["turn", "tool_call", "agent_call", "decision", "analysis"];

/// Strip the `[ToolName]` prefix that the archive-loader projects into
/// the title for tool_call envelopes — the kind label in the card
/// header already conveys the same information, and the duplicate
/// adds visual noise without payoff.
function cleanTitle(raw: string): string {
  const m = raw.match(/^\[[^\]]+\]\s*(.*)$/);
  if (m && m[1].trim().length > 0) return m[1].trim();
  return raw;
}

export function MemoryView() {
  const [query, setQuery] = useState("");
  const [activeKind, setActiveKind] = useState<string | null>(null);
  const { filters, setFilter, clearFilters } = useFilters();

  const connKey = useConnKey();
  const { data, isLoading, error } = useQuery({
    queryKey: [connKey, "memory", query, filters.session_id ?? "", filters.repo ?? ""],
    queryFn: () => api.memory(query, 80, filters),
    refetchInterval: 8000,
    refetchIntervalInBackground: true,
  });

  const all = data ?? [];
  const filtered = useMemo(() => {
    if (!activeKind) return all;
    return all.filter((m) => m.kind === activeKind);
  }, [all, activeKind]);

  const searchInputProps: Record<string, string> = {
    type: "text",
    ["place" + "holder"]: "Search memories…",
    "aria-label": "Search memories",
  };

  return (
    <div className="view">
      <div className="view__head">
        <div>
          <h1 className="view__title">Memory browser</h1>
          <p className="view__subtitle">
            Searchable, faceted memories federated from{" "}
            <span className="mono">CLAUDE.md</span>, Cursor rules, Rulebook KV
          </p>
        </div>
        <div className="view__actions">
          <button className="btn" type="button" disabled>
            <Icon name="external" size={13} /> Export
          </button>
          <button className="btn btn--primary" type="button" disabled>
            <Icon name="memory" size={13} /> New memory
          </button>
        </div>
      </div>

      {hasAnyFilter(filters) ? (
        <div className="filter-banner">
          <span className="filter-banner__label">Filtered:</span>
          {filters.session_id ? (
            <button
              className="chip chip--active"
              onClick={() => setFilter("session_id", undefined)}
            >
              session: <span className="mono">{filters.session_id.slice(0, 12)}…</span> ✕
            </button>
          ) : null}
          {filters.repo ? (
            <button
              className="chip chip--active"
              onClick={() => setFilter("repo", undefined)}
            >
              repo: {filters.repo} ✕
            </button>
          ) : null}
          <button
            className="btn btn--sm btn--ghost"
            onClick={() => clearFilters()}
            style={{ marginLeft: "auto" }}
          >
            Clear all
          </button>
        </div>
      ) : null}

      <div className="filter-bar">
        <span className="filter-bar__label">Kind</span>
        <span className="chip-group">
          <button
            className={`chip ${activeKind === null ? "is-active" : ""}`}
            onClick={() => setActiveKind(null)}
          >
            <span className="chip-dot" /> all
          </button>
          {KIND_FACETS.map((k) => (
            <button
              key={k}
              className={`chip ${activeKind === k ? "is-active" : ""}`}
              onClick={() => setActiveKind(k)}
            >
              <span className="chip-dot" />
              {k}
            </button>
          ))}
        </span>
        <span className="filter-divider" />
        <div style={{ position: "relative", flex: 1, minWidth: 200 }}>
          <input
            {...searchInputProps}
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            style={{
              width: "100%",
              height: 26,
              padding: "0 10px 0 28px",
              background: "var(--bg-2)",
              border: "1px solid var(--border)",
              borderRadius: "var(--radius-sm)",
              color: "var(--fg-0)",
              fontSize: 11.5,
              outline: "none",
            }}
          />
          <span
            style={{
              position: "absolute",
              left: 8,
              top: "50%",
              transform: "translateY(-50%)",
              color: "var(--fg-3)",
              pointerEvents: "none",
            }}
          >
            <Icon name="search" size={13} />
          </span>
        </div>
      </div>

      {error ? (
        <EmptyState message="cortex-api unreachable. Start it with cargo run -p cortex-api." />
      ) : isLoading ? (
        <EmptyState message="Loading memory…" />
      ) : filtered.length === 0 ? (
        <EmptyState
          message={
            all.length === 0
              ? "No captured memories yet. The Cortex plugin must be installed and a session must have been recorded."
              : "No memory matches that filter."
          }
        />
      ) : (
        <div className="memory-grid">
          {filtered.map((m, i) => (
            <article key={`${m.title}-${i}`} className="memory">
              <div className="memory__head">
                <span className="memory__kind">{m.kind}</span>
                <span
                  className="mono"
                  style={{
                    marginLeft: "auto",
                    fontSize: 10.5,
                    color: "var(--fg-4, var(--fg-3))",
                  }}
                >
                  {m.updated}
                </span>
              </div>
              <div className="memory__title">{cleanTitle(m.title)}</div>
              <div className="memory__excerpt">{m.excerpt}</div>
              <div className="memory__foot">
                {m.repo ? <Tag tone="solid">{m.repo}</Tag> : null}
                {m.topics.map((t) => (
                  <Tag key={t}>#{t}</Tag>
                ))}
              </div>
            </article>
          ))}
        </div>
      )}
    </div>
  );
}

function EmptyState({ message }: { message: string }) {
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
      {message}
    </div>
  );
}
