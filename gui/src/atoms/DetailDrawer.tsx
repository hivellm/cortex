import { useEffect } from "react";
import type { ReactNode } from "react";

/// Right-side sliding drawer used for "view full text" affordances on
/// the Decision register and Analysis library cards. Clicking a card
/// opens this — backdrop click and ESC close it. Body scrolls
/// independently so long markdown bodies don't drag the page.
export function DetailDrawer(props: {
  open: boolean;
  onClose: () => void;
  title: ReactNode;
  subtitle?: ReactNode;
  children: ReactNode;
}) {
  const { open, onClose, title, subtitle, children } = props;

  useEffect(() => {
    if (!open) return;
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape") onClose();
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, onClose]);

  if (!open) return null;

  return (
    <div className="drawer-root" role="dialog" aria-modal="true">
      <div
        className="drawer-backdrop"
        onClick={onClose}
        aria-hidden="true"
      />
      <aside className="drawer">
        <header className="drawer__head">
          <div className="drawer__titles">
            <div className="drawer__title">{title}</div>
            {subtitle ? <div className="drawer__subtitle">{subtitle}</div> : null}
          </div>
          <button
            type="button"
            className="drawer__close"
            onClick={onClose}
            aria-label="Close"
            title="Close (Esc)"
          >
            ×
          </button>
        </header>
        <div className="drawer__body">{children}</div>
      </aside>
    </div>
  );
}
