# phase4j — guard the CI gate by grep-asserting the workflow + Makefile shape
**Source**: manual
**Date**: 2026-04-28
**Related Task**: phase4j_doctor_ci_gate
**Tags**: phase4j, ci, github-actions, makefile, doctor
When the doctor binary's CI gate is wired through a Makefile target + a GitHub Actions workflow, the cheapest "did anyone forget to update this when the cargo path moved" guard is a `cargo test`-level test that reads `Makefile` and `.github/workflows/doctor.yml` and grep-asserts:

1. The `.PHONY` declaration includes the new target name (so `make` re-runs when a stale file with that name exists).
2. The Makefile recipe shells the canonical `cargo run -p cortex-ops -- doctor-consistency` command (loose match on whitespace).
3. The workflow runs the cargo command in `--json` mode (the artifact upload only makes sense for the structured report, not the markdown table).
4. The workflow uploads the artifact under the exact name referenced from the spec doc (`doctor-consistency-report`).
5. The workflow brings up `docker compose up` AND runs `cortex-bootstrap --workspace` to seed the stack — without seeding, the doctor reports "archive empty" and the gate becomes vacuous.

The Live workflow run against a real CI environment can't be exercised from a dev host, but these grep guards catch the failure modes that actually fire when someone refactors the cargo path or the artifact name without updating the spec / Makefile in lockstep.