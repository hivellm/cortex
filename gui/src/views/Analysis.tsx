import { useQuery } from "@tanstack/react-query";

import { Tag } from "../atoms/Tag";
import { api } from "../lib/api";
import { useConnKey } from "../lib/connections/useConnKey";

export function AnalysisView() {
  const connKey = useConnKey();
  const { data, isLoading, error } = useQuery({
    queryKey: [connKey, "analyses"],
    queryFn: () => api.analyses(),
    refetchInterval: 15_000,
  });
  const rows = data ?? [];
  const concluded = rows.filter((a) => a.status === "concluded").length;
  const promoted = rows.filter((a) => !!a.decision_id).length;
  const totalRounds = rows.reduce((s, a) => s + (a.rounds ?? 0), 0);
  const avgRounds = rows.length ? (totalRounds / rows.length).toFixed(1) : "0";

  // An imported audit (phase4e) carries a `source_path` and is keyed
  // by a bootstrap event id; spec-15 deep-analyses do not. The two
  // shapes share the same surface but the imports are pure documents
  // (no panel / judge / rounds / duration), so we relabel the empty-
  // state and stat copy depending on what's present.
  const imports = rows.filter((a) => !!a.source_path).length;
  const debates = rows.length - imports;
  const subtitleSegments: string[] = [];
  if (imports > 0) {
    subtitleSegments.push(`${imports} imported audit${imports === 1 ? "" : "s"} (docs/analysis/)`);
  }
  if (debates > 0) {
    subtitleSegments.push(`${debates} spec-15 debate${debates === 1 ? "" : "s"}`);
  }
  const subtitleSummary =
    subtitleSegments.length > 0
      ? subtitleSegments.join(" · ")
      : "kind=analysis envelopes — bootstrap imports + spec-15 debates land here";

  return (
    <div className="view">
      <div className="view__head">
        <div>
          <h1 className="view__title">Analysis library</h1>
          <p className="view__subtitle">{subtitleSummary}</p>
        </div>
      </div>

      <div className="stats-grid" style={{ gridTemplateColumns: "repeat(3, 1fr)" }}>
        <Stat label="Captured" value={String(rows.length)} sub={`${concluded} concluded · ${imports} imports`} />
        <Stat label="Promoted" value={String(promoted)} sub="linked to a decision" />
        <Stat label="Avg rounds" value={avgRounds} sub="across captured analyses" />
      </div>

      {error ? (
        <Empty msg="cortex-api unreachable." />
      ) : isLoading ? (
        <Empty msg="Loading analyses…" />
      ) : rows.length === 0 ? (
        <Empty msg="No analyses captured yet. Run cortex-bootstrap to ingest docs/analysis/**, or wait for spec-15 deep-analysis envelopes." />
      ) : (
        rows.map((a) => {
          const isImport = !!a.source_path;
          // Older indexed envelopes (pre-fix) wrote the whole markdown
          // status sentence into the field. Sanitize at render so the
          // badge never balloons into a paragraph regardless of what
          // upstream produced.
          const statusBadge = badgeToken(a.status) || "draft";
          return (
            <article key={a.id} className="analysis-card">
              <div>
                <div style={{ display: "flex", alignItems: "center", gap: 10, marginBottom: 4, flexWrap: "wrap" }}>
                  <span className="mono" style={{ fontSize: 11, color: "var(--accent)", fontWeight: 600 }}>
                    {a.id}
                  </span>
                  <Tag tone={statusBadge === "concluded" ? "ok" : "default"}>{statusBadge}</Tag>
                  {a.repo ? <Tag tone="default">{a.repo}</Tag> : null}
                  {isImport ? <Tag tone="default">imported</Tag> : null}
                  <span className="mono" style={{ fontSize: 10.5, color: "var(--fg-3)", marginLeft: "auto" }}>
                    {a.occurred_at}
                  </span>
                </div>
                <h3 className="analysis-card__title">{a.title}</h3>
                {isImport ? (
                  <div
                    className="mono"
                    style={{ fontSize: 11, color: "var(--fg-3)", marginBottom: 6 }}
                    title={a.source_path}
                  >
                    {a.source_path}
                  </div>
                ) : (
                  <div
                    style={{
                      display: "flex",
                      gap: 10,
                      fontSize: 11.5,
                      color: "var(--fg-3)",
                      fontFamily: "var(--font-mono)",
                    }}
                  >
                    <span>{a.rounds} rounds</span>
                    <span>·</span>
                    <span>{a.duration_s}s</span>
                    <span>·</span>
                    <span>
                      judge: <span style={{ color: "var(--fg-1)" }}>{a.judge || "—"}</span>
                    </span>
                    {a.decision_id ? (
                      <>
                        <span>·</span>
                        <span>
                          → <span style={{ color: "var(--accent)" }}>{a.decision_id}</span>
                        </span>
                      </>
                    ) : null}
                  </div>
                )}
                <div className={isImport ? "analysis-card__summary" : "analysis-card__verdict"}>
                  {isImport ? excerptFromMarkdown(a.verdict) : a.verdict}
                </div>
              </div>
              <div className="analysis-panel">
                <div style={{ color: "var(--fg-3)", fontSize: 10, textTransform: "uppercase", letterSpacing: "0.08em" }}>
                  {isImport ? "Source" : "Panel"}
                </div>
                {isImport ? (
                  <div className="muted" style={{ fontSize: 11 }}>
                    bootstrap import — no debate panel
                  </div>
                ) : a.panel.length === 0 ? (
                  <div className="muted" style={{ fontSize: 11 }}>
                    no panelists recorded
                  </div>
                ) : (
                  a.panel.map((p) => (
                    <div key={p} className="panelist-row">
                      <span className="panelist-dot" />
                      <span style={{ color: "var(--fg-1)" }}>{p}</span>
                    </div>
                  ))
                )}
              </div>
            </article>
          );
        })
      )}
    </div>
  );
}

