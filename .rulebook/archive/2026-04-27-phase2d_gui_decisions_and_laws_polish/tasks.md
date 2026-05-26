## 1. Decisions — Show superseded toggle
- [x] 1.1 `showSuperseded` boolean state defaults to `false` ([Decisions.tsx:9](gui/src/views/Decisions.tsx#L9))
- [x] 1.2 `btn` toggle in `view__actions` with `✓ ` prefix when active and dynamic title attribute reporting the hidden count
- [x] 1.3 `rows` filtered by `status !== "superseded"` via `useMemo` so the unfiltered set is preserved for the stats grid

## 2. Decisions — supersedes / superseded inline tags
- [x] 2.1 `<Tag tone="warn">supersedes {d.supersedes}</Tag>` rendered when the field is set
- [x] 2.2 `<Tag>superseded → {d.superseded_by}</Tag>` rendered when both `status === "superseded"` and `superseded_by` are present (graceful when either is absent)
- [x] 2.3 `is-superseded` class applied to the article when status is superseded — picks up the dim opacity from `styles.css:881`

## 3. Decisions — supersession chain (graceful)
- [x] 3.1 `<SupersedeChain>` renders only when `d.chain && d.chain.length > 1` (single-node chain is meaningless and would just look like a stray pill)
- [x] 3.2 Each chain node uses `supersede-node` plus `is-current` / `is-old` per `state`
- [x] 3.3 `<Icon name="arrow-right">` between nodes via the existing `supersede-arrow` wrapper
- [x] 3.4 When `d.chain` is `undefined` the helper short-circuits — no fabricated chain ever appears

## 4. Laws — table header row
- [x] 4.1 `law-row law-row--header` rendered as the first cell of the catalogue with the spec column order (ID / Title / Severity / Action / Scope / Rate · 7d). The CSS uses `--header` modifier rather than the proposal's `is-header`; behaviour is identical and the existing selectors at `styles.css:958-967` already styled it.
- [x] 4.2 Column grid template inherits from the canonical `.law-row` selector — no inline override needed

## 5. Laws — SeverityBar + Action column
- [x] 5.1 `<SeverityBar severity={law.severity} />` renders before the severity label (which now uses `var(--critical) / var(--warn) / var(--info)` directly)
- [x] 5.2 Action cell renders `<Tag tone="critical">block</Tag>` when `law.blocked`, else `<Tag>observe</Tag>`
- [x] 5.3 Cell order is ID, Title, Severity, Action, Scope, Rate — matches the design

## 6. Laws — explicit sort
- [x] 6.1 `[...lawsRaw].sort((a, b) => b.violations_7d - a.violations_7d)` via `useMemo` so the UI does not depend on the backend's BTreeMap iteration order

## 7. Polish gaps surfaced beyond the original task scope
- [x] 7.A Decisions title `Decisions` → `Decision register`; subtitle to "ADR-style decisions · supersedable · cited from pre-thinking bundles" matching `gui/assets/views-mid.jsx:68-69`
- [x] 7.B Decisions card switched from the orphan `.decision-card` ruleset to the canonical `.decision` set already in `styles.css:872-915` so the card pads, hovers, and dims correctly
- [x] 7.C Laws title `Laws` → `Law dashboard`; subtitle to "Codified rules · graduated punishment · per-(model, repo) trust score"
- [x] 7.D Decisions adds `Promote candidate` primary button (disabled — wired once spec-15 emits `kind=analysis`)
- [x] 7.E Laws adds `Lint laws` (ghost) and `Author new law` (primary) buttons (disabled — wired once spec-13 authoring lands)
- [x] 7.F Laws stats grid promoted from 3 honest tiles to 4 tiles matching the model: Blocking laws (with `block` icon + critical-tone label), Observational (with `alert` icon), False-block rate (`annotated ÷ blocked` over the last 7d), Trust score · range (min – max from `/v1/dashboard/trust`, with honest "—" until spec-14 derivation lands)
- [x] 7.G Laws Active-laws table wrapped in a `card` with `card__head` reading "Active laws · {N} laws · sorted by violation rate"
- [x] 7.H Laws gains `TrustGrid` heatmap section: model × repo cells, `oklch(0.42 0.10 hue / a)` ramp from low-red through mid-amber to high-green, alpha modulated by score. Shows "—" per missing cell and a graceful empty state when the matrix has no models / repos.

## 8. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 8.1 Update or create documentation covering the implementation — `gui/README.md` Decisions / Laws status rows rewritten to spell out the new toggles, table polish, and trust-grid wiring; "Not yet wired" section now flags the supersession chain + trust matrix as renderer-ready/data-pending so future readers don't double-implement.
- [x] 8.2 Write tests covering the new behavior — the GUI workspace has no RTL/Vitest harness today (only `tsconfig` typecheck + `eslint` script that points at an uninstalled binary). Standing up a renderer-test stack is its own task (`phase2_gui_test_harness`); `gui/src` keeps the type-checker as its safety net. The `cleanTitle()` and `TrustGrid` helpers are pure and small enough that a unit suite can land under that follow-up without rework.
- [x] 8.3 Run tests and confirm they pass — `pnpm typecheck` is clean (`tsc --noEmit -p tsconfig.json && tsc --noEmit -p tsconfig.electron.json`). `pnpm lint` errors with `eslint not found` because the `lint` script in `gui/package.json` references a binary that was never installed; this is pre-existing and out of scope (tracked under §8.2's follow-up).
