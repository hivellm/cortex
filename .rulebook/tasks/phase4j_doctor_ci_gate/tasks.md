## 1. Make target
- [ ] 1.1 Add a `doctor` target to `Makefile` invoking `cargo run -p cortex-ops -- doctor-consistency` with the local-stack env vars
- [ ] 1.2 Document the env-var layout in the Makefile comment block

## 2. GitHub Actions workflow
- [ ] 2.1 Author `.github/workflows/doctor.yml` (or extend an existing workflow) that brings up `docker-compose` for Vectorizer + Nexus + Synap + Meilisearch
- [ ] 2.2 Run a synthetic bootstrap (3 tiny temp repos) so the doctor has data to compare against
- [ ] 2.3 Run `cortex-ops doctor-consistency --json` and upload the JSON report as a workflow artifact
- [ ] 2.4 Fail the workflow on non-zero exit code

## 3. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 3.1 Update or create documentation covering the implementation — cross-reference from `docs/specs/08-fulltext-indexer.md` to the workflow file plus a `### CI gate` paragraph naming the trigger and the artifact location
- [ ] 3.2 Write tests covering the new behavior — a workflow dry-run via `act` or equivalent, plus a unit test asserting the make target shells the right cargo command
- [ ] 3.3 Run tests and confirm they pass — workflow passes against a seeded stack
