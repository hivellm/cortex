# Proposal: phase2c_gui_inspector_richer

## Why

Two of the design's most useful debugging surfaces are missing: the EventInspector's "Payload (redacted)" code-block (so the user can read the raw envelope JSON), and the EventInspector's "Linked" section (so they can jump from a turn to the decision/analysis it cited). Laws have no Inspector at all — the design opens a LawInspector with the law's frontmatter, 7-day stats, and recent violations. Today the user can see *that* a tool call happened, but not *what fields* it carried, and a violation row dead-ends instead of opening the law that fired.

Source: `gui/assets/app.jsx` lines 102–166 (EventInspector) and 172–241 (LawInspector).

## What Changes

- EventInspector grows two sections:
  - **Payload (redacted)** — a `<pre class="code-block">` rendering the raw envelope JSON for the selected event. Backend currently exposes a flat shape via `/v1/dashboard/timeline/recent`; we render exactly what we have (id, kind, t, repo, session_id, model, title, detail). Honest about which fields are missing.
  - **Linked** — when the title text matches a known DEC- / ANL- prefix, render a clickable card that switches the view to Decisions/Analysis filtered by that id. Falls back to "no links yet" when nothing matches (no fake links).
- Laws view gets a row click → opens LawInspector with the law's metadata (id, title, severity, scope, applies, violations_7d, rate, detector, remediation), a code-block showing a synthesized YAML frontmatter for the law, and the violations_for_law subset of `/v1/dashboard/violations`.
- Inspector slide-in animation reused; ESC and click-outside both close.

## Impact

- Affected specs: none.
- Affected code: `gui/src/views/Timeline.tsx` (extend Inspector), `gui/src/views/Laws.tsx` (add LawInspector + row click), possibly extract a shared `gui/src/shell/Inspector.tsx` if both views grow it.
- Breaking change: NO.
- User benefit: debugging a turn no longer requires `curl /v1/dashboard/timeline/recent`; clicking a violation surfaces the law it broke.
