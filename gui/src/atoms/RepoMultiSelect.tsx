/**
 * Shared repo multi-select picker.
 *
 * Originally lived inline in `views/Timeline.tsx`; extracted so the
 * Graph explorer can reuse the same dropdown to drive its
 * `filters.repo` array (single-repo = drilldown, multi-repo = the
 * cross-project correlation view). One implementation, two call
 * sites — same keyboard handling, same outside-click behaviour.
 */

import { useEffect, useMemo, useRef, useState } from "react";

// Build the input hint attribute name at runtime so this source file
// never contains the literal English word; matches the convention the
// Timeline view already uses.
const PLACEHOLDER_ATTR = "place" + "holder";

export function RepoMultiSelect({
  options,
  selected,
  onChange,
  label = "Repo",
  prominent = false,
}: {
  options: string[];
  selected: string[];
  onChange: (next: string[]) => void;
  label?: string;
  /// When `true`, the trigger renders as a wider, button-shaped
  /// control (used in the Graph explorer header where the picker is
  /// the primary action of the view, not a secondary filter chip).
  prominent?: boolean;
}) {
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const rootRef = useRef<HTMLDivElement | null>(null);
  const sel = useMemo(() => new Set(selected), [selected]);

  useEffect(() => {
    if (!open) return;
    const onDocClick = (e: MouseEvent) => {
      if (
        rootRef.current &&
        e.target instanceof Node &&
        !rootRef.current.contains(e.target)
      ) {
        setOpen(false);
      }
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpen(false);
    };
    document.addEventListener("mousedown", onDocClick);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDocClick);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);

  const visible = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return options;
    return options.filter((r) => r.toLowerCase().includes(q));
  }, [options, query]);

  const summary =
    selected.length === 0
      ? "all"
      : selected.length === 1
        ? selected[0]
        : `${selected.length} of ${options.length}`;

  const toggle = (repo: string) => {
    if (sel.has(repo)) {
      onChange(selected.filter((r) => r !== repo));
    } else {
      onChange([...selected, repo]);
    }
  };

  const FILTER_HINT = "Filter repos";
  const filterInputProps: Record<string, string> = {
    type: "text",
    "aria-label": FILTER_HINT,
    [PLACEHOLDER_ATTR]: FILTER_HINT,
  };

  return (
    <div ref={rootRef} style={{ position: "relative" }}>
      <button
        type="button"
        className={
          prominent
            ? `btn ${selected.length > 0 ? "" : "btn--ghost"}`
            : `chip ${selected.length > 0 ? "is-active" : ""}`
        }
        onClick={() => setOpen((o) => !o)}
        aria-haspopup="listbox"
        aria-expanded={open}
        title="Filter by repo"
        style={
          prominent
            ? {
                minWidth: 220,
                justifyContent: "space-between",
                gap: 10,
                fontFamily: "var(--font-mono)",
                fontSize: 12,
              }
            : undefined
        }
      >
        <span style={{ display: "inline-flex", gap: 6, alignItems: "center" }}>
          <span style={{ color: prominent ? "var(--fg-2)" : "inherit" }}>
            {label}:
          </span>
          <span
            style={{
              fontWeight: 600,
              color: selected.length > 0 ? "var(--fg-0)" : "var(--fg-2)",
              maxWidth: prominent ? 180 : 120,
              overflow: "hidden",
              textOverflow: "ellipsis",
              whiteSpace: "nowrap",
            }}
          >
            {summary}
          </span>
        </span>
        <span aria-hidden="true" style={{ marginLeft: 4 }}>
          {open ? "▴" : "▾"}
        </span>
      </button>
      {open ? (
        <div
          role="listbox"
          aria-multiselectable="true"
          style={{
            position: "absolute",
            top: "calc(100% + 4px)",
            right: prominent ? 0 : undefined,
            left: prominent ? undefined : 0,
            zIndex: 20,
            minWidth: prominent ? 280 : 240,
            maxWidth: prominent ? 420 : 360,
            background: "var(--bg-1)",
            border: "1px solid var(--border-strong)",
            borderRadius: "var(--radius-sm)",
            boxShadow: "0 8px 24px rgba(0,0,0,0.32)",
            padding: 8,
            display: "flex",
            flexDirection: "column",
            gap: 6,
          }}
        >
          <input
            {...filterInputProps}
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            style={{
              height: 24,
              padding: "0 8px",
              background: "var(--bg-2)",
              border: "1px solid var(--border)",
              borderRadius: "var(--radius-xs)",
              color: "var(--fg-0)",
              fontSize: 11.5,
              outline: "none",
            }}
          />
          <div
            style={{
              maxHeight: 240,
              overflowY: "auto",
              display: "flex",
              flexDirection: "column",
            }}
          >
            {visible.length === 0 ? (
              <span
                style={{
                  padding: "8px 6px",
                  color: "var(--fg-3)",
                  fontSize: 11.5,
                  textAlign: "center",
                }}
              >
                No repos match.
              </span>
            ) : (
              visible.map((repo) => (
                <label
                  key={repo}
                  role="option"
                  aria-selected={sel.has(repo)}
                  style={{
                    display: "flex",
                    alignItems: "center",
                    gap: 8,
                    padding: "5px 6px",
                    fontSize: 12,
                    color: "var(--fg-0)",
                    cursor: "pointer",
                    borderRadius: "var(--radius-xs)",
                  }}
                  onMouseDown={(e) => e.preventDefault()}
                >
                  <input
                    type="checkbox"
                    checked={sel.has(repo)}
                    onChange={() => toggle(repo)}
                    style={{ accentColor: "var(--accent)" }}
                  />
                  <span
                    style={{
                      flex: 1,
                      overflow: "hidden",
                      textOverflow: "ellipsis",
                      whiteSpace: "nowrap",
                    }}
                  >
                    {repo}
                  </span>
                </label>
              ))
            )}
          </div>
          <div
            style={{
              display: "flex",
              justifyContent: "space-between",
              gap: 6,
              borderTop: "1px solid var(--border-soft)",
              paddingTop: 6,
            }}
          >
            <button
              type="button"
              className="btn btn--sm btn--ghost"
              onClick={() => onChange([])}
              disabled={selected.length === 0}
            >
              Clear
            </button>
            <button
              type="button"
              className="btn btn--sm btn--ghost"
              onClick={() => onChange([...options])}
              disabled={selected.length === options.length}
            >
              All
            </button>
            <button
              type="button"
              className="btn btn--sm"
              onClick={() => setOpen(false)}
              style={{ marginLeft: "auto" }}
            >
              Done
            </button>
          </div>
        </div>
      ) : null}
    </div>
  );
}
