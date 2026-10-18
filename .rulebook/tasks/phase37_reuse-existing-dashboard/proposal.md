# Proposal: phase37_reuse-existing-dashboard

## Why
The standalone runtime introduced a separate GUI instead of preserving the existing Cortex dashboard. The user explicitly requires the original layout while allowing functional and integration changes.

## What Changes
Reuse the existing gui renderer, shell, design tokens and navigation in the standalone executable. Connect supported local memory, history, relationships, classification, audit and feedback operations with exact scope and same-origin token authentication. Preserve local data and honestly identify unavailable legacy-only capabilities. Build and embed assets without Electron, Docker or Node at runtime; validate actual desktop/mobile layout and upgrade the installed release.

## Impact
- Affected specs: Existing dashboard layout preserved in standalone runtime
- Affected code: gui, cortex-local embedded web assets and local projections, build/CI wiring
- Breaking change: Building the renderer from source requires frontend tooling; running the installed executable remains self-contained.
- User benefit: Restore the expected dashboard while keeping the simplified local architecture
