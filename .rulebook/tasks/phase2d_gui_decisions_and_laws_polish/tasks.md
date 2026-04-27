## 1. Decisions — Show superseded toggle
- [ ] 1.1 Add a `showSuperseded` boolean state defaulting to `false`
- [ ] 1.2 Render a `btn`/`btn--ghost` button in `view__actions` matching the design — label "Show superseded" with a checkmark prefix when active
- [ ] 1.3 Filter `rows` by `status !== "superseded"` when the toggle is off; pass through when on

## 2. Decisions — supersedes / superseded inline tags
- [ ] 2.1 In each card head, render `<Tag tone="warn">supersedes {d.supersedes}</Tag>` when `d.supersedes` is set
- [ ] 2.2 Render `<Tag>superseded → {d.supersededBy}</Tag>` when `d.status === "superseded"` and the backend exposes `superseded_by` (today the field is absent — the tag stays hidden)
- [ ] 2.3 Apply `is-superseded` class to the card when status is superseded (already styled at `styles.css` line 881)

## 3. Decisions — supersession chain (graceful)
- [ ] 3.1 When `d.chain` is present (array of `{id, title, state}`), render the horizontal `supersede-chain` element using the existing CSS selectors
- [ ] 3.2 Each chain node uses `supersede-node` with `is-current` / `is-old` classes per `state`
- [ ] 3.3 Arrows between nodes use the `arrow-right` Icon
- [ ] 3.4 When `d.chain` is undefined, render nothing — never fabricate a chain

## 4. Laws — table header row
- [ ] 4.1 Render a `law-row` with `is-header` styling at the top of the table (background bg-2, mono uppercase): ID / Title / Severity / Action / Scope / Rate · 7d
- [ ] 4.2 Match the column grid template to the existing `.law-row` selector (lines 946–955)

## 5. Laws — SeverityBar + Action column
- [ ] 5.1 Replace the severity `Tag` with `<SeverityBar severity={law.severity} />` followed by the severity label
- [ ] 5.2 Add an Action cell rendering `<Tag tone="critical">block</Tag>` when `law.blocked` is true, else `<Tag>observe</Tag>`
- [ ] 5.3 Re-order the existing cells so the grid reads ID, Title, Severity, Action, Scope, Rate — matches the design

## 6. Laws — explicit sort
- [ ] 6.1 Sort `laws` by `violations_7d` descending before rendering (today the order comes from the backend's BTreeMap; explicit sort makes UI behavior independent of backend storage)

## 7. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 7.1 Update or create documentation covering the implementation — extend `gui/README.md` Decisions + Laws sub-sections with the new toggles, columns, and graceful chain handling
- [ ] 7.2 Write tests covering the new behavior — RTL: toggling "Show superseded" reveals/hides superseded rows; a decision with `chain` present renders the chain; a decision with `chain: undefined` renders no chain element; Laws table renders the header row, the Action cell, and the SeverityBar atom
- [ ] 7.3 Run tests and confirm they pass — `pnpm exec tsc --noEmit -p tsconfig.json`, `pnpm test`, `pnpm lint`
