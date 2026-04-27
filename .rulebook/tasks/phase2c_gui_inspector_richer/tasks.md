## 1. EventInspector — Payload section
- [ ] 1.1 Add `inspector__section` for "Payload (redacted)" rendering the selected event as JSON inside `<pre class="code-block">`
- [ ] 1.2 Use `JSON.stringify(ev, null, 2)` so output stays readable; mark missing optional fields with `null` in the printout (no fabricated values)
- [ ] 1.3 Body width is bounded by the inspector — `code-block` already wraps with horizontal scroll

## 2. EventInspector — Linked section
- [ ] 2.1 Detect DEC- / ANL- ids in `ev.title` via regex; collect into a list of links
- [ ] 2.2 Render each as a `violation-card`-style row with the id badge + a "switch to Decisions/Analysis" button
- [ ] 2.3 Empty state: "no linked decisions or analyses" — never invent links

## 3. Laws — row click + LawInspector
- [ ] 3.1 Make law rows in `LawsView` clickable; clicking sets a `selectedLawId` state
- [ ] 3.2 Render an Inspector (reuse the styles already in `styles.css` lines 749–809) with: head (severity icon + id + severity tag), Definition section (synthesized YAML from the row), 7-day stats (kv-list), Recent violations (filtered subset of `/v1/dashboard/violations`)
- [ ] 3.3 ESC and click-outside close the Inspector
- [ ] 3.4 Active row gets `is-active` class

## 4. Refactor — shared Inspector shell (when duplication crosses 80 lines)
- [ ] 4.1 Extract `gui/src/shell/Inspector.tsx` exporting a generic `<Inspector open onClose>{children}</Inspector>` shell — only when both Timeline and Laws would otherwise duplicate the head/body/backdrop wiring
- [ ] 4.2 Migrate Timeline + Laws to use the shared shell
- [ ] 4.3 Verify slide animation, ESC handler, and backdrop click still work in both views

## 5. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 5.1 Update or create documentation covering the implementation — extend `gui/README.md` with an "Inspector" sub-section listing the two flavors (event vs law) and what each surfaces
- [ ] 5.2 Write tests covering the new behavior — RTL: clicking a Timeline row shows the Payload `<pre>` containing the event id; clicking a row whose title contains "DEC-2026-014" shows a Linked card with that id; clicking a Laws row opens the LawInspector and ESC closes it
- [ ] 5.3 Run tests and confirm they pass — `pnpm exec tsc --noEmit -p tsconfig.json`, `pnpm test`, `pnpm lint`
