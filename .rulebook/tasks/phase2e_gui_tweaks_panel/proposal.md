# Proposal: phase2e_gui_tweaks_panel

## Why

The design ships a Tweaks panel (`gui/assets/tweaks-panel.jsx` + `gui/assets/app.jsx` lines 244–284) that lets the operator switch theme, accent hue (a 20–320° slider plus five preset chips), data density (1–10 slider mapped to `--header-h`), sidebar collapse, and SSE simulation flag. The current GUI exposes only a theme toggle in the Header, with no persistence — the user's chosen theme resets on every reload. The hue/density tokens are defined in `styles.css` but no UI surfaces them.

The original `tweaks-panel.jsx` is a `postMessage` harness for Anthropic's iframe prototyping environment; it cannot be ported as-is to Electron. The concept (a floating settings drawer with `localStorage` persistence) is portable.

Source: `gui/assets/tweaks-panel.jsx` (Tweak* controls), `gui/assets/app.jsx` lines 244–299 (TweaksFor + useTweaks integration).

## What Changes

- New `gui/src/shell/Tweaks.tsx` — a slide-in drawer triggered by a header gear icon. Mirrors the `inspector` styling so the visual language stays consistent.
- New `gui/src/lib/useTweaks.ts` — `localStorage`-persisted state under key `cortex.tweaks`. Shape:
  ```ts
  {
    theme: "dark" | "light",
    accentHue: number,           // 20–320
    density: number,             // 1–10
    sidebarCollapsed: boolean,
  }
  ```
- Tweaks render four control groups:
  - **Theme** — radio (Dark / Light)
  - **Color** — hue slider + five preset chips (Amber 75 / Green 155 / Blue 230 / Purple 290 / Red 25)
  - **Layout** — sidebar collapse toggle + density slider (1–10 maps to `--header-h: calc(52px - (10 - density) * 0.8px)`)
  - **About** — read-only kv list with `service`, `version`, `pid`, `uptime` from `/v1/status`
- `App.tsx` provides a `TweaksContext`; existing `theme` / `collapsed` state moves into the tweaks store.
- Header gets a gear icon in `header__right` that opens the panel.
- The `liveSSE` flag from the original prototype is dropped — the live/pause control on Timeline (phase2b) replaces it.

## Impact

- Affected specs: none.
- Affected code: `gui/src/App.tsx`, `gui/src/shell/Header.tsx`, `gui/src/shell/Tweaks.tsx` (new), `gui/src/lib/useTweaks.ts` (new), `gui/src/styles.css` (extend with `.tweaks` selectors mirroring `.inspector`).
- Breaking change: NO — defaults match current behavior.
- Depends on: nothing.
- User benefit: theme/accent/density survive reload; operator can tune the GUI without forking CSS.
