# Container HEALTHCHECK passing does not mean the worker's consume loop is progressing

**Category**: observability
**Tags**: cortex, observability, docker, analysis:cortex-platform-2026-07

## Description

cortex-graph-worker showed Docker-level "healthy" continuously for 2+ weeks because its HEALTHCHECK only probes that /healthz responds. The actual Nexus-consumer loop silently stopped processing events on 2026-06-27 (after a run of transient Nexus errors) and stayed dead for 8 days undetected at the container level. It was cortex-api's own application-level /v1/health freshness check (a 600-second no-activity threshold, admin_health.rs) that correctly flagged it "degraded" — Docker's healthcheck and the application's own freshness signal disagreed, and the freshness signal was the one telling the truth.

## When to Use

Designing HEALTHCHECK directives (or any liveness probe) for a background worker/consumer process.

## When NOT to Use

Stateless request/response services where "the HTTP listener answers" is actually equivalent to "the service works."
