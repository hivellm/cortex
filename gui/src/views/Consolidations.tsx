import { useMemo, useState } from "react";
import { useQuery } from "@tanstack/react-query";

import { DetailDrawer } from "../atoms/DetailDrawer";
import { Markdown } from "../atoms/Markdown";
import { Tag } from "../atoms/Tag";
import { api, type ConsolidationRow, type ConsolidatorHealthReport, type GrainHealth } from "../lib/api";
import { useConnKey } from "../lib/connections/useConnKey";

const GRAINS = ["", "session", "topic", "decision_trace"] as const;
type Grain = (typeof GRAINS)[number];

export function ConsolidationsView() {
  const [repoFilter, setRepoFilter] = useState<string>("");
  const [grainFilter, setGrainFilter] = useState<Grain>("");
  const [openId, setOpenId] = useState<string | null>(null);

  const connKey = useConnKey();
  const { data: allRows } = useQuery({
    queryKey: [connKey, "consolidations", "all"],
    queryFn: () => api.consolidations(),
    refetchInterval: 60_000,
  });

  const { data, isLoading, error } = useQuery({
    queryKey: [connKey, "consolidations", repoFilter || "all", grainFilter || "any"],
    queryFn: () =>
      api.consolidations({
        repo: repoFilter || undefined,
        grain: grainFilter || undefined,
      }),
    refetchInterval: 60_000,
    refetchIntervalInBackground: true,
  });

  // phase14a §4.2 — pulls the daemon-side last-run / last-status
  // per grain. Surfaced as a banner so operators can tell whether
  // the daemon has fired recently before reading the consolidation
  // list itself. Polled every 30 s — the daemon is not a hot path.
  const { data: daemonHealth } = useQuery({
    queryKey: [connKey, "consolidator-health"],
    queryFn: () => api.healthConsolidator(),
    refetchInterval: 30_000,
    refetchIntervalInBackground: true,
  });

  const rows = data?.rows ?? [];
  const allRowsList = allRows?.rows ?? [];

  const repoOptions = useMemo(() => {
    const set = new Set<string>();
    for (const r of allRowsList) {
      if (r.repo) set.add(r.repo.toLowerCase());
    }
    return Array.from(set).sort();
  }, [allRowsList]);

  const counts = useMemo(() => {
    const total = allRows?.total ?? 0;
    const byGrain: Record<string, number> = {};
    let totalSourceEvents = 0;
    for (const r of allRowsList) {
      const g = r.grain || "(legacy)";
      byGrain[g] = (byGrain[g] ?? 0) + 1;
      totalSourceEvents += r.source_event_count ?? 0;
    }
    return { total, byGrain, totalSourceEvents };
  }, [allRows, allRowsList]);

  return (
    <div className="view">
      <div className="view__head">
        <div>
          <h1 className="view__title">Consolidations</h1>
          <p className="view__subtitle">
            Session / topic / decision-trace digests · cortex-consolidator + tool-call-digest output
          </p>
        </div>
        <div className="view__actions">
          <select
            value={repoFilter}
            onChange={(e) => setRepoFilter(e.target.value)}
            className="btn btn--ghost"
            style={{ fontFamily: "var(--font-mono)", fontSize: 12, padding: "5px 10px" }}
            title="Filter consolidations by project"
          >
            <option value="">All projects ({allRows?.total ?? 0})</option>
            {repoOptions.map((r) => {
              const c = allRowsList.filter(
                (d) => d.repo?.toLowerCase() === r,
              ).length;
              return (
                <option key={r} value={r}>
                  {r} ({c})
                </option>
              );
            })}
          </select>
          <select
            value={grainFilter}
            onChange={(e) => setGrainFilter(e.target.value as Grain)}
            className="btn btn--ghost"
            style={{ fontFamily: "var(--font-mono)", fontSize: 12, padding: "5px 10px" }}
            title="Filter by consolidation grain"
          >
            <option value="">All grains</option>
            <option value="session">session</option>
            <option value="topic">topic</option>
            <option value="decision_trace">decision_trace</option>
          </select>
        </div>
      </div>

      <div className="stats-grid" style={{ gridTemplateColumns: "repeat(3, 1fr)" }}>
        <Stat label="Total consolidations" value={String(counts.total)} sub="across every project" />
        <Stat
          label="By grain"
          value={Object.entries(counts.byGrain)
            .map(([g, n]) => `${g}:${n}`)
            .join("  ") || "—"}
          sub={Object.keys(counts.byGrain).length === 0 ? "no data" : undefined}
        />
        <Stat
          label="Source events folded"
          value={counts.totalSourceEvents.toLocaleString()}
          sub="raw envelopes summarised"
        />
      </div>

      <DaemonHealthPanel report={daemonHealth} />

      {error ? (
        <Empty msg="cortex-api unreachable. Start it with cargo run -p cortex-api." />
      ) : isLoading ? (
        <Empty msg="Loading consolidations…" />
      ) : rows.length === 0 ? (
        <Empty msg="No consolidations match the current filter. cortex-consolidator and tool_call_digest produce these on their cron schedule." />
      ) : (
        <div className="decision-list">
          {rows.map((c) => (
            <article
              key={c.id}
              className="decision"
              role="button"
              tabIndex={0}
              onClick={() => setOpenId(c.id)}
              onKeyDown={(e) => {
                if (e.key === "Enter" || e.key === " ") {
                  e.preventDefault();
                  setOpenId(c.id);
                }
              }}
              title="Click to view full consolidation"
            >
              <div className="decision__head">
                <span className="decision__id">{c.consolidation_id ?? c.id}</span>
                {c.repo ? <Tag tone="info">{c.repo}</Tag> : null}
                {c.grain ? <Tag>{c.grain}</Tag> : null}
                {c.depth ? <Tag tone={c.depth === "deep" ? "warn" : undefined}>{c.depth}</Tag> : null}
                <span className="decision__title">{c.title}</span>
                {c.source_event_count > 0 ? (
                  <Tag tone="ok">{c.source_event_count} events</Tag>
                ) : null}
                <span className="muted mono" style={{ marginLeft: "auto", fontSize: 10.5 }}>
                  {c.occurred_at}
                </span>
              </div>
              <p className="decision__rationale">{c.excerpt}</p>
              <div className="decision__foot">
                {c.model ? (
                  <span>
                    by <span className="strong">{c.model}</span>
                  </span>
                ) : null}
                {c.topics.length > 0 ? (
                  <span style={{ marginLeft: "auto", display: "flex", gap: 6 }}>
                    {c.topics.slice(0, 6).map((t) => (
                      <Tag key={t}>#{t}</Tag>
                    ))}
                  </span>
                ) : null}
              </div>
            </article>
          ))}
        </div>
      )}

      <ConsolidationDrawer
        id={openId}
        row={openId ? rows.find((r) => r.id === openId) ?? null : null}
        onClose={() => setOpenId(null)}
      />
    </div>
  );
}

