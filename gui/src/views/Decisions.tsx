import { useMemo, useState } from "react";
import { useQuery } from "@tanstack/react-query";

import { DetailDrawer } from "../atoms/DetailDrawer";
import { Icon } from "../atoms/Icon";
import { Tag } from "../atoms/Tag";
import { api, type DecisionChainNode, type DecisionRow } from "../lib/api";
import { useConnKey } from "../lib/connections/useConnKey";

export function DecisionsView() {
  const [showSuperseded, setShowSuperseded] = useState(false);
  const [repoFilter, setRepoFilter] = useState<string>("");
  const [openId, setOpenId] = useState<string | null>(null);

  // Pull the unfiltered list once so the repo dropdown stays stable
  // even as the filtered view narrows. The list is small (decisions
  // count in the tens, not hundreds) so a second cache is cheap.
  const connKey = useConnKey();
  const { data: allRows } = useQuery({
    queryKey: [connKey, "decisions", "all"],
    queryFn: () => api.decisions(),
    // Spec 21 — useDashboardStream pushes on decision.changed.
    refetchInterval: 300_000,
  });

  const { data, isLoading, error } = useQuery({
    queryKey: [connKey, "decisions", repoFilter || "all"],
    queryFn: () => api.decisions(repoFilter || undefined),
    refetchInterval: 300_000,
    refetchIntervalInBackground: true,
  });

  const rowsRaw = data ?? [];
  const active = rowsRaw.filter((d) => d.status === "active").length;
  const superseded = rowsRaw.filter((d) => d.status === "superseded").length;
  const withRationale = rowsRaw.filter((d) => !!d.rationale).length;

  const repoOptions = useMemo(() => {
    const set = new Set<string>();
    for (const d of allRows ?? []) {
      if (d.repo) set.add(d.repo);
    }
    return Array.from(set).sort();
  }, [allRows]);

  const rows = useMemo(
    () =>
      showSuperseded ? rowsRaw : rowsRaw.filter((d) => d.status !== "superseded"),
    [rowsRaw, showSuperseded],
  );

  return (
    <div className="view">
      <div className="view__head">
        <div>
          <h1 className="view__title">Decision register</h1>
          <p className="view__subtitle">
            ADR-style decisions · supersedable · cited from pre-thinking bundles
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
            title="Filter decisions by project"
          >
            <option value="">All projects ({allRows?.length ?? 0})</option>
            {repoOptions.map((r) => {
              const c =
                (allRows ?? []).filter((d) => d.repo === r).length;
              return (
                <option key={r} value={r}>
                  {r} ({c})
                </option>
              );
            })}
          </select>
          <button
            className={`btn ${showSuperseded ? "" : "btn--ghost"}`}
            onClick={() => setShowSuperseded((s) => !s)}
            title={
              showSuperseded
                ? "Hide superseded decisions"
                : `Show ${superseded} superseded decision${superseded === 1 ? "" : "s"}`
            }
          >
            {showSuperseded ? "✓ " : ""}Show superseded
          </button>
          <button
            className="btn btn--primary"
            type="button"
            disabled
            title="Promotes a candidate decision out of an in-progress analysis. Available once spec-15 emits kind=analysis envelopes."
          >
            <Icon name="decision" size={13} /> Promote candidate
          </button>
        </div>
      </div>

      <div className="stats-grid" style={{ gridTemplateColumns: "repeat(3, 1fr)" }}>
        <Stat label="Total decisions" value={String(rows.length)} sub="captured envelopes" />
        <Stat label="Active" value={String(active)} sub={`${superseded} superseded`} />
        <Stat label="With rationale" value={String(withRationale)} sub="non-empty body" />
      </div>

      {error ? (
        <Empty msg="cortex-api unreachable. Start it with cargo run -p cortex-api." />
      ) : isLoading ? (
        <Empty msg="Loading decisions…" />
      ) : rows.length === 0 ? (
        <Empty msg="No decisions captured yet. The Cortex graph populates this list as decision envelopes flow in." />
      ) : (
        <div className="decision-list">
          {rows.map((d) => (
            <article
              key={d.id}
              className={`decision ${d.status === "superseded" ? "is-superseded" : ""}`}
              role="button"
              tabIndex={0}
              onClick={() => setOpenId(d.id)}
              onKeyDown={(e) => {
                if (e.key === "Enter" || e.key === " ") {
                  e.preventDefault();
                  setOpenId(d.id);
                }
              }}
              title="Click to view full decision"
            >
              <div className="decision__head">
                <span className="decision__id">{d.id}</span>
                {d.repo ? <Tag tone="info">{d.repo}</Tag> : null}
                <span className="decision__title">{d.title}</span>
                {d.status === "active" ? <Tag tone="ok">active</Tag> : null}
                {d.status === "superseded" && d.superseded_by ? (
                  <Tag>superseded → {d.superseded_by}</Tag>
                ) : null}
                {d.status !== "active" && d.status !== "superseded" ? (
                  <Tag>{d.status}</Tag>
                ) : null}
                {d.supersedes ? (
                  <Tag tone="warn">supersedes {d.supersedes}</Tag>
                ) : null}
                <span className="muted mono" style={{ marginLeft: "auto", fontSize: 10.5 }}>
                  {d.occurred_at}
                </span>
              </div>
              {d.rationale ? (
                <p className="decision__rationale">{d.rationale}</p>
              ) : null}
              <div className="decision__foot">
                {d.author ? (
                  <span>
                    by <span className="strong">{d.author}</span>
                  </span>
                ) : null}
                {d.author && (d.source_analysis || d.cites.length > 0) ? <span>·</span> : null}
                {d.source_analysis ? (
                  <>
                    <span>
                      from <span style={{ color: "var(--accent)" }}>{d.source_analysis}</span>
                    </span>
                    <span>·</span>
                  </>
                ) : null}
                <span>{d.occurred_at}</span>
                {d.cites.length > 0 ? (
                  <>
                    <span>·</span>
                    <span>
                      {d.cites.map((c, i) => (
                        <span key={i} style={{ marginRight: 6 }}>
                          {c}
                        </span>
                      ))}
                    </span>
                  </>
                ) : null}
                {d.tags.length > 0 ? (
                  <span style={{ marginLeft: "auto", display: "flex", gap: 6 }}>
                    {d.tags.map((t) => (
                      <Tag key={t}>#{t}</Tag>
                    ))}
                  </span>
                ) : null}
              </div>
              {d.chain && d.chain.length > 1 ? <SupersedeChain chain={d.chain} /> : null}
            </article>
          ))}
        </div>
      )}

      <DecisionDrawer
        id={openId}
        row={openId ? rowsRaw.find((r) => r.id === openId) ?? null : null}
        onClose={() => setOpenId(null)}
      />
    </div>
  );
}

