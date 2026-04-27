## 1. Atoms
- [ ] 1.1 Port `SeverityBar` from `gui/assets/atoms.jsx` to `gui/src/atoms/SeverityBar.tsx` (3 segments, info/warn/critical color map)
- [ ] 1.2 Port `Bars` from `gui/assets/atoms.jsx` to `gui/src/atoms/Bars.tsx`
- [ ] 1.3 Use `Sparkline` in at least one place (defer Timeline stats grid to phase2b — wire it here as a smoke test, e.g. inside the Sidebar overview header)

## 2. Sidebar — Repos group + nav counts
- [ ] 2.1 Fetch `/v1/dashboard/overview` once at mount; expose `recent_repos` + `events_total` to the Sidebar
- [ ] 2.2 Render group label "Repos · N indexed" with the repo count
- [ ] 2.3 Each repo row: color dot, mono name, event count pill; click toggles `filters.repo` and forces `view = "timeline"`
- [ ] 2.4 Active repo gets `is-active` class (mirrors session-item)
- [ ] 2.5 Each nav item gets an optional count pill (Memory: events_total; Decisions: decisions length; Sessions: sessions length; Tools: distinct tools count) — compute via the existing TanStack Query caches, not new fetches
- [ ] 2.6 Counts are formatted via `fmtNum`; hide pill when count is 0

## 3. Memory card polish
- [ ] 3.1 Replace plain `Tag tone="info"` with the design's solid repo tag (`Tag tone="solid"` — add the variant if missing)
- [ ] 3.2 Topic chips render with `#` prefix (matches `views-mid.jsx` line 52)
- [ ] 3.3 Card hover lift effect — confirm `.memory-card:hover` matches the design's `.memory:hover` (translateY + shadow); add the hover rule if missing in `styles.css`

## 4. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 4.1 Update or create documentation covering the implementation — extend `gui/README.md` with a sub-section "Atoms" listing `Icon / Sparkline / Bars / SeverityBar / Tag` and where each is used
- [ ] 4.2 Write tests covering the new behavior — Vitest unit tests for `SeverityBar` (3 severities → correct seg counts), `Bars` (data array → correct DOM), `Sparkline` (existing atom — add a missing test for empty input); React Testing Library smoke test for the Sidebar Repos group click → filter set
- [ ] 4.3 Run tests and confirm they pass — `pnpm exec tsc --noEmit -p tsconfig.json` (zero errors), `pnpm test`, `pnpm lint`
