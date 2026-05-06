## 1. Inventory pass
- [ ] 1.1 Walk `.rulebook/archive/*/proposal.md` and extract the `Affected code:` bullet from each.
- [ ] 1.2 For each file path, run `git ls-files <path>` to confirm presence.
- [ ] 1.3 Produce `docs/analysis/rework/opus5.7/appendix/archived-tasks-audit.csv` with columns `task_id, status, affected_files, status_reason`.
- [ ] 1.4 Status taxonomy: `still-live`, `superseded-by-X` (cite the superseding task id), `dead-code-candidate`, `redundant`.

## 2. Categorise each task
- [ ] 2.1 Run `cortex-ops audit archived-tasks --inventory <csv> --output md`.
- [ ] 2.2 Output `archived-tasks-audit.md` summarising counts per status + per-status table.
- [ ] 2.3 Author reviews the categorisation in a PR; mismatches get re-categorised before §3.

## 3. Dead-code removal
- [ ] 3.1 For each `dead-code-candidate` file with author confirmation, delete the file + remove from `Cargo.toml` modules.
- [ ] 3.2 `cargo check --workspace` MUST pass after every deletion.
- [ ] 3.3 Per-deletion git commit cites the archived task id.

## 4. Tail (mandatory)
- [ ] 4.1 New `docs/analysis/rework/opus5.7/appendix/archived-tasks-audit.md` + `CHANGELOG.md` Removed.
- [ ] 4.2 Tests: `cargo test --workspace` post-removal MUST pass; LOC reduction MUST be ≥5%.
- [ ] 4.3 `cargo check --workspace && cargo clippy -- -D warnings && cargo test --workspace` clean.
## 99. Mandatory tail (rulebook v5.3.0)
- [ ] 99.1 Update or create documentation covering the implementation.
- [ ] 99.2 Write tests covering the new behavior.
- [ ] 99.3 Run tests and confirm they pass.
