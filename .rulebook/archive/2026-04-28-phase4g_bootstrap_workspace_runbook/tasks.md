## 1. Workspace template
- [x] 1.1 Author `bootstrap.workspace.toml.example` at the repo root with 17 `[[repo]]` entries, paths set to `${HIVE_ROOT}/<RepoName>` so operators replace a single variable
- [x] 1.2 Each entry sets `id` to the canonical HiveLLM repo name so the Cortex checkpoint key matches what the rest of the stack expects
- [x] 1.3 Add a leading comment block explaining the env-var substitution + how to copy the file to `bootstrap.workspace.toml`

## 2. Operations runbook
- [x] 2.1 Author `docs/operations/bootstrap-workspace.md` with the five-step sequence: clone repos → fill template → estimate → run → verify
- [x] 2.2 Document the verification queries (Vectorizer collections list + Meili `/stats` indexes list + Nexus `MATCH (r:Repo) RETURN r.name`)
- [x] 2.3 Cross-reference from `docs/specs/09-bootstrap-cli.md` §Workspace orchestration

## 3. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 3.1 Update or create documentation covering the implementation — the runbook itself satisfies item 2.1 of this task; spec-09 cross-reference satisfies 2.3
- [x] 3.2 Write tests covering the new behavior — a CI guard that lints the example TOML against `cortex_bootstrap::workspace::load_workspace` so a typo in the template fails CI before reaching the operator
- [x] 3.3 Run tests and confirm they pass — `cargo test -p cortex-bootstrap --test workspace -- bootstrap_workspace_example_loads` returns 0
