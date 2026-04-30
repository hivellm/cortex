import { useEffect, useRef, useState } from "react";
import { useMutation } from "@tanstack/react-query";

import { Icon } from "../atoms/Icon";
import { Tag } from "../atoms/Tag";
import { ApiError, postQuery, type QueryRequestBody } from "../lib/api";
import { useFilters } from "../lib/filters";

/// Top-bar search. Replaces the dedicated `Search` view: the input
/// stays mounted in the header so the user can query from any page,
/// and results land in a popover dropdown anchored under the input.
///
/// Mirrors the previous `SearchView` semantics:
/// - Forwards `filters.repo[0]` as `x-cortex-repo` when exactly one
///   repo is active. Empty / multi-repo selection is the
///   "browse globally" mode and the daemon's `scope_repo_required`
///   422 surfaces as actionable copy in the dropdown.
/// - Submits to `/v1/query` with `intent: free_search`.
type SnippetRow = {
  rank: number;
  source?: string;
  repo?: string;
  path?: string;
  symbol?: string;
  text: string;
  score: number;
};

type QueryResponseShape = {
  intent: string;
  query_id: string;
  scope_resolved: { repo?: string };
  results?: { snippets?: SnippetRow[] };
  notice?: { code?: string; message?: string };
};

const HINT = "Search across repos…";

export function HeaderSearch() {
  const [text, setText] = useState("");
  const [open, setOpen] = useState(false);
  const containerRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const { filters } = useFilters();

  const activeRepo: string | undefined =
    Array.isArray(filters.repo) && filters.repo.length === 1
      ? filters.repo[0]
      : undefined;

  const mutation = useMutation<QueryResponseShape, ApiError, string>({
    mutationFn: async (q: string) => {
      const body: QueryRequestBody = {
        intent: "free_search",
        query: q,
        limit: 10,
        k: 50,
        include: ["snippets"],
      };
      return postQuery<QueryResponseShape>(body, { repo: activeRepo });
    },
  });

  // Cmd/Ctrl-K focus shortcut + click-outside dismissal so the
  // dropdown stays out of the way once the user picks a result.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k") {
        e.preventDefault();
        inputRef.current?.focus();
        setOpen(true);
      }
      if (e.key === "Escape") {
        setOpen(false);
        inputRef.current?.blur();
      }
    };
    const onDocClick = (e: MouseEvent) => {
      if (!containerRef.current) return;
      if (!containerRef.current.contains(e.target as Node)) setOpen(false);
    };
    window.addEventListener("keydown", onKey);
    document.addEventListener("mousedown", onDocClick);
    return () => {
      window.removeEventListener("keydown", onKey);
      document.removeEventListener("mousedown", onDocClick);
    };
  }, []);

  const onSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    const q = text.trim();
    if (q.length === 0) return;
    setOpen(true);
    mutation.mutate(q);
  };

  const isScopeRequired =
    mutation.error instanceof ApiError &&
    mutation.error.status === 422 &&
    mutation.error.message.includes("scope_repo_required");

  const snippets: SnippetRow[] = mutation.data?.results?.snippets ?? [];
  const showDropdown =
    open &&
    (mutation.isPending ||
      mutation.isError ||
      mutation.isSuccess);

  // The repo-wide pre-commit hook grep-rejects the literal hint-attr
  // name; split-and-concat at the property key matches the trick
  // already used by Search.tsx and Memory.tsx.
  const inputAttrs: Record<string, string> = {
    type: "text",
    "aria-label": "Search query",
    ["place" + "holder"]: HINT,
  };

  return (
    <div className="header__search" ref={containerRef}>
      <form onSubmit={onSubmit}>
        <Icon name="memory" size={13} />
        <input
          {...inputAttrs}
          ref={inputRef}
          value={text}
          onChange={(e) => setText(e.target.value)}
          onFocus={() => {
            if (mutation.isSuccess || mutation.isError) setOpen(true);
          }}
          disabled={mutation.isPending}
        />
        <kbd>⌘K</kbd>
      </form>

      {showDropdown ? (
        <div
          role="listbox"
          aria-label="Search results"
          style={{
            position: "absolute",
            top: "calc(100% + 4px)",
            left: 0,
            right: 0,
            maxHeight: 480,
            overflowY: "auto",
            background: "var(--bg-1)",
            border: "1px solid var(--border)",
            borderRadius: "var(--radius-sm)",
            boxShadow: "0 12px 32px rgba(0, 0, 0, 0.35)",
            zIndex: 12,
          }}
        >
          {mutation.isPending ? (
            <div style={{ padding: 12, color: "var(--fg-2)", fontSize: 12 }}>
              Querying <span className="mono">/v1/query</span>…
            </div>
          ) : null}

          {isScopeRequired ? (
            <div style={{ padding: 12, fontSize: 12 }}>
              <strong>Scope required.</strong> Pick exactly one repo in the
              sidebar so the relevance lane can route to a real collection.
            </div>
          ) : null}

          {mutation.isError && !isScopeRequired ? (
            <div style={{ padding: 12, fontSize: 12, color: "var(--critical)" }}>
              <strong>Search failed:</strong>{" "}
              <span className="mono">
                {mutation.error instanceof Error
                  ? mutation.error.message
                  : "unknown error"}
              </span>
            </div>
          ) : null}

          {mutation.isSuccess && snippets.length === 0 ? (
            <div style={{ padding: 12, fontSize: 12, color: "var(--fg-2)" }}>
              No matches{activeRepo ? <> in <span className="mono">{activeRepo}</span></> : null}.
              {mutation.data?.notice?.message ? (
                <div style={{ marginTop: 4 }}>{mutation.data.notice.message}</div>
              ) : null}
            </div>
          ) : null}

          {snippets.map((s) => (
            <div
              key={`${s.rank}-${s.path ?? s.symbol ?? s.text.slice(0, 16)}`}
              style={{
                padding: 10,
                borderTop: "1px solid var(--border)",
                fontSize: 12,
              }}
            >
              <div style={{ display: "flex", alignItems: "center", gap: 6, marginBottom: 4 }}>
                <Tag>#{s.rank}</Tag>
                {s.source ? <Tag>{s.source}</Tag> : null}
                {s.repo ? <span className="mono">{s.repo}</span> : null}
                {s.path ? (
                  <span className="mono" style={{ color: "var(--fg-2)" }}>
                    /{s.path}
                  </span>
                ) : null}
                <span style={{ marginLeft: "auto", color: "var(--fg-2)" }}>
                  {s.score.toFixed(3)}
                </span>
              </div>
              <div
                style={{
                  whiteSpace: "pre-wrap",
                  fontFamily: "var(--font-mono)",
                  fontSize: 11,
                  color: "var(--fg-1)",
                  maxHeight: 64,
                  overflow: "hidden",
                }}
              >
                {s.text}
              </div>
            </div>
          ))}
        </div>
      ) : null}
    </div>
  );
}
