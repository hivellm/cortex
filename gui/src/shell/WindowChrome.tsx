import { isElectron } from "../lib/bridge";

/// Custom Electron titlebar — a slim 28-px strip pinned above the
/// regular header. Renders only inside Electron (the Vite browser
/// preview stays unchanged). Vectorizer's GUI uses the same
/// pattern: drag region OUTSIDE the interactive header so clicks
/// on the menu / search / nav buttons never get intercepted by
/// `-webkit-app-region: drag`.
export function WindowChrome() {
  if (!isElectron) return null;
  return (
    <div className="window-chrome drag-region">
      <div className="window-chrome__brand">
        <span className="window-chrome__mark" aria-hidden />
        <span className="window-chrome__title">Cortex</span>
      </div>
      <div className="window-chrome__controls no-drag">
        <button
          type="button"
          className="window-btn"
          onClick={() => window.cortex?.windowMinimize?.()}
          title="Minimize"
          aria-label="Minimize window"
        >
          <span className="window-btn__glyph">─</span>
        </button>
        <button
          type="button"
          className="window-btn"
          onClick={() => window.cortex?.windowMaximize?.()}
          title="Maximize"
          aria-label="Maximize window"
        >
          <span className="window-btn__glyph">▢</span>
        </button>
        <button
          type="button"
          className="window-btn window-btn--close"
          onClick={() => window.cortex?.windowClose?.()}
          title="Close"
          aria-label="Close window"
        >
          <span className="window-btn__glyph">✕</span>
        </button>
      </div>
    </div>
  );
}
