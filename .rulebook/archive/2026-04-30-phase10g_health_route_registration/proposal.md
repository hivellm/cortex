# Proposal: phase10g_health_route_registration

## Why

The audit hit empty bodies on `/v1/health`, `/v1/health/freshness`,
`/v1/health/divergence`, `/v1/health/versions`, `/v1/health/config`
against the live daemon. The handlers are implemented (the GUI
Health tab consumes them and there are vitest fixtures for them)
but the running cortex-api does not register them on its router.
Symptom: every `health*` API call from the GUI returns blank,
forcing the operator back to grep + cargo logs to know if a
subsystem is alive.

This is a routing miss, not a missing-feature gap. The fix is
single-file (`build_dashboard_router`) plus the daemon binary
must be released afterwards so the running instance picks the
new wiring up.

## What Changes

1. `build_dashboard_router` mounts the five `/v1/health/*` routes
   alongside the existing `/v1/dashboard/*` set.
2. A smoke test in `crates/cortex-api/src/dashboard.rs` asserts
   the router answers each route with HTTP 200.
3. Doctor probe added so `cortex-ops doctor` checks the routes
   are reachable on the configured `CORTEX_API_URL` — a missed
   registration in a future refactor surfaces as a doctor red.
4. CHANGELOG note + restart instruction in the dashboard README
   so operators rolling forward from a pre-phase10g daemon know
   to bounce the binary.

## Impact

- Affected specs: `docs/specs/16-dashboard.md` §"Health view"
  (clarify the route inventory).
- Affected code: `crates/cortex-api/src/dashboard.rs` (router
  builder + tests), `crates/cortex-cli/src/ops/doctor.rs`.
- Breaking change: NO. Pure routing fix.
- User benefit: the Health tab in the GUI populates again; the
  doctor surfaces routing regressions before the operator opens
  the GUI.
