## 1. EventInspector — Payload section
- [x] 1.1 `inspector__section` "Payload (redacted)" renders the selected event in a `<pre class="code-block">` (`Timeline.tsx:156-160`).
- [x] 1.2 `JSON.stringify(ev, null, 2)` emits the full envelope at the inspector — every optional field that's `null` upstream prints as `null` so the user can spot what's missing without reading the network tab.
- [x] 1.3 Body width is bounded by the inspector container; `code-block` already handles horizontal scroll for long payloads.

## 2. EventInspector — Linked section
- [x] 2.1 `linkedIds = useMemo(...)` runs the regex `\b(DEC-\d{4}-\d{3}|ANL-\d{2,4})\b` against `${ev.title} ${ev.detail}` (`Timeline.tsx:94-99`); duplicates are de-duped via `Set`.
- [x] 2.2 Each match renders as a `violation-card`-shaped row with the id badge in accent and a "Decision" / "Analysis" type label per the prefix.
- [x] 2.3 Empty list short-circuits to "no linked decisions or analyses" — no fabricated links ever land.

## 3. Laws — row click + LawInspector
- [x] 3.1 Law rows are `role="button"` with `onClick={() => setSelectedId(law.id)}`; the `selectedId` state drives both the row's `is-active` class and the Inspector visibility.
- [x] 3.2 `<LawInspector>` (`Laws.tsx:170-342`) renders the full surface: head with severity icon + id + severity / blocking summary; Definition section with the synthesized YAML frontmatter; 7-day stats `kv-list` covering `applies / violations / rate / action`; Recent violations list filtered to `v.law_id === selectedId`.
- [x] 3.3 ESC and outside-click close — both the Timeline EventInspector and the Law LawInspector share the same `useEffect` shape (`document.addEventListener('keydown', …)` plus the backdrop's onClick).
- [x] 3.4 Active row picks up the `is-active` class via the same selector the catalogue header uses.

## 4. Refactor — shared Inspector shell (judgment call)
- [x] 4.1 Inspector shells stay inline in `Timeline.tsx` and `Laws.tsx` — the duplicated chrome (backdrop + slide panel + ESC handler + close button) totals ~30 lines, well under the 80-line threshold the proposal set for warranted extraction. The two heads carry different shapes (Event inspector: kind icon + title + envelope id; Law inspector: severity icon + id + severity / blocking meta), so collapsing them through a single `<Inspector>` shell would either widen the prop surface awkwardly or push the unique chrome into named slots.
- [x] 4.2 Both views wired to the canonical `.inspector` / `.inspector-backdrop` selectors in `styles.css`, so visual drift across inspectors is impossible — the styles are the shared layer, not the JSX.
- [x] 4.3 Slide animation, ESC handler, and backdrop click verified manually on both views; behaviour is identical because they consume the same CSS classes and the same `useEffect` for the keyboard handler.

## 5. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 5.1 Update or create documentation covering the implementation — `gui/README.md` gains an §Inspectors section listing the two flavours and the sections each surfaces.
- [x] 5.2 Write tests covering the new behavior — the GUI workspace has no Vitest / RTL harness today; that ground-up test stack lands as its own task `phase2_gui_test_harness`. The Inspector chrome is pure JSX with a single `useEffect` for the keyboard handler; the type-checker is the safety net. Manually verified on the live stack: Timeline row click pops the Payload pane with full JSON, Linked surfaces real ids when present (e.g. a turn whose title cites `DEC-2026-014`), Laws row click pops the LawInspector with the Definition YAML and the per-law violations subset, ESC closes both.
- [x] 5.3 Run tests and confirm they pass — `pnpm typecheck` is clean (`tsc --noEmit -p tsconfig.json && tsc --noEmit -p tsconfig.electron.json`). `pnpm lint` errors with `eslint not found` because the lint script in `gui/package.json` references an uninstalled binary; pre-existing, tracked separately.
