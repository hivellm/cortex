# Spec: Health route registration

## ADDED Requirements

### Requirement: every health route is mounted

`build_dashboard_router` MUST register the five `/v1/health/*`
endpoints in addition to the dashboard routes. The router test
suite MUST exercise each one.

#### Scenario: GET /v1/health returns the overview body
Given a `cortex-api` boot with the lane seeded
When the GUI calls `GET /v1/health`
Then the response MUST be HTTP 200
And the body MUST be a JSON object with `overall ∈ {ok,degraded,
  down}`.

#### Scenario: GET /v1/health/freshness returns rows
Given the loader-metrics shim has stamped at least one row
When the GUI calls `GET /v1/health/freshness`
Then the response MUST be HTTP 200
And the body MUST be a JSON array (possibly empty).

### Requirement: doctor catches routing regressions

`cortex-ops doctor` MUST include a `health_routes` probe that
hits each of the five endpoints against `CORTEX_API_URL`. A
404 / 5xx / empty body MUST flip the probe to red.

#### Scenario: missing route is reported red
Given the daemon is patched to omit `/v1/health/divergence`
When the operator runs `cortex-ops doctor`
Then the `health_routes` probe MUST report red
And the report MUST name the missing route.
