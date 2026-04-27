import { useQuery } from "@tanstack/react-query";

import { Icon } from "../atoms/Icon";
import { api } from "../lib/api";
import { bridge } from "../lib/bridge";

type HeaderProps = {
  collapsed: boolean;
  onToggleSidebar: () => void;
  onOpenTweaks: () => void;
};

export function Header({ collapsed, onToggleSidebar, onOpenTweaks }: HeaderProps) {
  const statusQ = useQuery({
    queryKey: ["status"],
    queryFn: () => api.status(),
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