function ConsolidationDrawer({
  id,
  row,
  onClose,
}: {
  id: string | null;
  row: ConsolidationRow | null;
  onClose: () => void;
}) {
  const connKey = useConnKey();
  const { data, isLoading, error } = useQuery({
    queryKey: [connKey, "consolidation-detail", id ?? ""],
    queryFn: () => api.consolidationDetail(id as string),
    enabled: !!id,
  });

  return (
    <DetailDrawer
      open={!!id}
      onClose={onClose}
      title={
        <span>
          <span className="mono" style={{ color: "var(--accent)", marginRight: 8 }}>
            {row?.consolidation_id ?? id}
          </span>
          {row?.title ?? data?.title ?? ""}
        </span>
      }
      subtitle={
        <span>
          {row?.repo ? `${row.repo} · ` : ""}
          {row?.grain || "?"} · {row?.depth || "?"}
          {row?.model ? ` · ${row.model}` : ""}
          {row?.source_event_count ? ` · ${row.source_event_count} source events` : ""}
        </span>
      }
    >
      {error ? (
        <pre className="drawer__markdown" style={{ color: "var(--err, #f88)" }}>
          Failed to load consolidation body.
        </pre>
      ) : isLoading ? (
        <pre className="drawer__markdown muted">Loading…</pre>
      ) : (
        <Markdown
          source={
            data?.body_markdown?.trim()
              ? data.body_markdown
              : row?.excerpt ?? "(no body)"
          }
        />
      )}
    </DetailDrawer>
  );
}

function DaemonHealthPanel({ report }: { report?: ConsolidatorHealthReport }) {
  const grains: Array<{ key: string; label: string; data?: GrainHealth }> = [
    { key: "session", label: "session", data: report?.session_grain },
    { key: "topic", label: "topic", data: report?.topic_grain },
    { key: "decision_trace", label: "decision_trace", data: report?.decision_trace_grain },
  ];
  return (
    <div
      style={{
        marginTop: 16,
        padding: "10px 14px",
        border: "1px solid var(--border)",
        borderRadius: "var(--radius-md)",
        background: "var(--bg-2, transparent)",
        display: "flex",
        gap: 18,
        alignItems: "center",
        flexWrap: "wrap",
        fontSize: 12,
      }}
      title="Daemon-side last-run + last-status per grain (cortex-consolidator). Polled every 30s from /v1/health/consolidator."
    >
      <span className="muted" style={{ fontWeight: 600, letterSpacing: 0.4 }}>
        DAEMON
      </span>
      {grains.map((g) => {
        const last = g.data?.last_run ?? null;
        const status = g.data?.last_status ?? null;
        const tone: "ok" | "warn" | "info" | undefined =
          status === "success" ? "ok" : status === "failed" ? "warn" : undefined;
        return (
          <span
            key={g.key}
            style={{ display: "flex", gap: 6, alignItems: "baseline" }}
          >
            <span className="mono" style={{ color: "var(--fg-2)" }}>
              {g.label}
            </span>
            {status ? (
              <Tag tone={tone}>{status}</Tag>
            ) : (
              <span className="muted">never</span>
            )}
            {last ? (
              <span
                className="muted mono"
                style={{ fontSize: 10.5 }}
                title={last}
              >
                {formatRelative(last)}
              </span>
            ) : null}
            {g.data && g.data.envelopes_emitted > 0 ? (
              <span className="muted" style={{ fontSize: 10.5 }}>
                · {g.data.envelopes_emitted} env · {g.data.latency_ms}ms
              </span>
            ) : null}
          </span>
        );
      })}
      {!report ? (
        <span className="muted" style={{ marginLeft: "auto" }}>
          /v1/health/consolidator unreachable
        </span>
      ) : null}
    </div>
  );
}

function formatRelative(iso: string): string {
  const t = new Date(iso).getTime();
  if (Number.isNaN(t)) return iso;
  const delta = Math.max(0, Date.now() - t);
  const m = Math.floor(delta / 60_000);
  if (m < 1) return "just now";
  if (m < 60) return `${m}m ago`;
  const h = Math.floor(m / 60);
  if (h < 24) return `${h}h ago`;
  const d = Math.floor(h / 24);
  return `${d}d ago`;
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
