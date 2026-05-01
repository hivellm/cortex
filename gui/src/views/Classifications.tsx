import { useMemo, useState } from "react";
import { useQuery } from "@tanstack/react-query";

import { Tag } from "../atoms/Tag";
import { api, type ClassificationRow } from "../lib/api";
import { fmtNum } from "../lib/format";
import { useConnKey } from "../lib/connections/useConnKey";

/// Classifications view — operator's read on what the per-event
/// classifier (Sonnet via the local CLI) is producing across the
/// corpus. Renders the topic cloud, per-severity / per-pii_risk
/// breakdown, and a recent-rows table so the user can sanity-check
/// the classifier's output at scale.
///
/// Data path: cortex-classifier-worker stamps `topics`, `severity`,
/// `pii_risk`, and `summary` on every Meili doc; cortex-api's
/// meili_loader projects them onto LaneHit extras; this view's
/// /v1/dashboard/classifications endpoint aggregates + paginates.
export function ClassificationsView() {
  const [repoFilter, setRepoFilter] = useState<string>("");
  const [topicFilter, setTopicFilter] = useState<string>("");
  const [severityFilter, setSeverityFilter] = useState<string>("");
  const [kindFilter, setKindFilter] = useState<string>("");

  const filters = useMemo(
    () => ({
      repo: repoFilter || undefined,
      topic: topicFilter || undefined,
      severity: severityFilter || undefined,
      kind: kindFilter || undefined,
      limit: 200,
    }),
    [repoFilter, topicFilter, severityFilter, kindFilter],
  );

  const connKey = useConnKey();
  const { data, isLoading, error } = useQuery({
    queryKey: [connKey, "classifications",
      repoFilter || "all",
      topicFilter || "all",
      severityFilter || "all",
      kindFilter || "all",
    ],
    queryFn: () => api.classifications(filters),
    refetchInterval: 20_000,
    refetchIntervalInBackground: true,
  });

  // Pull the unfiltered result once so the dropdowns stay populated
  // even when the active filter narrows everything to 0.
  const { data: allData } = useQuery({
    queryKey: [connKey, "classifications", "all"],
    queryFn: () => api.classifications({ limit: 1 }),
    refetchInterval: 60_000,
  });

  const stats = data?.stats;
  const rows = data?.rows ?? [];

  const severityCounts = stats?.by_severity ?? [];
  const piiCounts = stats?.by_pii_risk ?? [];
  const allRepos = allData?.stats.by_repo ?? [];
  const allTopics = allData?.stats.top_topics ?? [];

  return (
    <div className="view">
      <div className="view__head">
        <div>
          <h1 className="view__title">Classifications</h1>
          <p className="view__subtitle">
            Per-event classifier output · topics · severity · pii_risk · summary
            preview · stamped by{" "}
            <span className="mono" style={{ color: "var(--accent)" }}>
              cortex-classifier-worker
            </span>
          </p>
        </div>
        <div className="view__actions">
          <select
            value={repoFilter}
            onChange={(e) => setRepoFilter(e.target.value)}
            className="btn btn--ghost"
            style={selectStyle}
            title="Filter by project"
          >
            <option value="">All repos</option>
            {allRepos.map((r) => (
              <option key={r.repo} value={r.repo}>
                {r.repo} ({r.count})
              </option>
            ))}
          </select>
          <select
            value={kindFilter}
            onChange={(e) => setKindFilter(e.target.value)}
            className="btn btn--ghost"
            style={selectStyle}
            title="Filter by event kind"
          >
            <option value="">All kinds</option>
            <option value="turn">turn</option>
            <option value="tool_call">tool_call</option>
            <option value="agent_call">agent_call</option>
            <option value="decision">decision</option>
            <option value="memory">memory</option>
            <option value="law_violation">law_violation</option>
          </select>
          <select
            value={severityFilter}
            onChange={(e) => setSeverityFilter(e.target.value)}
            className="btn btn--ghost"
            style={selectStyle}
            title="Filter by severity"
          >
            <option value="">All severities</option>
            {severityCounts.map((s) => (
              <option key={s.kind} value={s.kind}>
                {s.kind} ({s.count})
              </option>
            ))}
          </select>
          {(repoFilter || topicFilter || severityFilter || kindFilter) && (
            <button
              className="btn btn--ghost"
              onClick={() => {
                setRepoFilter("");
                setTopicFilter("");
                setSeverityFilter("");
                setKindFilter("");
              }}
              style={{ fontSize: 11 }}
            >
              clear
            </button>
          )}
        </div>
      </div>

      {error ? (
        <Empty msg="cortex-api unreachable. Start it with cargo run -p cortex-api." />
      ) : isLoading || !stats ? (
        <Empty msg="Loading classifications…" />
      ) : (
        <>
          <div
            className="stats-grid"
            style={{ gridTemplateColumns: "repeat(4, 1fr)" }}
          >
            <Stat
              label="Total events"
              value={fmtNum(stats.total)}
              sub="filtered set"
            />
            <Stat
              label="Topics seen"
              value={String(stats.top_topics.length)}
              sub={`${allTopics.length} total in vocab`}
            />
            <Stat
              label="By severity"
              value={
                severityCounts.map((s) => `${s.kind}:${s.count}`).join(" · ") ||
                "—"
              }
              sub=""
            />
            <Stat
              label="By PII risk"
              value={
                piiCounts.map((p) => `${p.kind}:${p.count}`).join(" · ") || "—"
              }
              sub=""
            />
          </div>

          <h2 className="cls-h2">Topic cloud</h2>
          {stats.top_topics.length === 0 ? (
            <div className="muted" style={{ fontSize: 11.5, marginBottom: 16 }}>
              No topics stamped on any surfaced event yet — the
              cortex-classifier-worker may not have caught up, or all
              events landed before the classifier was running.
            </div>
          ) : (
            <div className="topic-cloud">
              {stats.top_topics.map((t) => {
                const active = topicFilter === t.topic;
                return (
                  <button
                    key={t.topic}
                    type="button"
                    className={`topic-chip ${active ? "is-active" : ""}`}
                    onClick={() =>
                      setTopicFilter(active ? "" : t.topic)
                    }
                    title={`${t.count} events`}
                    style={{
                      fontSize: 11 + Math.min(6, Math.log2(t.count + 1)),
                    }}
                  >
                    #{t.topic}
                    <span className="topic-chip__count">{t.count}</span>
                  </button>
                );
              })}
            </div>
          )}

          <h2 className="cls-h2">Recent classifications</h2>
          {rows.length === 0 ? (
            <Empty msg="No events match the active filters." />
          ) : (
            <div className="cls-list">
              {rows.map((r) => (
                <ClassificationCard key={r.event_id} row={r} />
              ))}
            </div>
          )}
        </>
      )}
    </div>
  );
}

