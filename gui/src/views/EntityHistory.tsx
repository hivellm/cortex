import { useEffect, useState } from "react";
import { useQuery } from "@tanstack/react-query";

import { Tag } from "../atoms/Tag";
import {
  api,
  type BitemporalTimelineEvent,
  type SupersessionChainNode,
} from "../lib/api";
import { useConnKey } from "../lib/connections/useConnKey";

function fmtUnix(unix: number | undefined | null): string {
  if (!unix) return "—";
  return new Date(unix * 1000).toISOString().slice(0, 10);
}

export function EntityHistoryView() {
  const [entityId, setEntityId] = useState("");
  const [asOf, setAsOf] = useState("");
  // Committed query params — only update on explicit "Load" to avoid
  // thrashing the network on every keystroke.
  const [queryId, setQueryId] = useState("");
  const [queryAsOf, setQueryAsOf] = useState("");

  // Listen for cross-view open-entity-history events dispatched by
  // the BitemporalTimeline view when the user clicks an entity_id.
  useEffect(() => {
    function onOpenEntity(e: Event) {
      const id = (e as CustomEvent<{ id: string }>).detail?.id;
      if (typeof id === "string" && id.length > 0) {
        setEntityId(id);
        setQueryId(id);
        setQueryAsOf("");
        setAsOf("");
      }
    }
    window.addEventListener("cortex:open-entity-history", onOpenEntity);
    return () => window.removeEventListener("cortex:open-entity-history", onOpenEntity);
  }, []);

  const connKey = useConnKey();

  const historyQ = useQuery({
    queryKey: [connKey, "entity-history", queryId, queryAsOf],
    queryFn: () => api.entityHistory(queryId, queryAsOf || undefined),
    refetchInterval: 300_000,
    enabled: queryId.trim().length > 0,
  });

  const supersessionQ = useQuery({
    queryKey: [connKey, "entity-supersession", queryId],
    queryFn: () => api.entitySupersession(queryId),
    refetchInterval: 300_000,
    enabled: queryId.trim().length > 0,
  });

  const events: BitemporalTimelineEvent[] = historyQ.data?.events ?? [];
  const lineage = supersessionQ.data?.lineage;

  const onLoad = () => {
    setQueryId(entityId.trim());
    setQueryAsOf(asOf);
  };

  return (
    <div className="view">
      <div className="view__head">
        <div>
          <h1 className="view__title">Entity History</h1>
          <p className="view__subtitle">
            Bitemporal trail + supersession lineage ·{" "}
            <span className="mono">/v1/entity/{"{id}"}/history</span>
          </p>
        </div>
      </div>

      <div
        style={{
          display: "flex",
          alignItems: "center",
          gap: 8,
          padding: "8px 0",
          borderBottom: "1px solid var(--border)",
          marginBottom: 16,
          flexWrap: "wrap",
        }}
      >
        <span style={{ color: "var(--fg-3)", fontSize: 11 }}>Entity ID</span>
        <input
          type="text"
          value={entityId}
          onChange={(e) => setEntityId(e.target.value)}
          aria-label="Entity ID"
          style={{
            height: 28,
            padding: "0 10px",
            background: "var(--bg-2)",
            border: "1px solid var(--border)",
            borderRadius: "var(--radius-sm)",
            color: "var(--fg-0)",
            fontSize: 11.5,
            width: 260,
          }}
          onKeyDown={(e) => {
            if (e.key === "Enter") onLoad();
          }}
        />
        <span style={{ color: "var(--fg-3)", fontSize: 11 }}>As of</span>
        <input
          type="date"
          value={asOf}
          onChange={(e) => setAsOf(e.target.value)}
          aria-label="As of date"
          style={{
            height: 28,
            padding: "0 8px",
            background: "var(--bg-2)",
            border: "1px solid var(--border)",
            borderRadius: "var(--radius-sm)",
            color: "var(--fg-0)",
            fontSize: 11.5,
          }}
        />
        <button
          type="button"
          className="btn btn--primary"
          onClick={onLoad}
          disabled={entityId.trim().length === 0}
        >
          Load
        </button>
      </div>

      {queryId.trim().length === 0 ? (
        <Empty msg="Enter an entity ID above and press Load to fetch its bitemporal history." />
      ) : historyQ.error ? (
        <Empty msg="cortex-api unreachable. Start it with cargo run -p cortex-api." />
      ) : historyQ.isLoading ? (
        <Empty msg="Loading entity history…" />
      ) : (
        <div>
          {lineage ? (
            <div
              className="card"
              style={{ padding: 16, marginBottom: 20 }}
            >
              <h3
                style={{
                  fontSize: 11,
                  textTransform: "uppercase",
                  letterSpacing: "0.05em",
                  color: "var(--fg-3)",
                  marginTop: 0,
                  marginBottom: 10,
                }}
              >
                Supersession Lineage
              </h3>
              <div style={{ display: "flex", alignItems: "center", gap: 8, flexWrap: "wrap" }}>
                {(lineage.predecessors ?? []).map((p, i) => (
                  <ChainNode key={p.id ?? String(i)} node={p} isCurrent={false} />
                ))}
                {(lineage.predecessors ?? []).length > 0 ? (
                  <span style={{ color: "var(--fg-3)", fontSize: 12 }}>→</span>
                ) : null}
                <div
                  style={{
                    padding: "4px 10px",
                    background: "var(--accent)",
                    borderRadius: "var(--radius-sm)",
                    color: "var(--bg-1)",
                    fontSize: 11.5,
                    fontFamily: "var(--font-mono)",
                  }}
                  title="Current entity"
                >
                  {lineage.label ?? lineage.id ?? queryId}
                </div>
                {(lineage.successors ?? []).length > 0 ? (
                  <span style={{ color: "var(--fg-3)", fontSize: 12 }}>→</span>
                ) : null}
                {(lineage.successors ?? []).map((s, i) => (
                  <ChainNode key={s.id ?? String(i)} node={s} isCurrent={false} />
                ))}
              </div>
            </div>
          ) : null}

          {events.length === 0 ? (
            <Empty msg="No history events found for this entity." />
          ) : (
            <div className="decision-list">
              {events.map((ev, i) => (
                <HistoryRow key={ev.id ?? String(i)} ev={ev} />
              ))}
            </div>
          )}
        </div>
      )}
    </div>
  );
}

