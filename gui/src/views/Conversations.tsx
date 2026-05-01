import { useMemo, useState } from "react";
import { useQuery } from "@tanstack/react-query";

import { Tag } from "../atoms/Tag";
import { api, type SessionSummary } from "../lib/api";
import { useConnKey } from "../lib/connections/useConnKey";

/// Conversations view — chat-history lens over `kind=turn` envelopes.
/// The list pane shows one row per session (chat thread); selecting
/// a session reveals the full transcript with paired user prompts +
/// assistant replies. Both halves are captured by the
/// UserPromptSubmit and Stop adapter hooks.
export function ConversationsView() {
  const connKey = useConnKey();
  const [selectedSession, setSelectedSession] = useState<string | null>(null);
  const [repoFilter, setRepoFilter] = useState<string>("");

  const { data: list, isLoading: listLoading, error: listError } = useQuery({
    queryKey: [connKey, "conversations"],
    queryFn: () => api.conversations(),
    refetchInterval: 8_000,
    refetchIntervalInBackground: true,
  });

  const repos = useMemo(() => {
    const s = new Set<string>();
    for (const c of list ?? []) for (const r of c.repos) s.add(r);
    return Array.from(s).sort();
  }, [list]);

  const filtered = useMemo(() => {
    const all = list ?? [];
    if (!repoFilter) return all;
    return all.filter((c) => c.repos.includes(repoFilter));
  }, [list, repoFilter]);

  // Auto-select the most recent conversation when the list loads.
  if (!selectedSession && filtered.length > 0) {
    setSelectedSession(filtered[0].session_id);
  }

  const { data: detail, isLoading: detailLoading } = useQuery({
    queryKey: [connKey, "conversation", selectedSession],
    queryFn: () => api.conversation(selectedSession!),
    enabled: !!selectedSession,
    refetchInterval: 5_000,
  });

  return (
    <div className="view">
      <div className="view__head">
        <div>
          <h1 className="view__title">Conversations</h1>
          <p className="view__subtitle">
            Chat history · paired user prompts + assistant replies per session
          </p>
        </div>
        <div className="view__actions">
          <select
            value={repoFilter}
            onChange={(e) => {
              setRepoFilter(e.target.value);
              setSelectedSession(null);
            }}
            className="btn btn--ghost"
            style={{
              fontFamily: "var(--font-mono)",
              fontSize: 12,
              padding: "5px 10px",
            }}
            title="Filter conversations by project"
          >
            <option value="">All projects ({list?.length ?? 0})</option>
            {repos.map((r) => {
              const c = (list ?? []).filter((c) => c.repos.includes(r)).length;
              return (
                <option key={r} value={r}>
                  {r} ({c})
                </option>
              );
            })}
          </select>
        </div>
      </div>

      {listError ? (
        <Empty msg="cortex-api unreachable. Start it with cargo run -p cortex-api." />
      ) : listLoading ? (
        <Empty msg="Loading conversations…" />
      ) : filtered.length === 0 ? (
        <Empty msg="No chat history captured yet. Conversations populate as the adapter publishes turn envelopes." />
      ) : (
        <div className="conversations-grid">
          <ConversationList
            rows={filtered}
            selected={selectedSession}
            onSelect={setSelectedSession}
          />
          <ConversationDetailPane
            sessionId={selectedSession}
            detail={detail}
            loading={detailLoading}
          />
        </div>
      )}
    </div>
  );
}

function ConversationList({
  rows,
  selected,
  onSelect,
}: {
  rows: NonNullable<Awaited<ReturnType<typeof api.conversations>>>;
  selected: string | null;
  onSelect: (s: string) => void;
}) {
  return (
    <aside className="conv-list">
      {rows.map((c) => (
        <button
          key={c.session_id}
          type="button"
          className={`conv-list__item ${selected === c.session_id ? "is-active" : ""}`}
          onClick={() => onSelect(c.session_id)}
          title={c.session_id}
        >
          <div className="conv-list__title">
            {c.title || "(empty turn)"}
          </div>
          <div className="conv-list__meta">
            <span className="mono">{c.session_id.slice(0, 8)}</span>
            <span>·</span>
            <span>{c.repos[0] ?? "—"}</span>
            <span>·</span>
            <span>{c.turn_count} turn{c.turn_count === 1 ? "" : "s"}</span>
          </div>
          <div className="conv-list__time">{tsRelative(c.last_at_ms)}</div>
        </button>
      ))}
    </aside>
  );
}

