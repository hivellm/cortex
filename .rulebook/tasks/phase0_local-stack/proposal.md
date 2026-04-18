# Proposal: phase0_local-stack

## Why

No worker can be written or tested without the four backend services running locally. This task stands up a one-command dev stack (`make up`) that every other MVP task depends on, with pinned versions so behavior is reproducible across contributors.

## What Changes

- `docker-compose.yml` bringing up Vectorizer, Nexus, Synap, Meilisearch with fixed image tags.
- Healthchecks, volumes, ports, env vars.
- `Makefile` targets: `up`, `down`, `logs`, `reset`, `smoke`.
- Seed init scripts (create default collections / constraints / indexes) idempotent on re-run.
- `.env.example` documenting every env var consumed by the stack.
- Smoke script: publish one event → assert it lands in Parquet + Synap.

## Impact

- **Affected specs:** [`docs/specs/03-local-stack.md`](../../../docs/specs/03-local-stack.md); runtime prerequisite for every other MVP task.
- **Affected code:** `docker-compose.yml`, `Makefile`, `scripts/seed-*.sh`, `.env.example`.
- **Breaking change:** NO — greenfield.
- **User benefit:** contributors can run the whole system with `make up` + `make smoke` in under 5 minutes.

## Source

`docs/specs/03-local-stack.md` · depends on spec 02 · PRD FR-3.
