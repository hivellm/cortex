## 1. Make target
- [x] 1.1 Add a `doctor-consistency` target to `Makefile` invoking `cargo run -p cortex-ops -- doctor-consistency` with the local-stack env vars (the existing `doctor` target stays as the liveness probe)
- [x] 1.2 Document the env-var layout in the Makefile comment block

## 2. GitHub Actions workflow
- [x] 2.1 Author `.github/workflows/doctor.yml` (or extend an existing workflow) that brings up `docker-compose` for Vectorizer + Nexus + Synap + Meilisearch
- [x] 2.2 Run a synthetic bootstrap (3 tiny temp repos) so the doctor has data to compare against
- [x] 2.3 Run `cortex-ops doctor-consistency --json` and upload the JSON report as a workflow artifact
- [x] 2.4 Fail the workflow on non-zero exit code

## 3. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 3.1 Update or create documentation covering the implementation — cross-reference from `docs/specs/08-fulltext-indexer.md` to the workflow file plus a `### CI gate` paragraph naming the trigger and the artifact location
- [x] 3.2 Write tests covering the new behavior — `crates/cortex-ops/tests/ci_doctor_gate.rs` asserts the make target shells the canonical cargo command and the workflow runs `--json` mode + uploads the `doctor-consistency-report` artifact + brings up docker compose + seeds bootstrap
- [x] 3.3 Run tests and confirm they pass — `cargo test -p cortex-ops --test ci_doctor_gate` is green; full live workflow run-against-stack happens on the next push (the CI environment does not exist on the dev host)
