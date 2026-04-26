import { Icon } from "../atoms/Icon";

type HeaderProps = {
  collapsed: boolean;
  onToggleSidebar: () => void;
  theme: "dark" | "light";
  onToggleTheme: () => void;
};

export function Header({ collapsed, onToggleSidebar, theme, onToggleTheme }: HeaderProps) {
  return (
    <header className="header">
      <div className="header__brand">
        <button className="icon-btn" onClick={onToggleSidebar} title="Toggle sidebar" aria-label="Toggle sidebar">
          <Icon name="menu" size={15} />
        </button>
        <span className="brand-mark" />
        <span className="header__brand-text" style={{ display: "flex", alignItems: "baseline", gap: 8 }}>
          <span className="brand-name">Cortex</span>
          <span className="brand-version mono">v0.1</span>
        </span>
        {collapsed ? (
          <span className="mono" style={{ marginLeft: 12, fontSize: 10.5, color: "var(--fg-3)" }}>
            sidebar collapsed
          </span>
        ) : null}
      </div>
      <div className="header__actions">
        <button
          className="icon-btn"
          onClick={onToggleTheme}
          title={`Switch to ${theme === "dark" ? "light" : "dark"} mode`}
          aria-label="Toggle theme"
        >
          <Icon name={theme === "dark" ? "sun" : "moon"} size={15} />
        </button>
      </div>
    </header>
  );
}
