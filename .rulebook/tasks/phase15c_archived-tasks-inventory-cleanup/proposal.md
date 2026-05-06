# Proposal: phase15c_archived-tasks-inventory-cleanup

Source: `docs/analysis/rework/opus5.7/03-recommendation.md` Phase C.0 + pre-Phase A patch #6.

## Why

`.rulebook/archive/` carries 117 archived tasks. The 4-doc and opus5.7 analyses both note this as the canonical signal of abstraction debt — features land in parallel forever, dead code accumulates, and nobody audits afterward. Without an inventory pass, Phase A's structural fixes risk colliding with stale code that was never removed.

## What Changes

- Inventory pass: read every `.rulebook/archive/*/proposal.md`, extract `Affected code:` lines, and produce a CSV mapping archived task → still-live files.
- Categorise each task: `still-live` (work landed and is current), `superseded-by-X` (Phase A/B trait replaces it), `dead-code-candidate` (work landed but the affected files are gone), `redundant` (multiple tasks shipped the same change).
- Delete dead-code candidates after author review.
- Output: `docs/analysis/rework/opus5.7/appendix/archived-tasks-audit.md`.

## Impact

- Affected specs: NONE (purely operational).
- Affected code: dead modules removed (target ~5-15% LOC reduction across `cortex-workers`).
- Breaking change: NO. Only removes truly-dead code.
- User benefit: codebase smaller, easier to reason about; Phase A's refactors stop tripping over stale shims.
