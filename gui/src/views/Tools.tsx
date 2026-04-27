import { useQuery } from "@tanstack/react-query";

import { fmtNum } from "../lib/format";
import { api, type HeatmapBlock } from "../lib/api";

export function ToolsView() {
  const { data, isLoading, error } = useQuery({
    queryKey: ["tools-stats"],
    queryFn: () => api.toolsStats(),
    refetchInterval: 8000,
    refetchIntervalInBackground: true,
  });

  const rows = data?.tools ?? [];
  const heatmap = data?.heatmap;
  // The `share` field on each row already encodes calls/total. The
  // bar reflects that share so the visual width and the trailing
  // percentage label tell the same story; using calls/max here would
  // peg the top tool at 100% even when its real share is, say, 44%
  // — which is what the screenshot in the bug report showed.
  const total = rows.reduce((s, r) => s + r.calls, 0);
  const maxShare = rows.reduce((m, r) => Math.max(m, r.share), 0);

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
                <span className="tool-bar">
                  <span className="tool-bar__track">
                    <span
                      className="tool-bar__fill"
                      style={{
                        // Scale the bar by the row's share relative to
                        // the largest share so the leader pins at 100%
                        // and every other row is in honest proportion.
                        width: `${maxShare > 0 ? (t.share / maxShare) * 100 : 0}%`,
                      }}
                    />
                  </span>
                  <span
                    className="mono tabular"
                    style={{ fontSize: 10.5, color: "var(--fg-3)", width: 48, textAlign: "right" }}
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
