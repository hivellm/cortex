## 1. Routing
- [ ] 1.1 In `crates/cortex-api/src/dashboard.rs::build_dashboard_router`, register `/v1/health`, `/v1/health/freshness`, `/v1/health/divergence`, `/v1/health/versions`, `/v1/health/config`
- [ ] 1.2 Each route forwards to the existing `health::*` handler the GUI's vitest fixtures already exercise
- [ ] 1.3 `/healthz` (legacy) keeps working as a passthrough to `/v1/health`

## 2. Doctor probe
- [ ] 2.1 In `crates/cortex-cli/src/ops/doctor.rs`, add a `health_routes` probe that hits each of the five endpoints against `CORTEX_API_URL`
- [ ] 2.2 The probe reports red when any returns < 200 or empty body

## 3. Tests
- [ ] 3.1 `crates/cortex-api/src/dashboard.rs` integration test asserts every health route returns 200 with a JSON object
- [ ] 3.2 `cortex-ops doctor` smoke test fails when one of the routes is unmounted (synthetic regression)

## 4. Spec / docs
- [ ] 4.1 `docs/specs/16-dashboard.md` §"Health view" lists the five routes explicitly under "Backend endpoints"
- [ ] 4.2 CHANGELOG entry noting the daemon must be relaunched to pick the routing up

## 5. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 5.1 Update or create documentation covering the implementation
- [ ] 5.2 Write tests covering the new behavior
- [ ] 5.3 Run tests and confirm they pass
