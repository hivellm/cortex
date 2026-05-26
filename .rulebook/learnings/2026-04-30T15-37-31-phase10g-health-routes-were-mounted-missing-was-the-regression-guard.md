# phase10g — health routes were mounted; missing was the regression guard
**Source**: manual
**Date**: 2026-04-30
**Related Task**: phase10g_health_route_registration
**Tags**: health, router, doctor, phase10g, regression-guard
The 2026-04-29 audit hit empty bodies on `/v1/health/*` against the live daemon. Investigation showed the routes ARE registered in `cortex-api/src/http.rs::build_router_with` whenever a `DashboardState` is supplied — and the boot path always supplies one. So the audit was likely against a stale binary that predated the original phase8b/8c/8d registrations.

Phase10g closed the regression-guard gap rather than re-registering routes:
1. **Comprehensive integration test** (`every_v1_health_route_is_mounted_on_router_with_dashboard`) hits all five routes (`/v1/health`, `/freshness`, `/divergence`, `/versions`, `/config`) on a single `build_test_router()` and asserts each returns 200 with a JSON body. Future refactors that drop the `merge()` of `health_router` will fail this single test instead of triggering empty-body symptoms in production.
2. **Legacy `/healthz` passthrough** pinned in a second test so the workers' default URL probes keep working.
3. **`cortex-ops doctor` probe** added: when `CORTEX_API_URL` is set, doctor curls each `/v1/health/*` URL and reports red on missing routes. Operator-side surface for the same regression class.
4. **Spec 16 route inventory** consolidated under "Health view route inventory (phase10g)" so the spec lists the five routes explicitly.
5. **CHANGELOG entry** flags that operators rolling forward from a pre-phase10g cortex-api MUST relaunch the binary — the `/v1/health/*` routes don't back-port into an already-serving process.

Lesson: when an audit reports empty endpoint bodies, check if the routes are registered AND check the deployed binary's age before assuming a registration bug. The fix here was 99% test coverage + 1% doctor probe; no router code changed.