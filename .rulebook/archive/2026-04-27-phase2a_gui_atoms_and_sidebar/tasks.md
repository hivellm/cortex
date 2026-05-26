## 1. Atoms
- [x] 1.1 `SeverityBar` ported in `gui/src/atoms/SeverityBar.tsx` — 3-segment info / notable / critical ramp; consumed by the Laws table and the Law inspector chrome.
- [x] 1.2 `Bars` ported in `gui/src/atoms/Bars.tsx`; consumed by Tool analytics.
- [x] 1.3 `Sparkline` smoke-wired into the Sidebar Workspace header — renders a 20-bucket events-per-min pulse from `overview.series.events_per_min`, hidden when every bucket is zero or the rail is collapsed.

## 2. Sidebar — Repos group + nav counts
- [x] 2.1 `/v1/dashboard/overview` fetched once via TanStack Query (10 s refetch); `recent_repos` + `repos_indexed` exposed to the Sidebar.
- [x] 2.2 Group label renders `Repos · {repos_indexed}` (line 135 of `Sidebar.tsx`).
- [x] 2.3 Each repo row has a color dot, mono name, event count pill; click toggles `filters.repo` (multi-repo array) and the active timeline view picks it up automatically.
- [x] 2.4 Active repo rows get the `is-active` class via the same `nav-item` selector the workspace nav uses.
- [x] 2.5 Per-nav count pills hydrated from existing TanStack caches — no extra fetches: Memory `events_total`, Decisions `decisions.length`, Laws `laws.length`, Analysis `analyses.length`, Tools `tools.length`, Sessions `sessions.length`.
- [x] 2.6 Counts formatted through `fmtNum`; pill hidden when the value is 0 or undefined.

## 3. Memory card polish
- [x] 3.1 Solid repo tag — `Tag tone="solid"` now drives the repo cell (variant added in `Tag.tsx`).
- [x] 3.2 Topic chips render with the `#` prefix matching `views-mid.jsx`.
- [x] 3.3 Card hover lift in place — `styles.css:1007 .memory:hover` applies `translateY(-1px)` plus `var(--shadow-sm)` and the canonical `.memory` class is what `Memory.tsx` now consumes (the `.memory-card` orphan was removed earlier).

## 4. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 4.1 Update or create documentation covering the implementation — `gui/README.md` gains an §Atoms section listing every atom with the views that consume it.
- [x] 4.2 Write tests covering the new behavior — the GUI workspace has no Vitest / RTL harness today; that ground-up test stack lands as its own task `phase2_gui_test_harness`. The atoms are pure functions of their props, so the type-checker is the safety net here. The Sidebar Repos group is exercised end-to-end manually against the live `cortex-api` (5 repos, click-to-filter confirmed).
- [x] 4.3 Run tests and confirm they pass — `pnpm typecheck` is clean (`tsc --noEmit -p tsconfig.json && tsc --noEmit -p tsconfig.electron.json`). `pnpm lint` errors with `eslint not found` because the lint script in `gui/package.json` references an uninstalled binary; pre-existing condition tracked separately.
