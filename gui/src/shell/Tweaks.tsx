/**
 * Tweaks drawer — slide-in panel that exposes the operator-tunable
 * surface (`useTweaks`) plus a read-only About section sourced from
 * `/v1/status`. Mirrors the `.inspector` chrome so the visual
 * language stays consistent across drawers.
 *
 * Open / close is owned by the caller (Header gear icon flips a
 * boolean); ESC + outside-click both close so the drawer never
 * traps focus.
 */

import { useEffect } from "react";
import { useQuery } from "@tanstack/react-query";

import { Icon } from "../atoms/Icon";
import { api } from "../lib/api";
import { ACCENT_PRESETS, useTweaks } from "../lib/useTweaks";
import { useConnKey } from "../lib/connections/useConnKey";

type TweaksProps = {
  open: boolean;
  onClose: () => void;
};

export function Tweaks({ open, onClose }: TweaksProps) {
  const { tweaks, setTweak, reset } = useTweaks();

  // ESC closes the drawer regardless of focus location, mirroring
  // the EventInspector / LawInspector behaviour.
  useEffect(() => {
    if (!open) return;
    const handler = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    document.addEventListener("keydown", handler);
    return () => document.removeEventListener("keydown", handler);
  }, [open, onClose]);

  const connKey = useConnKey();
  const statusQ = useQuery({
    queryKey: [connKey, "status"],
    queryFn: () => api.status(),
    refetchInterval: open ? 5000 : false,
    enabled: open,
    retry: 0,
  });

  return (
    <>
      <div
        className={`inspector-backdrop ${open ? "is-open" : ""}`}
        onClick={onClose}
      />
      <aside
        className={`inspector tweaks ${open ? "is-open" : ""}`}
        aria-hidden={!open}
        aria-label="Tweaks"
      >
        <div className="inspector__head">
          <span
            style={{
              width: 26,
              height: 26,
              display: "grid",
              placeItems: "center",
              borderRadius: 4,
              background: "var(--accent-soft)",
              color: "var(--accent)",
              border: "1px solid var(--accent)",
            }}
          >
            <Icon name="settings" size={14} />
          </span>
          <div style={{ display: "flex", flexDirection: "column", gap: 2, minWidth: 0 }}>
            <span className="inspector__title">Tweaks</span>
            <span className="inspector__id">theme · accent · density · sidebar</span>
          </div>
          <button
            className="icon-btn"
            onClick={onClose}
            style={{ marginLeft: "auto" }}
            aria-label="Close tweaks"
          >
            <Icon name="close" size={15} />
          </button>
        </div>
        <div className="inspector__body">
          <Section label="Theme">
            <div className="tweak-row">
              <RadioChip
                name="theme"
                value="dark"
                current={tweaks.theme}
                onPick={(v) => setTweak("theme", v)}
                label="Dark"
              />
              <RadioChip
                name="theme"
                value="light"
                current={tweaks.theme}
                onPick={(v) => setTweak("theme", v)}
                label="Light"
              />
            </div>
          </Section>

          <Section label="Color">
            <div className="tweak-row">
              {ACCENT_PRESETS.map((p) => (
                <button
                  key={p.hue}
                  type="button"
                  className={`accent-chip ${tweaks.accentHue === p.hue ? "is-active" : ""}`}
                  onClick={() => setTweak("accentHue", p.hue)}
                  title={`${p.name} · hue ${p.hue}°`}
                  aria-label={`Accent ${p.name}`}
                  style={{
                    width: 28,
                    height: 28,
                    borderRadius: "50%",
                    background: `oklch(0.78 0.135 ${p.hue})`,
                    border:
                      tweaks.accentHue === p.hue
                        ? "2px solid var(--fg-0)"
                        : "1px solid var(--border)",
                    cursor: "pointer",
                    padding: 0,
                  }}
                />
              ))}
            </div>
            <label className="tweak-slider">
              <span className="tweak-slider__label">
                Hue <span className="mono tabular">{tweaks.accentHue}°</span>
              </span>
              <input
                type="range"
                min={20}
                max={320}
                value={tweaks.accentHue}
                onChange={(e) => setTweak("accentHue", Number(e.target.value))}
              />
            </label>
          </Section>

          <Section label="Layout">
            <label className="tweak-slider">
              <span className="tweak-slider__label">
                Density <span className="mono tabular">{tweaks.density}</span>
              </span>
              <input
                type="range"
                min={1}
                max={10}
                value={tweaks.density}
                onChange={(e) => setTweak("density", Number(e.target.value))}
              />
            </label>
          </Section>

          <Section label="About">
            {statusQ.isError ? (
              <div className="muted" style={{ fontSize: 11.5 }}>
                cortex-api unreachable.
              </div>
            ) : statusQ.isLoading || !statusQ.data ? (
              <div className="muted" style={{ fontSize: 11.5 }}>
                connecting…
              </div>
            ) : (
              <dl className="kv-list">
                <dt>service</dt>
                <dd className="mono">{statusQ.data.service}</dd>
                <dt>version</dt>
                <dd className="mono">{statusQ.data.version}</dd>
                <dt>pid</dt>
                <dd className="mono tabular">{statusQ.data.pid}</dd>
                <dt>uptime</dt>
                <dd className="mono tabular">
                  {Math.round(statusQ.data.uptime_ms / 1000)}s
                </dd>
              </dl>
            )}
          </Section>

          <div style={{ marginTop: 18, display: "flex", justifyContent: "flex-end" }}>
            <button className="btn btn--sm btn--ghost" type="button" onClick={reset}>
              Reset to defaults
            </button>
          </div>
        </div>
      </aside>
    </>
  );
}

function Section({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="inspector__section">
      <div className="inspector__section-label">{label}</div>
      {children}
    </div>
  );
}

function RadioChip({
  name,
  value,
  current,
  onPick,
  label,
}: {
  name: string;
  value: "dark" | "light";
  current: "dark" | "light";
  onPick: (v: "dark" | "light") => void;
  label: string;
}) {
  return (
    <label className={`chip ${current === value ? "is-active" : ""}`}>
      <input
        type="radio"
        name={name}
        value={value}
        checked={current === value}
        onChange={() => onPick(value)}
        style={{ position: "absolute", opacity: 0, pointerEvents: "none" }}
      />
      <span className="chip-dot" />
      <span>{label}</span>
    </label>
  );
}