/**
 * Reduce a markdown body to a one-paragraph excerpt suitable for a
 * card preview. Skips the H1 title, front-matter quote blocks,
 * fenced code, and bullet lists; takes the first prose paragraph and
 * clips it on a word boundary at ~220 chars with an ellipsis.
 *
 * The card UI already shows the title, repo, and source path — so
 * the body here is just enough text to remind the reader what the
 * doc is about, not the full audit.
 */
function excerptFromMarkdown(body: string): string {
  const paragraphs = body
    .split(/\n{2,}/)
    .map((p) => p.trim())
    .filter((p) => p.length > 0);
  for (const p of paragraphs) {
    const first = p.split("\n")[0]?.trim() ?? "";
    if (
      first.startsWith("#") ||      // headings
      first.startsWith(">") ||       // blockquote / front-matter (Status: ...)
      first.startsWith("- ") ||      // bullets
      first.startsWith("* ") ||
      first.startsWith("|") ||       // tables
      first.startsWith("```")        // fenced code
    ) {
      continue;
    }
    return clipOnWord(p.replace(/\s+/g, " "), 220);
  }
  // Fall back to the raw clip when no clean prose paragraph was found.
  return clipOnWord(body.replace(/\s+/g, " ").trim(), 220);
}

/**
 * Reduce an arbitrary status string to its first whitespace- or
 * punctuation-delimited token, lower-cased. `"draft. Indexed as
 * first-class `Analysis` entities ..."` becomes `"draft"`. Empty
 * string falls back to `null` so the caller can pick a default.
 */
function badgeToken(s: string | undefined): string | null {
  if (!s) return null;
  const m = s.trim().match(/[A-Za-z][A-Za-z0-9_-]*/);
  return m ? m[0].toLowerCase() : null;
}

function clipOnWord(s: string, max: number): string {
  if (s.length <= max) return s;
  const slice = s.slice(0, max);
  const lastSpace = slice.lastIndexOf(" ");
  const cut = lastSpace > max * 0.6 ? slice.slice(0, lastSpace) : slice;
  return `${cut.trimEnd()}…`;
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
