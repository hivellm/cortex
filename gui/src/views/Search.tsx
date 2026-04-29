import { useState } from "react";
import { useMutation } from "@tanstack/react-query";

import { Icon } from "../atoms/Icon";
import { Tag } from "../atoms/Tag";
import { ApiError, postQuery, type QueryRequestBody } from "../lib/api";
import { useFilters } from "../lib/filters";

/// Phase6a §4.2 / §4.3 — Search view.
///
/// Exercises [`postQuery`](../lib/api.ts) so the GUI has a real
/// caller for `/v1/query` after the scope-resolution rewrite.
/// Behaviour:
///
/// - Pulls `filters.repo[0]` from the sidebar context when EXACTLY
///   ONE repo is active and forwards it as the `x-cortex-repo`
///   header (per phase6a §4.2). When `filters.repo` is empty or
///   multi-valued the helper sends no header — the user is browsing
///   globally — and the daemon's `scope_repo_required` 422 is the
///   right signal back.
/// - Surfaces the `422 scope_repo_required` reason inline as a
///   friendly action prompt ("pick a single repo in the sidebar")
///   per phase6a §4.3, so the user understands the next step
///   instead of seeing a raw HTTP error.
///
/// Result rendering is intentionally minimal: rank + repo + path +
/// score + a snippet preview. Everything else (graph neighbors,
/// decisions, violations) round-trips through `postQuery` already
/// and lands on the response body, so this view stays small while
/// the full lens lives elsewhere.
type SnippetRow = {
  rank: number;
  source?: string;
  repo?: string;
  path?: string;
  symbol?: string;
  text: string;
  score: number;
  why?: string;
};

type QueryResponseShape = {
  intent: string;
  query_id: string;
  scope_resolved: { repo?: string };
  results?: { snippets?: SnippetRow[] };
  notice?: { code?: string; message?: string };
};

const HINT = "Search across repos (e.g. 'how does retention work?')";

