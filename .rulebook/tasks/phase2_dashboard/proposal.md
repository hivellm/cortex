# Proposal: phase2_dashboard

## Why

Phase 1 closed the capture → processing → retrieval loop, but everything ships as JSON over HTTP. Operators have no way to *see* what the system is capturing, no surface to author Laws or Decisions, and no UI to debug retrieval quality. The dashboard is the first human-facing surface — without it, Cortex is observable only by other code, and the governance loop (laws, violations, trust scores) stays invisible to the team running it.

The visual design is no longer hypothetical: a complete React-18 prototype with all seven views, the Inspector drawer, the Tweaks panel, and the entire token system sits at [`gui/assets/`](../../../gui/assets/) (`Cortex Dashboard.html` plus seven JSX/CSS files). This task ports that prototype to a production Vite + React + TypeScript SPA, swaps the `MOCK` constant for live `cortex-api` fetchers, and lights up the `cortex-api/src/dashboard/` backend the SPA needs.

## What Changes

- Promote the prototype in `gui/assets/` to a production SPA under [`gui/`](../../../gui/) (Vite + React 18 + TypeScript). Keep the prototype as `gui/assets/` so reviewers can diff the production port against the design source.
- Reuse the prototype's CSS verbatim — `styles.css` is already a strong design system (oklch tokens, dark/light themes, accent-hue picker, density slider). **No Tailwind**.
- Replace the prototype's `MOCK` with TanStack Query fetchers + an SSE hook. Query types match the shapes in `gui/assets/data.js` so the API contract is already documented.
- Backend endpoints on `cortex-api`: `/v1/dashboard/overview`, `/v1/dashboard/timeline/stream` (SSE), `/v1/dashboard/memory`, `/v1/dashboard/decisions`, `/v1/dashboard/laws`, `/v1/dashboard/analyses`, `/v1/dashboard/tools/stats`, `/v1/dashboard/trust`, `/v1/dashboard/rum`.
- Auth via single API key (v1) with an OIDC hook stubbed.
- Filters (repo / model / topic / severity / time) reflected in URL query strings — the prototype already has the chip-bar pattern.
- Tweaks panel ships as a power-user surface (theme + accent hue + density + sidebar collapse + SSE toggle) — already implemented in the prototype.
- Lighthouse a11y ≥90 on the Timeline view.

## Impact

- **Affected specs:** [`docs/specs/16-dashboard.md`](../../../docs/specs/16-dashboard.md); the spec's "Reuse Vectorizer's scaffold" decision becomes "Port the in-tree prototype" — update the Decisions section accordingly. The spec's "Graph explorer is a wrapper, not a rewrite" becomes "Inline SVG graph renderer" — the prototype already ships a custom SVG that is simpler and self-contained.
- **Affected code:** new `gui/` SPA (Vite + React + TS, raw CSS from the prototype); new `cortex-api/src/dashboard/` module + SSE stream wiring; new `cortex-api` CLI sub-command `cortex admin issue-api-key --scope dashboard`.
- **Breaking change:** NO — greenfield.
- **User benefit:** humans can see what Cortex captured, audit retrieval, author laws, and supervise the governance loop without piping JSON through curl. The prototype's Tweaks panel makes the dashboard tunable per operator without forking CSS.

## Source

[`docs/specs/16-dashboard.md`](../../../docs/specs/16-dashboard.md) · in-tree prototype at [`gui/assets/`](../../../gui/assets/) · depends on specs 11 (🟢) + 14 (Trust score, future) · architecture §5.6.
