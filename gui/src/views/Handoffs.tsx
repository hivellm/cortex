import { useMemo, useState } from "react";
import { useQuery } from "@tanstack/react-query";

import { Tag } from "../atoms/Tag";
import { api } from "../lib/api";
import { useConnKey } from "../lib/connections/useConnKey";

/// Handoffs view — surfaces every `.rulebook/handoff/*.md` snapshot
/// across all bootstrapped repos, grouped by project. The walker
/// promotes these as memory envelopes; this view filters for the
/// hand-off subset so a user resuming work can pull the latest
/// hand-off from any project without grepping the filesystem.
export function HandoffsView() {
  const [repoFilter, setRepoFilter] = useState<string>("");

  const connKey = useConnKey();
  const { data, isLoading, error } = useQuery({
    queryKey: [connKey, "handoffs", repoFilter || "all"],
    queryFn: () => api.handoffs(repoFilter || undefined),
    refetchInterval: 15_000,
    refetchIntervalInBackground: true,
  });

  const { data: allRows } = useQuery({
    queryKey: [connKey, "handoffs", "all"],
    queryFn: () => api.handoffs(),
    refetchInterval: 60_000,
  });

  const rows = data ?? [];
  const repos = useMemo(() => {
    const s = new Set<string>();
    for (const r of allRows ?? []) {
      if (r.repo) s.add(r.repo);
    }
    return Array.from(s).sort();
  }, [allRows]);

  return (
    <div className="view">
      <div className="view__head">
        <div>
          <h1 className="view__title">Handoffs</h1>
          <p className="view__subtitle">
            Per-project session hand-offs · `.rulebook/handoff/*.md` ·
            indexed and queryable
          </p>
        </div>
        <div className="view__actions">
          <select
            value={repoFilter}
            onChange={(e) => setRepoFilter(e.target.value)}
            className="btn btn--ghost"
            style={{
              fontFamily: "var(--font-mono)",
              fontSize: 12,
              padding: "5px 10px",
            }}
            title="Filter handoffs by project"
          >
            <option value="">All projects ({allRows?.length ?? 0})</option>
            {repos.map((r) => {
              const c = (allRows ?? []).filter((row) => row.repo === r).length;
              return (
                <option key={r} value={r}>
                  {r} ({c})
                </option>
              );
            })}
          </select>
        </div>
      </div>

      {error ? (
        <Empty msg="cortex-api unreachable. Start it with cargo run -p cortex-api." />
      ) : isLoading ? (
        <Empty msg="Loading handoffs…" />
      ) : rows.length === 0 ? (
        <Empty
          msg={
            repoFilter
              ? `No handoffs captured for ${repoFilter}.`
              : "No handoffs captured yet. Bootstrap a repo with `.rulebook/handoff/*.md` to populate this list."
          }
        />
      ) : (
        <div className="handoff-list">
          {rows.map((r, i) => (
            <article
              key={`${r.repo ?? "unk"}-${r.path ?? i}`}
              className="handoff"
            >
              <div className="handoff__head">
                {r.repo ? <Tag tone="info">{r.repo}</Tag> : null}
                <span className="handoff__filename mono">{r.filename}</span>
                <span
                  className="muted"
                  style={{ marginLeft: "auto", fontSize: 11 }}
                >
                  {r.updated || ""}
                </span>
              </div>
              {r.path ? (
                <div className="handoff__path mono muted">{r.path}</div>
              ) : null}
              <pre className="handoff__excerpt">{r.excerpt}</pre>
            </article>
          ))}
        </div>
      )}
    </div>
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
