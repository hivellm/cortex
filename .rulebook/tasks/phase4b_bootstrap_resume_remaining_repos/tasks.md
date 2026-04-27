## 1. Workspace config schema
- [ ] 1.1 Define `BootstrapWorkspaceConfig` struct in `cortex-bootstrap` (id, path, optional `config_override` per repo)
- [ ] 1.2 Author `bootstrap.workspace.toml` template at the repo root listing the 17 Hive repo ids and example local paths
- [ ] 1.3 Parser rejects duplicate ids, missing paths, and non-git checkouts at load-time

## 2. Orchestrator entrypoint
- [ ] 2.1 Add `cortex-bootstrap --workspace <path>` flag in `cli.rs` (or split a `cortex-bootstrap-all` binary if preferred)
- [ ] 2.2 Pre-flight: for every configured repo, assert the path exists, is a git repo, and has a readable `cortex.toml`; abort with the full failure list if any fails
- [ ] 2.3 Iterate repos in config order calling `run_repo` per checkout; aggregate `RepoRunReport`s
- [ ] 2.4 Optional `--parallel N` flag wires through `run_repos_parallel` (already at runner.rs:253)
- [ ] 2.5 Print a summary table (id, events, files_dropped, duration, status) at the end

## 3. Resumable & idempotent
- [ ] 3.1 Per-repo checkpoint entry written via `write_atomic` after each repo completes (already supported by checkpoint format)
- [ ] 3.2 Bypass repos whose checkpoint reports `status = done` AND `last_git_ref` equals current `HEAD`
- [ ] 3.3 `--force` flag overrides the bypass; logs `info` per forced repo
- [ ] 3.4 Ctrl-C between repos leaves checkpoint consistent; the next invocation resumes at the first not-done repo

## 4. Run the missing repos
- [ ] 4.1 Author `bootstrap.workspace.toml` with all 17 repo paths on the user's machine (input from user)
- [ ] 4.2 Execute `cortex-bootstrap --workspace bootstrap.workspace.toml`
- [ ] 4.3 Verify in Vectorizer that every repo has at least `code` and `docs` collections (12 repos × 2 families minimum); recorded count matches `RepoRunReport.events_published`

## 5. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 5.1 Update or create documentation covering the implementation (extend spec-09 with a `## Workspace orchestration` section)
- [ ] 5.2 Write tests covering the new behavior (integration test with two tiny temp git repos, asserts both produce events and the checkpoint tracks both)
- [ ] 5.3 Run tests and confirm they pass (`cargo check -p cortex-bootstrap` → `cargo clippy -p cortex-bootstrap --all-targets -- -D warnings` → `cargo test -p cortex-bootstrap`)