function HistoryRow({ ev }: { ev: BitemporalTimelineEvent }) {
  return (
    <article
      className="decision"
      style={{ display: "flex", flexDirection: "column", gap: 4 }}
    >
      <div className="decision__head">
        {ev.kind ? <Tag tone="info">{ev.kind}</Tag> : null}
        <span className="decision__title">{ev.title ?? "(no title)"}</span>
        <span
          className="muted mono"
          style={{ marginLeft: "auto", fontSize: 10.5 }}
        >
          valid {fmtUnix(ev.valid_from_unix)} · recorded {fmtUnix(ev.recorded_at_unix)}
        </span>
      </div>
      {ev.summary ? (
        <p style={{ fontSize: 11.5, color: "var(--fg-2)", margin: 0 }}>
          {ev.summary}
        </p>
      ) : null}
    </article>
  );
}

function ChainNode({
  node,
  isCurrent,
}: {
  node: SupersessionChainNode;
  isCurrent: boolean;
}) {
  return (
    <div
      style={{
        padding: "4px 10px",
        background: isCurrent ? "var(--accent)" : "var(--bg-3)",
        borderRadius: "var(--radius-sm)",
        color: isCurrent ? "var(--bg-1)" : "var(--fg-1)",
        fontSize: 11.5,
        fontFamily: "var(--font-mono)",
      }}
    >
      {node.id ?? "(unknown)"}
      {node.kind ? (
        <span style={{ fontSize: 10, color: isCurrent ? "var(--bg-2)" : "var(--fg-3)", marginLeft: 4 }}>
          {node.kind}
        </span>
      ) : null}
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
