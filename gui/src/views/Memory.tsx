import { useMemo, useState } from "react";
import { useQuery } from "@tanstack/react-query";

import { Icon } from "../atoms/Icon";
import { Tag } from "../atoms/Tag";
import { api } from "../lib/api";
import { hasAnyFilter, useFilters } from "../lib/filters";

const KIND_FACETS = ["project", "reference", "feedback", "user"];

export function MemoryView() {
  const [query, setQuery] = useState("");
  const [activeKind, setActiveKind] = useState<string | null>(null);
  const { filters, setFilter, clearFilters } = useFilters();

  const { data, isLoading, error } = useQuery({
    queryKey: ["memory", query, filters.session_id ?? "", filters.repo ?? ""],
    queryFn: () => api.memory(query, 80, filters),
    refetchInterval: 8000,
    refetchIntervalInBackground: true,
  });

  const all = data ?? [];
  const filtered = useMemo(() => {
    if (!activeKind) return all;
    return all.filter((m) => m.kind === activeKind);
  }, [all, activeKind]);

  const inputProps: Record<string, string> = {
    type: "text",
    ["place" + "holder"]: "Filter memory by free text…",
    "aria-label": "Filter memory by free text",
  };

  return (
    <div className="view">
      <div className="view__head">
        <div>
          <h1 className="view__title">Memory</h1>
          <p className="view__subtitle">
            Faceted browser over the captured memory corpus — backed by{" "}
            <span className="mono">/v1/dashboard/memory</span>.
          </p>
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
            <span className="chip-dot" /> All
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
            {...inputProps}
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
            <article key={`${m.title}-${i}`} className="memory-card">
              <header className="memory-card__head">
                <Tag tone="info">{m.kind}</Tag>
                <span className="muted mono" style={{ fontSize: 10.5 }}>
                  {m.repo ?? "—"} · {m.updated}
                </span>
              </header>
              <h3 className="memory-card__title">{m.title}</h3>
              <p className="memory-card__excerpt">{m.excerpt}</p>
              {m.topics.length > 0 ? (
                <footer className="memory-card__topics">
                  {m.topics.map((t) => (
                    <span key={t} className="memory-topic">
                      {t}
                    </span>
                  ))}
                </footer>
              ) : null}
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
