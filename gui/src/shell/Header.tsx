import { useQuery } from "@tanstack/react-query";

import { Icon } from "../atoms/Icon";
import { api } from "../lib/api";
import { bridge } from "../lib/bridge";
import { useConnKey } from "../lib/connections/useConnKey";
import { ConnectionSwitcher } from "./ConnectionSwitcher";
import { HeaderSearch } from "./HeaderSearch";

type HeaderProps = {
  onOpenTweaks: () => void;
  /// Phase8g — invoked when the topbar health pill is clicked so the
  /// shell can navigate to /health from any view.
  onJumpToHealth?: () => void;
  /// Phase3 §5 — invoked when the user picks "Manage connections…"
  /// from the active-connection switcher dropdown.
  onJumpToConnections?: () => void;
};

export function Header({
  onOpenTweaks,
  onJumpToHealth,
  onJumpToConnections,
}: HeaderProps) {
  const connKey = useConnKey();
  const statusQ = useQuery({
    queryKey: [connKey, "status"],
    queryFn: () => api.status(),
    refetchInterval: 5000,
    refetchIntervalInBackground: true,
    retry: 0,
  });
  // Phase8g — overall stack health pill. Polls /v1/health every 5 s
  // and renders a green/yellow/red dot visible from every view.
  // Click jumps to /health.
  const healthQ = useQuery({
    queryKey: [connKey, "health", "overview", "topbar"],
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
        <span className="brand-mark" />
        <span className="header__brand-text" style={{ display: "flex", alignItems: "baseline", gap: 8 }}>
          <span className="brand-name">Cortex</span>
          <span className="brand-version mono">v{version}</span>
        </span>
      </div>
      <HeaderSearch />
      <div className="header__right">
        <ConnectionSwitcher onManage={onJumpToConnections} />
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
    </header>
  );
}