function ConversationDetailPane({
  sessionId,
  detail,
  loading,
}: {
  sessionId: string | null;
  detail?: Awaited<ReturnType<typeof api.conversation>>;
  loading: boolean;
}) {
  // Sonnet-generated summary. Disabled by default — analysis costs
  // money and shouldn't run unprompted; user clicks "Analyze with
  // Sonnet" to trigger. The query stays enabled after first click
  // so the cache survives view re-mounts.
  const [analyzeRequested, setAnalyzeRequested] = useState(false);
  const connKey = useConnKey();
  const summaryQ = useQuery({
    queryKey: [connKey, "conversation-summary", sessionId],
    queryFn: () => api.conversationSummary(sessionId!),
    enabled: !!sessionId && analyzeRequested,
    retry: 0,
    staleTime: Infinity,
  });

  if (!sessionId) {
    return <div className="conv-detail conv-detail--empty">Select a conversation</div>;
  }
  if (loading || !detail) {
    return <div className="conv-detail conv-detail--empty">Loading transcript…</div>;
  }
  return (
    <section className="conv-detail">
      <header className="conv-detail__head">
        <span className="mono" style={{ color: "var(--fg-3)", fontSize: 11 }}>
          {detail.session_id}
        </span>
        {detail.repos.map((r) => (
          <Tag key={r} tone="info">
            {r}
          </Tag>
        ))}
        <span className="muted" style={{ marginLeft: "auto", fontSize: 11 }}>
          {detail.turns.length} turn{detail.turns.length === 1 ? "" : "s"}
        </span>
        <button
          type="button"
          className="btn btn--ghost btn--sm"
          onClick={() => setAnalyzeRequested(true)}
          disabled={summaryQ.isFetching}
          title="Run Sonnet to summarise this session and surface its key actions, references, and topics"
          style={{ fontSize: 11, padding: "4px 10px" }}
        >
          {summaryQ.isFetching
            ? "Analyzing…"
            : summaryQ.data
              ? "Re-analyze"
              : "Analyze with Sonnet"}
        </button>
      </header>
      {analyzeRequested ? (
        <SummaryPane
          summary={summaryQ.data}
          loading={summaryQ.isFetching}
          error={summaryQ.error as Error | null}
        />
      ) : null}
      <div className="conv-transcript">
        {detail.turns.length === 0 ? (
          <Empty msg="No turns recorded under this session." />
        ) : (
          detail.turns.map((t) => (
            <article key={t.turn_id} className="conv-turn">
              {t.user_message ? (
                <div className="conv-msg conv-msg--user">
                  <div className="conv-msg__role">user</div>
                  <pre className="conv-msg__body">{t.user_message}</pre>
                </div>
              ) : null}
              {t.assistant_message ? (
                <div className="conv-msg conv-msg--assistant">
                  <div className="conv-msg__role">assistant</div>
                  <pre className="conv-msg__body">{t.assistant_message}</pre>
                </div>
              ) : (
                <div className="conv-msg conv-msg--pending muted" style={{ fontSize: 11 }}>
                  (assistant reply not captured — turn still open or
                  pre-Stop-hook archive)
                </div>
              )}
            </article>
          ))
        )}
      </div>
    </section>
  );
}

function SummaryPane({
  summary,
  loading,
  error,
}: {
  summary: SessionSummary | undefined;
  loading: boolean;
  error: Error | null;
}) {
  if (loading) {
    return (
      <div className="conv-summary conv-summary--loading">
        Asking Sonnet to summarise this session… (typical 10–40 s)
      </div>
    );
  }
  if (error) {
    return (
      <div className="conv-summary conv-summary--error">
        <strong>Summary unavailable.</strong>{" "}
        Likely the local <code>claude</code> CLI isn't on PATH or returned malformed JSON.
        Check the cortex-api log for the exact reason.
      </div>
    );
  }
  if (!summary) return null;
  return (
    <div className="conv-summary">
      <div className="conv-summary__head">
        <span className="conv-summary__badge">Sonnet · sonnet-4-6</span>
        {summary.topics.length > 0 ? (
          <span className="conv-summary__topics">
            {summary.topics.map((t) => (
              <Tag key={t}>#{t}</Tag>
            ))}
          </span>
        ) : null}
      </div>
      <p className="conv-summary__body">{summary.summary}</p>
      {summary.key_actions.length > 0 ? (
        <>
          <div className="conv-summary__section-label">Key actions</div>
          <ul className="conv-summary__list">
            {summary.key_actions.map((a, i) => (
              <li key={i}>{a}</li>
            ))}
          </ul>
        </>
      ) : null}
      {summary.references.length > 0 ? (
        <>
          <div className="conv-summary__section-label">References</div>
          <div className="conv-summary__refs">
            {summary.references.map((r, i) => (
              <Tag key={i} tone="accent">
                {r}
              </Tag>
            ))}
          </div>
        </>
      ) : null}
    </div>
  );
}

function tsRelative(ms: number): string {
  if (!ms) return "";
  const diff = Date.now() - ms;
  const secs = Math.max(0, Math.floor(diff / 1000));
  if (secs < 60) return `${secs}s ago`;
  const mins = Math.floor(secs / 60);
  if (mins < 60) return `${mins}m ago`;
  const hours = Math.floor(mins / 60);
  if (hours < 24) return `${hours}h ago`;
  return `${Math.floor(hours / 24)}d ago`;
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
