/**
 * `useTweaks` — operator-tunable surface (theme / accent hue / density /
 * sidebar collapse) backed by `localStorage` so the chosen settings
 * survive a renderer reload.
 *
 * Storage shape mirrors the design's tweak-panel store
 * (`gui/assets/app.jsx` lines 244-299) minus the iframe `liveSSE`
 * plumbing, which the Timeline view's own pause/resume control
 * (phase2b) replaces.
 *
 * Defaults match the renderer's pre-tweak behaviour, so a stale
 * localStorage entry without one of the keys still produces a
 * working UI — never re-prompts the user.
 */

import { createContext, useCallback, useContext, useEffect, useMemo, useState } from "react";

export type TweaksState = {
  /// Dark / Light controls `document.documentElement.dataset.theme`.
  theme: "dark" | "light";
  /// `--accent-h` CSS custom property (oklch hue, 20°–320°).
  accentHue: number;
  /// 1–10 density slider; mapped onto `--header-h` in the DOM sync
  /// effect so larger values give a chunkier UI.
  density: number;
};

export const TWEAKS_STORAGE_KEY = "cortex.tweaks";

export const DEFAULT_TWEAKS: TweaksState = {
  theme: "dark",
  accentHue: 75,
  density: 7,
};

/// Five accent presets shown as colour swatches in the drawer.
/// Hues match the design's `tweaks-panel.jsx` preset list.
export const ACCENT_PRESETS: { name: string; hue: number }[] = [
  { name: "Amber", hue: 75 },
  { name: "Green", hue: 155 },
  { name: "Blue", hue: 230 },
  { name: "Purple", hue: 290 },
  { name: "Red", hue: 25 },
];

type TweaksApi = {
  tweaks: TweaksState;
  setTweak: <K extends keyof TweaksState>(key: K, value: TweaksState[K]) => void;
  reset: () => void;
};

const TweaksContext = createContext<TweaksApi | null>(null);

/// Read the persisted tweaks from `localStorage`; deep-merges with
/// `DEFAULT_TWEAKS` so a partial entry (or one written by a previous
/// schema version) still produces a complete state.
function loadFromStorage(): TweaksState {
  if (typeof window === "undefined") return DEFAULT_TWEAKS;
  try {
    const raw = window.localStorage.getItem(TWEAKS_STORAGE_KEY);
    if (!raw) return DEFAULT_TWEAKS;
    const parsed = JSON.parse(raw) as Partial<TweaksState>;
    return {
      theme: parsed.theme === "light" ? "light" : "dark",
      accentHue: clampHue(typeof parsed.accentHue === "number" ? parsed.accentHue : DEFAULT_TWEAKS.accentHue),
      density: clampDensity(
        typeof parsed.density === "number" ? parsed.density : DEFAULT_TWEAKS.density,
      ),
    };
  } catch {
    return DEFAULT_TWEAKS;
  }
}

function clampHue(h: number): number {
  if (Number.isNaN(h)) return DEFAULT_TWEAKS.accentHue;
  if (h < 20) return 20;
  if (h > 320) return 320;
  return Math.round(h);
}

function clampDensity(d: number): number {
  if (Number.isNaN(d)) return DEFAULT_TWEAKS.density;
  if (d < 1) return 1;
  if (d > 10) return 10;
  return Math.round(d);
}

/// Wrap children with the tweaks store. Single instance per renderer
/// — the provider also drives the DOM-side variable sync so the
/// caller doesn't have to wire effects in `App.tsx`.
export function TweaksProvider({ children }: { children: React.ReactNode }) {
  const [tweaks, setTweaks] = useState<TweaksState>(() => loadFromStorage());

  // Persist every change. `localStorage` is synchronous so an
  // unmount mid-update can't drop the write.
  useEffect(() => {
    if (typeof window === "undefined") return;
    try {
      window.localStorage.setItem(TWEAKS_STORAGE_KEY, JSON.stringify(tweaks));
    } catch {
      // Storage quota / disabled — silent. The tweak is still live in
      // memory for this session.
    }
  }, [tweaks]);

  // Reflect into the DOM. Each variable is set on `documentElement`
  // so every `var(--…)` reference picks it up regardless of where
  // the component lives in the tree.
  useEffect(() => {
    const root = document.documentElement;
    root.dataset.theme = tweaks.theme;
    root.style.setProperty("--accent-h", String(tweaks.accentHue));
    // Density 1 → header gets ~7.2px shorter than the default 52;
    // density 10 → no change. Mirrors `gui/assets/app.jsx`.
    const headerH = 52 - (10 - tweaks.density) * 0.8;
    root.style.setProperty("--header-h", `${headerH.toFixed(1)}px`);
  }, [tweaks.theme, tweaks.accentHue, tweaks.density]);

  const setTweak = useCallback(
    <K extends keyof TweaksState>(key: K, value: TweaksState[K]) => {
      setTweaks((prev) => {
        if (prev[key] === value) return prev;
        return { ...prev, [key]: value };
      });
    },
    [],
  );

  const reset = useCallback(() => setTweaks(DEFAULT_TWEAKS), []);

  const api = useMemo<TweaksApi>(() => ({ tweaks, setTweak, reset }), [tweaks, setTweak, reset]);

  return <TweaksContext.Provider value={api}>{children}</TweaksContext.Provider>;
}

/// Consume the tweaks store. Throws when called outside the
/// provider so a stale import (e.g. a renderer-only test) surfaces
/// the wiring bug immediately instead of silently using defaults.
export function useTweaks(): TweaksApi {
  const ctx = useContext(TweaksContext);
  if (!ctx) {
    throw new Error("useTweaks called outside TweaksProvider");
  }
  return ctx;
}