function DecisionDrawer({
  id,
  row,
  onClose,
}: {
  id: string | null;
  row: DecisionRow | null;
  onClose: () => void;
}) {
  const connKey = useConnKey();
  const { data, isLoading, error } = useQuery({
    queryKey: [connKey, "decision-detail", id ?? ""],
    queryFn: () => api.decisionDetail(id as string),
    enabled: !!id,
  });

  return (
    <DetailDrawer
      open={!!id}
      onClose={onClose}
      title={
        <span>
          <span className="mono" style={{ color: "var(--accent)", marginRight: 8 }}>
            {id}
          </span>
          {row?.title ?? data?.title ?? ""}
        </span>
      }
      subtitle={
        <span>
          {row?.repo ? `${row.repo} · ` : ""}
          {row?.status ?? data?.status ?? ""}
          {row?.occurred_at ? ` · ${row.occurred_at}` : ""}
        </span>
      }
    >
      {(row?.author || row?.source_analysis || row?.supersedes || row?.superseded_by) ? (
        <div className="drawer__meta">
          {row.author ? <span>by <strong>{row.author}</strong></span> : null}
          {row.source_analysis ? (
            <span>from <strong>{row.source_analysis}</strong></span>
          ) : null}
          {row.supersedes ? (
            <span>supersedes <strong>{row.supersedes}</strong></span>
          ) : null}
          {row.superseded_by ? (
            <span>superseded by <strong>{row.superseded_by}</strong></span>
          ) : null}
          {row.tags.length > 0 ? (
            <span>tags: <strong>{row.tags.join(", ")}</strong></span>
          ) : null}
        </div>
      ) : null}
      {error ? (
        <pre className="drawer__markdown" style={{ color: "var(--err, #f88)" }}>
          Failed to load decision body.
        </pre>
      ) : isLoading ? (
        <pre className="drawer__markdown muted">Loading…</pre>
      ) : (
        <pre className="drawer__markdown">
          {data?.body_markdown?.trim()
            ? data.body_markdown
            : row?.rationale ?? "(no body)"}
        </pre>
      )}
    </DetailDrawer>
  );
}

function SupersedeChain({ chain }: { chain: DecisionChainNode[] }) {
  return (
    <div className="supersede-chain">
      {chain.map((c, i) => (
        <span key={c.id} style={{ display: "contents" }}>
          <div
            className={`supersede-node ${
              c.state === "current" ? "is-current" : "is-old"
            }`}
          >
            <span className="mono" style={{ fontSize: 10.5, color: "var(--fg-3)" }}>
              {c.id}
            </span>
            <span style={{ fontSize: 11.5, color: "var(--fg-1)" }}>{c.title}</span>
          </div>
          {i < chain.length - 1 ? (
            <span className="supersede-arrow">
              <Icon name="arrow-right" size={14} />
            </span>
          ) : null}
        </span>
      ))}
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
