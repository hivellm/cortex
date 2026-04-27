## 1. Persistent tweaks store
- [ ] 1.1 Create `gui/src/lib/useTweaks.ts` exporting `useTweaks()` hook returning `[tweaks, setTweak]`
- [ ] 1.2 Persist to `localStorage` under key `cortex.tweaks`; deep-merge with defaults on load (new fields stay defaulted)
- [ ] 1.3 Defaults: `{ theme: "dark", accentHue: 75, density: 7, sidebarCollapsed: false }`
- [ ] 1.4 Wrap `App.tsx` with a `TweaksContext.Provider` exposing the store; existing `theme` / `collapsed` `useState` calls move into the store

## 2. Reflect tweaks in the DOM
- [ ] 2.1 `useEffect` syncing `tweaks.theme` → `document.documentElement.dataset.theme`
- [ ] 2.2 `useEffect` syncing `tweaks.accentHue` → `document.documentElement.style.setProperty("--accent-h", tweaks.accentHue)`
- [ ] 2.3 `useEffect` syncing `tweaks.density` → `--header-h: calc(52px - (10 - density) * 0.8px)` via `style.setProperty`
- [ ] 2.4 `tweaks.sidebarCollapsed` controls the `.app.collapsed` class on the root container

## 3. Tweaks drawer UI
- [ ] 3.1 New `gui/src/shell/Tweaks.tsx` rendering a slide-in panel mirroring the Inspector styles (right-anchored, backdrop, ESC closes)
- [ ] 3.2 Section "Theme" — radio Dark / Light
- [ ] 3.3 Section "Color" — hue slider 20–320° + 5 preset chips (Amber 75 / Green 155 / Blue 230 / Purple 290 / Red 25), each chip rendered as a 24x24 swatch using `oklch(0.78 0.135 ${h})`
- [ ] 3.4 Section "Layout" — toggle for `sidebarCollapsed`, slider 1–10 for `density`
- [ ] 3.5 Section "About" — kv-list with `service`, `version`, `pid`, `uptime` from `/v1/status`
- [ ] 3.6 Add `.tweaks` selectors to `gui/src/styles.css` mirroring `.inspector` (slide animation, backdrop, head, body, sections)

## 4. Header trigger
- [ ] 4.1 Add a gear `icon-btn` to `header__right` opening the drawer
- [ ] 4.2 Remove the existing standalone theme toggle (now lives inside the drawer); leave the gear as the single entry point
- [ ] 4.3 Keep `Esc` and click-on-backdrop both closing the drawer

## 5. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 5.1 Update or create documentation covering the implementation — extend `gui/README.md` with a "Tweaks" section listing the four control groups, the localStorage key, and the CSS variables driven by the store
- [ ] 5.2 Write tests covering the new behavior — Vitest unit on `useTweaks` (round-trip through localStorage, deep-merge with defaults); RTL: opening the gear renders the drawer, picking the green preset writes `--accent-h: 155` to documentElement, density slider updates `--header-h`, ESC closes the drawer
- [ ] 5.3 Run tests and confirm they pass — `pnpm exec tsc --noEmit -p tsconfig.json`, `pnpm test`, `pnpm lint`