const selectStyle: React.CSSProperties = {
  fontFamily: "var(--font-mono)",
  fontSize: 11.5,
  padding: "5px 10px",
};

function ClassificationCard({ row }: { row: ClassificationRow }) {
  const sevTone =
    row.severity === "critical"
      ? "critical"
      : row.severity === "notable"
        ? "warn"
        : "default";
  return (
    <article className="cls-card">
      <div className="cls-card__head">
        <span className="cls-card__kind mono">{row.kind}</span>
        {row.repo ? <Tag tone="info">{row.repo}</Tag> : null}
        {row.severity ? (
          <Tag tone={sevTone as "critical" | "warn" | "default"}>
            {row.severity}
          </Tag>
        ) : null}
        {row.pii_risk && row.pii_risk !== "none" ? (
          <Tag tone="warn">pii: {row.pii_risk}</Tag>
        ) : null}
        <span
          className="muted mono"
          style={{ marginLeft: "auto", fontSize: 10.5 }}
        >
          {row.at}
        </span>
      </div>
      {row.path ? (
        <div className="cls-card__path mono">{row.path}</div>
      ) : null}
      {row.summary ? (
        <p className="cls-card__summary">{row.summary}</p>
      ) : null}
      {row.topics.length > 0 ? (
        <div className="cls-card__topics">
          {row.topics.map((t) => (
            <Tag key={t}>#{t}</Tag>
          ))}
        </div>
      ) : null}
    </article>
  );
}

function Stat({
  label,
  value,
  sub,
}: {
  label: string;
  value: string;
  sub: string;
}) {
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
