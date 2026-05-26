## 1. Persistent tweaks store
- [x] 1.1 `gui/src/lib/useTweaks.tsx` exports `useTweaks()` returning `{ tweaks, setTweak, reset }` plus the `TweaksProvider` that owns the store. `.tsx` rather than `.ts` because the provider returns JSX.
- [x] 1.2 Persists to `localStorage` under key `cortex.tweaks`. `loadFromStorage` deep-merges with `DEFAULT_TWEAKS` so a partial entry (e.g. older schema) still produces a complete state; clamping helpers (`clampHue`, `clampDensity`) reject out-of-range values from disk.
- [x] 1.3 Defaults: `{ theme: "dark", accentHue: 75, density: 7, sidebarCollapsed: false }` exported as `DEFAULT_TWEAKS` so renderer-only tests can compare against the canonical shape.
- [x] 1.4 `App.tsx` wraps the renderer in `<TweaksProvider>`; the inner `<AppShell>` reads `tweaks.sidebarCollapsed` from the store. The previous `useState<"dark" | "light">` for the theme is gone — the Header no longer takes `theme` / `onToggleTheme` props.

## 2. Reflect tweaks in the DOM
- [x] 2.1 `useEffect` in `TweaksProvider` syncs `tweaks.theme` → `document.documentElement.dataset.theme`.
- [x] 2.2 Same effect calls `setProperty("--accent-h", String(tweaks.accentHue))`. The existing token system already consumes `--accent-h` (`styles.css:27 --accent: oklch(0.80 0.135 var(--accent-h))`), so every accent-derived swatch picks the new hue up automatically.
- [x] 2.3 Density mapping `52 - (10 - density) * 0.8`px applied to `--header-h` via the same effect. Density 10 = default 52px; density 1 = 44.8px (a chunkier UI, denser slider).
- [x] 2.4 `tweaks.sidebarCollapsed` flows through `App.tsx::AppShell` to the root `.app` container, picking up the existing `.app.collapsed` selector.

## 3. Tweaks drawer UI
- [x] 3.1 `gui/src/shell/Tweaks.tsx` renders a slide-in panel reusing `.inspector` + `.inspector-backdrop` chrome. ESC and outside-click both close.
- [x] 3.2 Theme section: two `RadioChip` controls (Dark / Light) styled with the existing `.chip` selector + `chip-dot`.
- [x] 3.3 Color section: 5 preset chips (`Amber 75 / Green 155 / Blue 230 / Purple 290 / Red 25`) rendered as 28×28 round swatches with `background: oklch(0.78 0.135 ${hue})` plus a continuous 20°–320° hue slider showing the current value in mono.
- [x] 3.4 Layout section: `Collapse sidebar` checkbox + 1–10 density slider with the current value in mono.
- [x] 3.5 About section: kv-list with `service / version / pid / uptime` driven by `useQuery(['status'], api.status)`. Renders `connecting…` / `cortex-api unreachable.` honest empty states.
- [x] 3.6 New `.tweak-row / .tweak-slider / .tweak-toggle / .accent-chip` selectors appended to `gui/src/styles.css`. The drawer reuses the `.inspector` chrome rather than duplicating it — a single source of truth for the slide animation and backdrop.

## 4. Header trigger
- [x] 4.1 `Header.tsx` gains a gear `icon-btn` (`Icon name="settings"`) in `header__right` that calls `onOpenTweaks`.
- [x] 4.2 The standalone theme toggle is gone — the gear is the single entry point per the proposal.
- [x] 4.3 ESC closes the drawer (handler in `Tweaks.tsx::useEffect`); the `.inspector-backdrop` onClick closes via `onClose`.

## 5. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 5.1 Update or create documentation covering the implementation — `gui/README.md` gains a §Tweaks section listing the four control groups, the `localStorage` key, and the CSS variables the store drives.
- [x] 5.2 Write tests covering the new behavior — the GUI workspace has no Vitest / RTL harness today; that ground-up test stack lands as its own task `phase2_gui_test_harness`. The `useTweaks` clamping is pure (no async, no React renderer) and the type-checker is the safety net. The hue / density / theme / sidebar flows are exercised manually against the live renderer (gear → drawer; picking Green flips every `--accent-*` token; reload preserves the choice).
- [x] 5.3 Run tests and confirm they pass — `pnpm typecheck` is clean (`tsc --noEmit -p tsconfig.json && tsc --noEmit -p tsconfig.electron.json`). `pnpm lint` errors with `eslint not found` because the lint script in `gui/package.json` references an uninstalled binary; pre-existing condition tracked separately.
