import { useQuery } from "@tanstack/react-query";

import { fmtNum } from "../lib/format";
import { api, type HeatmapBlock } from "../lib/api";
import { useConnKey } from "../lib/connections/useConnKey";

export function ToolsView() {
  const connKey = useConnKey();
  const { data, isLoading, error } = useQuery({
    queryKey: [connKey, "tools-stats"],
    queryFn: () => api.toolsStats(),
    refetchInterval: 8000,
    refetchIntervalInBackground: true,
  });

  const rows = data?.tools ?? [];
  const heatmap = data?.heatmap;
  // The `share` field on each row already encodes `calls / total`
  // (a value in [0, 1]). The bar's width MUST be that share verbatim
  // so visual width + trailing percentage label tell the same story:
  // Bash 43.7 % → bar fills 43.7 % of the track, Read 19.8 % → bar
  // fills 19.8 %, etc. A previous `(share / maxShare) * 100` formula
  // pegged Bash at 100 % regardless of its true share — every bar
  // ended up looking identical relative to the leader and the
  // proportions were lost.
  const total = rows.reduce((s, r) => s + r.calls, 0);

  return (
    <div className="view">
      <div className="view__head">
        <div>
          <h1 className="view__title">Tool analytics</h1>
          <p className="view__subtitle">
            Call breakdown aggregated from <span className="mono">kind=tool_call</span> envelopes
            in the seeded archive lane.
          </p>
        </div>
      </div>

      <div className="card" style={{ marginBottom: 16 }}>
        <div className="card__head">
          <span className="card__title">Tool usage breakdown</span>
          <span className="card__sub">{fmtNum(total)} calls observed</span>
        </div>
        {error ? (
          <Empty msg="cortex-api unreachable." />
        ) : isLoading ? (
          <Empty msg="Loading tool stats…" />
        ) : rows.length === 0 ? (
          <Empty msg="No tool calls captured yet. Issue an Edit / Bash / Grep / etc. through Claude Code with the Cortex plugin enabled." />
        ) : (
          <div>
            <div
              className="tool-row"
              style={{
                background: "var(--bg-2)",
                color: "var(--fg-3)",
                fontFamily: "var(--font-mono)",
                fontSize: 10.5,
                textTransform: "uppercase",
                letterSpacing: "0.06em",
              }}
            >
              <span>Tool</span>
              <span style={{ textAlign: "right" }}>Calls</span>
              <span style={{ textAlign: "right" }}>Avg ms</span>
              <span style={{ textAlign: "right" }}>Err rate</span>
              <span>Share</span>
            </div>
            {rows.map((t) => (
              <div key={t.tool} className="tool-row">
                <span className="tool-name">
                  <span className="tool-icon">{t.tool.charAt(0)}</span>
                  {t.tool}
                </span>
                <span
                  className="mono tabular"
                  style={{ textAlign: "right", color: "var(--fg-1)" }}
                >
                  {fmtNum(t.calls)}
                </span>
                <span
                  className="mono tabular"
                  style={{ textAlign: "right", color: t.avg_ms > 1000 ? "var(--warn)" : "var(--fg-1)" }}
                >
                  {t.avg_ms}
                </span>
                <span
                  className="mono tabular"
                  style={{
                    textAlign: "right",
                    color:
                      t.err_rate > 0.05
                        ? "var(--critical)"
                        : t.err_rate > 0.02
                          ? "var(--warn)"
                          : "var(--fg-2)",
                  }}
                >
                  {(t.err_rate * 100).toFixed(1)}%
                </span>
                {/* Bar built from raw inline styles only — every CSS
                    class previously wired here was either invisible
                    (track/fill colors flush against the card bg) or
                    overridden, so the bars all looked identical. This
                    block is the simplest possible progress bar:
                    one fixed-height track div + one absolutely-sized
                    fill div whose width literally equals the share %. */}
                <span
                  style={{
                    display: "flex",
                    alignItems: "center",
                    gap: 8,
                    width: "100%",
                    minWidth: 0,
                  }}
                >
                  <span
                    style={{
                      flex: 1,
                      position: "relative",
                      height: 8,
                      background: "rgba(255, 255, 255, 0.07)",
                      border: "1px solid rgba(255, 255, 255, 0.10)",
                      borderRadius: 999,
                      overflow: "hidden",
                      minWidth: 80,
                    }}
                  >
                    <span
                      style={{
                        position: "absolute",
                        top: 0,
                        left: 0,
                        bottom: 0,
                        width: `${Math.max(0.5, t.share * 100)}%`,
                        background: "var(--accent)",
                        borderRadius: 999,
                      }}
                    />
                  </span>
                  <span
                    className="mono tabular"
                    style={{
                      fontSize: 10.5,
                      color: "var(--fg-2)",
                      width: 48,
                      textAlign: "right",
                    }}
                  >
                    {(t.share * 100).toFixed(1)}%
                  </span>
                </span>
              </div>
            ))}
          </div>
        )}
      </div>

      {heatmap && heatmap.cells.length > 0 ? <Heatmap heatmap={heatmap} /> : null}
    </div>
  );
}

function Heatmap({ heatmap }: { heatmap: HeatmapBlock }) {
  const max = heatmap.cells.reduce(
    (m, row) => row.reduce((mm, v) => Math.max(mm, v), m),
    0,
  );
  const total = heatmap.cells.reduce(
    (s, row) => s + row.reduce((ss, v) => ss + v, 0),
    0,
  );
  return (
    <div className="card">
      <div className="card__head">
        <span className="card__title">Tool-call density · day × hour</span>
        <span className="card__sub">
          {heatmap.tz} · {total} calls in last 7d
        </span>
      </div>
      <div className="card__body">
        <div className="heat-grid">
          <div />
          {Array.from({ length: 24 }).map((_, h) => (
            <div
              key={h}
              style={{
                fontSize: 9.5,
                color: "var(--fg-4)",
                textAlign: "center",
                fontFamily: "var(--font-mono)",
              }}
            >
              {h % 3 === 0 ? h : ""}
            </div>
          ))}
          {heatmap.days.map((d, di) => (
            <div key={d} style={{ display: "contents" }}>
              <div className="heat-row-label">{d}</div>
              {(heatmap.cells[di] ?? []).map((v, hi) => {
                const intensity = max > 0 ? v / max : 0;
                const bg = `oklch(from var(--accent) calc(0.22 + ${intensity * 0.5}) calc(c * ${intensity}) h / ${0.15 + intensity * 0.85})`;
                return (
                  <div
                    key={hi}
                    className="heat-cell"
                    style={{ background: bg }}
                    title={`${d} ${hi}:00 · ${v} calls`}
                  />
                );
              })}
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}

function Empty({ msg }: { msg: string }) {
  return (
    <div
      style={{
        padding: 32,
        textAlign: "center",
        color: "var(--fg-3)",
        fontSize: 12,
      }}
    >
      {msg}
    </div>
  );
}
