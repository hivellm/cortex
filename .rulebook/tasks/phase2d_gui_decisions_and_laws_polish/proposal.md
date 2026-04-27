# Proposal: phase2d_gui_decisions_and_laws_polish

## Why

The Decisions register is missing the design's "Show superseded" toggle and the visual supersession chain (line 119–131 of `views-mid.jsx` shows current/old nodes connected by arrows). Without the toggle the list silently filters out superseded decisions — a user can't tell whether the data is empty or hidden. Without the chain the user can't see the lineage that justified the current decision.

The Laws table is rendered as plain rows without a header (compare with `views-mid.jsx` lines 184–202). Severity is shown as a Tag instead of the design's three-segment `SeverityBar`. There is no Action column distinguishing `block` vs `observe`. These are visual gaps that make the table read as a flat list instead of a sortable matrix.

Source: `gui/assets/views-mid.jsx` (DecisionsView lines 61–137; LawsView lines 178–205).

## What Changes

### Decisions
- Add `Show superseded` toggle in the view actions; defaults to off. Filters `rows` by `status !== "superseded"` when off.
- Each card head surfaces inline `supersedes <id>` and `superseded → <id>` tags (both already present in the response shape; render only when the field exists).
- When the backend supplies `chain` (planned in phase2h, but optional today), render the horizontal `supersede-chain` element using the styles already in `styles.css` lines 917–941. When `chain` is absent, render nothing — no fake chain.

### Laws
- Add a header row to the law table (ID / Title / Severity / Action / Scope / Rate · 7d) using the styles from `views-mid.jsx`.
- Replace the severity `Tag` with the new `SeverityBar` atom (port lands in phase2a).
- Add an Action column showing `block` (critical tone) or `observe` (default tone), driven by the `blocked` boolean.
- Sort rows by violations_7d descending — already the implicit default but make it explicit.

## Impact

- Affected specs: none (renders existing data shapes).
- Affected code: `gui/src/views/Decisions.tsx`, `gui/src/views/Laws.tsx`, `gui/src/styles.css` (no new selectors expected — design CSS already shipped).
- Breaking change: NO.
- Depends on: phase2a (SeverityBar atom). Optionally consumes phase2h decision `chain` field when available.
- User benefit: Decisions become navigable through their lineage, Laws table reads like a sortable matrix instead of a list.
