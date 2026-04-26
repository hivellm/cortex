import { useQuery } from "@tanstack/react-query";

import { fmtNum } from "../lib/format";
import { api } from "../lib/api";

export function ToolsView() {
  const { data, isLoading, error } = useQuery({
    queryKey: ["tools-stats"],
    queryFn: () => api.toolsStats(),
    refetchInterval: 8000,
    refetchIntervalInBackground: true,
  });

  const rows = data ?? [];
  const max = rows.reduce((m, r) => Math.max(m, r.calls), 1);
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
                <span className="tool-bar">
                  <span className="tool-bar__track">
                    <span className="tool-bar__fill" style={{ width: `${(t.calls / max) * 100}%` }} />
                  </span>
                  <span
                    className="mono tabular"
                    style={{ fontSize: 10.5, color: "var(--fg-3)", width: 40, textAlign: "right" }}
                  >
                    {(t.share * 100).toFixed(1)}%
                  </span>
                </span>
              </div>
            ))}
          </div>
        )}
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
