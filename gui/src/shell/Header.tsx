import { useQuery } from "@tanstack/react-query";

import { Icon } from "../atoms/Icon";
import { api } from "../lib/api";
import { bridge } from "../lib/bridge";

type HeaderProps = {
  collapsed: boolean;
  onToggleSidebar: () => void;
  onOpenTweaks: () => void;
  /// Phase8g — invoked when the topbar health pill is clicked so the
  /// shell can navigate to /health from any view.
  onJumpToHealth?: () => void;
};

export function Header({
  collapsed,
  onToggleSidebar,
  onOpenTweaks,
  onJumpToHealth,
}: HeaderProps) {
  const statusQ = useQuery({
    queryKey: ["status"],
    queryFn: () => api.status(),
    refetchInterval: 5000,
    refetchIntervalInBackground: true,
    retry: 0,
  });
  // Phase8g — overall stack health pill. Polls /v1/health every 5 s
  // and renders a green/yellow/red dot visible from every view.
  // Click jumps to /health.
  const healthQ = useQuery({
    queryKey: ["health", "overview", "topbar"],
    queryFn: () => api.healthOverview(),
    refetchInterval: 5000,
    refetchIntervalInBackground: true,
    retry: 0,
  });

  const live = !statusQ.isError && !!statusQ.data;
  const version = statusQ.data?.version ?? bridge.buildId;
  const pillLabel = statusQ.isLoading
    ? "connecting…"
    : live
      ? `live · pid ${statusQ.data!.pid}`
      : "offline";

  return (
    <header className="header">
      <div className="header__brand">
        <button className="icon-btn" onClick={onToggleSidebar} title="Toggle sidebar" aria-label="Toggle sidebar">
          <Icon name="menu" size={15} />
        </button>
        <span className="brand-mark" />
        <span className="header__brand-text" style={{ display: "flex", alignItems: "baseline", gap: 8 }}>
          <span className="brand-name">Cortex</span>
          <span className="brand-version mono">v{version}</span>
        </span>
      </div>
      <div className="header__right">
        <button
          type="button"
          className={`status-pill health-topbar-pill is-${
            healthQ.data?.overall ?? (healthQ.isError ? "down" : "unknown")
          }`}
          title={
            healthQ.data
              ? `stack ${healthQ.data.overall} · ${healthQ.data.subsystems.length} subsystems · click for /health`
              : healthQ.isError
                ? "health stream offline — click for /health"
                : "loading /v1/health…"
          }
          onClick={onJumpToHealth}
          style={{ cursor: onJumpToHealth ? "pointer" : "default" }}
        >
          <span className="dot" />
          <span className="mono">
            {healthQ.data?.overall ?? (healthQ.isError ? "down" : "…")}
          </span>
        </button>
        <span
          className={`status-pill ${live ? "" : "is-paused"}`}
          title={
            live
              ? `cortex-api ${statusQ.data!.service} · uptime ${Math.round(statusQ.data!.uptime_ms / 1000)}s`
              : "cortex-api unreachable — start it with `cargo run -p cortex-api`"
          }
        >
          <span className="dot" />
          <span className="mono">{pillLabel}</span>
        </span>
        <button
          className="icon-btn"
          onClick={onOpenTweaks}
          title="Open tweaks (theme · accent · density)"
          aria-label="Open tweaks panel"
        >
          <Icon name="settings" size={15} />
        </button>
      </div>
      {collapsed ? null : null}
    </header>
  );
}