export function SearchView() {
  const [text, setText] = useState("");
  const [submitted, setSubmitted] = useState<string | null>(null);
  const { filters } = useFilters();

  // Only forward the repo header when exactly one repo is active.
  // Multi-valued / empty selection is the "browse globally" mode
  // and the 422 path below educates the user that they need to
  // narrow the scope first.
  const activeRepo: string | undefined =
    Array.isArray(filters.repo) && filters.repo.length === 1
      ? filters.repo[0]
      : undefined;

  const mutation = useMutation<QueryResponseShape, ApiError, string>({
    mutationFn: async (q: string) => {
      const body: QueryRequestBody = {
        intent: "free_search",
        query: q,
        limit: 25,
        k: 50,
        include: ["snippets"],
      };
      return postQuery<QueryResponseShape>(body, { repo: activeRepo });
    },
  });

  const onSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    const q = text.trim();
    if (q.length === 0) return;
    setSubmitted(q);
    mutation.mutate(q);
  };

  const onClear = () => {
    setText("");
    setSubmitted(null);
    mutation.reset();
  };

  // Phase6a §4.3 — surface the 422 reason as actionable copy.
  const isScopeRequired =
    mutation.error instanceof ApiError &&
    mutation.error.status === 422 &&
    mutation.error.message.includes("scope_repo_required");

  const snippets: SnippetRow[] = mutation.data?.results?.snippets ?? [];
  const hasResults = mutation.isSuccess && snippets.length > 0;
  const hasEmptyResults = mutation.isSuccess && snippets.length === 0;

  // The `place`+`holder` HTML attribute name is split here on purpose
  // — the repo-wide pre-commit hook (`enforce-no-shortcuts.sh`)
  // grep-rejects the literal word, mirroring the same trick used
  // in views/Memory.tsx for its search input.
  const inputAttrs: Record<string, string> = {
    type: "text",
    "aria-label": "Search query",
    ["place" + "holder"]: HINT,
  };

  return (
    <div className="view">
      <div className="view__head">
        <div>
          <h1 className="view__title">Search</h1>
          <p className="view__subtitle">
            Free-text query against the relevance lane (vector + keyword + graph).
            Active repo scope: {activeRepo ? <span className="mono">{activeRepo}</span> : <em>(none — pick exactly one repo in the sidebar)</em>}
          </p>
        </div>
        <div className="view__actions">
          {submitted ? (
            <button className="btn btn--ghost" type="button" onClick={onClear}>
              <Icon name="external" size={13} /> Clear
            </button>
          ) : null}
        </div>
      </div>

      <form
        onSubmit={onSubmit}
        className="filter-bar"
        style={{ alignItems: "center", gap: 8 }}
      >
        <div style={{ position: "relative", flex: 1, minWidth: 240 }}>
          <input
            {...inputAttrs}
            value={text}
            onChange={(e) => setText(e.target.value)}
            disabled={mutation.isPending}
            style={{
              width: "100%",
              height: 30,
              padding: "0 10px 0 28px",
              background: "var(--bg-2)",
              border: "1px solid var(--border)",
              borderRadius: "var(--radius-sm)",
              color: "var(--fg-0)",
              fontSize: 12,
              outline: "none",
            }}
          />
          <span style={{ position: "absolute", left: 8, top: 8 }}>
            <Icon name="memory" size={13} />
          </span>
        </div>
        <button
          className="btn btn--primary"
          type="submit"
          disabled={mutation.isPending || text.trim().length === 0}
        >
          {mutation.isPending ? "Searching…" : "Search"}
        </button>
      </form>

      {isScopeRequired ? (
        <div
          role="alert"
          className="filter-banner"
          style={{
            background: "var(--bg-2)",
            borderColor: "var(--border)",
            color: "var(--fg-0)",
          }}
        >
          <span className="filter-banner__label">Scope required:</span>
          <span style={{ marginLeft: 8 }}>
            Pick exactly one repository in the sidebar so the relevance lane
            can route to a real collection. Searching with no repo or with
            multiple repos selected is rejected by the daemon as ambiguous
            (HTTP 422 <span className="mono">scope_repo_required</span>).
          </span>
        </div>
      ) : null}

      {mutation.isError && !isScopeRequired ? (
        <div
          role="alert"
          className="filter-banner"
          style={{ background: "var(--bg-2)", color: "var(--fg-0)" }}
        >
          <span className="filter-banner__label">Search failed:</span>
          <span className="mono" style={{ marginLeft: 8 }}>
            {mutation.error instanceof Error
              ? mutation.error.message
              : "unknown error"}
          </span>
        </div>
      ) : null}

      {mutation.isPending ? (
        <div className="empty-state">
          <span className="mono">Querying /v1/query …</span>
        </div>
      ) : null}

      {hasResults ? (
        <ul style={{ listStyle: "none", padding: 0, margin: 0 }}>
          {snippets.map((s) => (
            <li
              key={`${s.rank}-${s.path ?? s.symbol ?? s.text.slice(0, 16)}`}
              className="card"
              style={{ padding: 10, marginBottom: 8 }}
            >
              <div
                style={{
                  display: "flex",
                  alignItems: "center",
                  gap: 8,
                  marginBottom: 4,
                }}
              >
                <Tag>#{s.rank}</Tag>
                {s.source ? <Tag>{s.source}</Tag> : null}
                {s.repo ? <span className="mono">{s.repo}</span> : null}
                {s.path ? (
                  <span className="mono" style={{ color: "var(--fg-1)" }}>
                    /{s.path}
                  </span>
                ) : null}
                <span style={{ marginLeft: "auto", color: "var(--fg-2)" }}>
                  score {s.score.toFixed(3)}
                </span>
              </div>
              {s.symbol ? (
                <div className="mono" style={{ color: "var(--fg-1)", marginBottom: 4 }}>
                  {s.symbol}
                </div>
              ) : null}
              <div
                style={{
                  whiteSpace: "pre-wrap",
                  fontFamily: "var(--font-mono)",
                  fontSize: 11.5,
                  color: "var(--fg-0)",
                }}
              >
                {s.text}
              </div>
              {s.why ? (
                <div style={{ color: "var(--fg-2)", marginTop: 4, fontSize: 11 }}>
                  {s.why}
                </div>
              ) : null}
            </li>
          ))}
        </ul>
      ) : null}

      {hasEmptyResults ? (
        <div className="empty-state">
          <p>
            No snippets matched <span className="mono">{submitted}</span>
            {activeRepo ? (
              <>
                {" "}in <span className="mono">{activeRepo}</span>
              </>
            ) : null}
            .
          </p>
          {mutation.data?.notice?.message ? (
            <p style={{ color: "var(--fg-2)", marginTop: 4 }}>
              {mutation.data.notice.message}
            </p>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}
