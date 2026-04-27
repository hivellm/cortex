# Proposal: phase2a_gui_atoms_and_sidebar

## Why

Two design atoms drafted in `gui/assets/atoms.jsx` were never ported (`SeverityBar`, `Bars`) and one was ported but never used (`Sparkline`). The Sidebar lost the design's "Repos · N indexed" group plus the per-nav-item count pill — both are honest and trivial given `/v1/dashboard/overview` already returns `recent_repos` + `events_total`. Memory cards lack the design's `#topic` styling and solid repo tag. Footnote: today the GUI looks 30% emptier than the design reference because of these missing pieces, and the missing atoms gate the next two tasks (Laws polish needs `SeverityBar`; Timeline stats need `Sparkline`).

Source: `gui/assets/atoms.jsx`, `gui/assets/app.jsx` (Sidebar block), `gui/assets/views-mid.jsx` (MemoryView).

## What Changes

- Port `SeverityBar` and `Bars` atoms from `gui/assets/atoms.jsx` to `gui/src/atoms/`. Keep the same exported names.
- Wire the existing `Sparkline` atom into at least one consumer (Sidebar mini-trend or stat tile background).
- Sidebar gets a "Repos · N indexed" group rendered from `overview.recent_repos`. Each repo row is clickable and toggles the global `repo` filter.
- Sidebar nav items get a count pill: Decisions count, Memory count, Sessions count, Tools count — all derived from existing endpoints (`/v1/dashboard/overview` + `/decisions` + `/sessions` + `/tools/stats`).
- Memory card adopts `#topic` chip style and a solid repo tag (matches `views-mid.jsx` line 50–53).

## Impact

- Affected specs: none — design parity only.
- Affected code: `gui/src/atoms/`, `gui/src/shell/Sidebar.tsx`, `gui/src/views/Memory.tsx`, `gui/src/styles.css` (extend `.memory-topic` + add `.repo-group` styles).
- Breaking change: NO.
- User benefit: sidebar surfaces the same workspace facts as the design (repos + counts), Laws/Timeline tasks unblock once `SeverityBar`/`Sparkline` are in place, Memory matches the prototype's visual language.
