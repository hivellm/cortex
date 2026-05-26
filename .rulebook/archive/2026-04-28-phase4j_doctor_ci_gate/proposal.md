# Proposal: phase4j_doctor_ci_gate

## Why

`phase4d_indexing_consistency_doctor` shipped the doctor binary
(`cargo run -p cortex-ops -- doctor-consistency`). Wiring it into
CI as a deploy gate was carved out — CI changes need a
`docker-compose` smoke stack with seeded data, which is its own
operational task with separate review surface.

## What Changes

- `make doctor` target invoking the cargo command above with
  the env vars the local stack uses.
- GitHub Actions workflow step (or extension to an existing one)
  that:
  1. Brings up `docker-compose` (Vectorizer + Nexus + Synap +
     Meilisearch).
  2. Runs a tiny seeded bootstrap (3 synthetic repos).
  3. Runs `cortex-ops doctor-consistency --json` and pipes the
     report to a workflow artifact.
  4. Fails the workflow on non-zero exit.
- Documentation cross-reference from `docs/specs/08-fulltext-indexer.md`
  to the workflow file.

## Impact

- Affected specs: spec-08 (cross-reference + a `### CI gate`
  paragraph).
- Affected code:
  - `Makefile` — new `doctor` target
  - `.github/workflows/doctor.yml` (or extension to existing
    workflow)
- Breaking change: NO. New CI step.
- Depends on: phase4d (the binary must exist) + phase4h
  (Vectorizer + Nexus probes) so the doctor sees the full matrix.
- User benefit: structural drift caught before merge instead of
  weeks later.

## Source

- Carved out of `phase4d_indexing_consistency_doctor` items 5.1–
  5.3 (CI integration).
