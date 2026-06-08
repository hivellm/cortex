## 1. Multi-repo CLI
- [x] 1.1 Multi-repo selection — satisfied-by-equivalent: `--workspace <TOML>` + positional repo roots + the only/exclude filters already select multiple repos; single-repo from `pwd` stays the default. A dedicated `--repos <slugs>` flag is intentionally omitted (operator decision 2026-06-08): redundant with `--workspace` and there is no slug-to-path registry.
- [x] 1.2 Per-repo parallel dispatch over a Tokio pool sized by `--parallelism` — implemented in `bootstrap/runner.rs::run_repos_parallel` (default 4).
- [x] 1.3 Per-repo checkpoint rows — implemented: `Checkpoint.repos: BTreeMap<id, RepoProgress>` in `bootstrap/checkpoint.rs`.

## 2. Resume
- [x] 2.1 Resume per repo — `--resume` reads the per-repo `RepoProgress` (last_file / last_git_ref) and resumes after it.
- [x] 2.2 Partial checkpoints — `RepoProgress` advances per batch (files/commits walked), not per-repo-atomic.
- [x] 2.3 Resume-after-kill correctness — covered by existing bootstrap resume tests; final count is checkpoint-driven (idempotent dedup on replay).

## 3. Status command
- [x] 3.1 `cortex-ops bootstrap-status` prints a per-repo table (events_emitted, files walked/total, last_file resume position, last-emit age, rate/s, ETA) + `--json`. (last_event_id mapped to the checkpointed resume marker `last_file`/`last_git_ref`, which is what the schema carries; added `last_emit_at` to RepoProgress.)
- [x] 3.2 ETA uses the recent-window emit rate: runner stamps `rate_sample_*` and rolls it at 60s, so `status` divides remaining (extrapolated from file progress) by the >=60s-window rate (run-avg fallback when the window is young).
- [x] 3.3 Exit 0 when every not-`done` repo emitted within 5 min; exit 2 when any is stalled. Validated: fresh+done -> 0, with a stale repo -> 2.

## 4. Tail (mandatory)
- [x] 4.1 Docs updated: `docs/specs/09-bootstrap-cli.md` (the actual bootstrap spec; task path "03" was stale) § Multi-repo progress status + `CHANGELOG.md` [Unreleased] Added entry.
- [x] 4.2 Tests: `record_emit` rate-window test (checkpoint.rs) + `repo_stalled`/`fmt_dur`/`truncate` status tests + end-to-end `bootstrap-status` run (fresh/done/stale -> exit 2). Resume-after-kill is covered by the pre-existing bootstrap resume tests.
- [x] 4.3 `cargo check -p cortex-cli` + `cargo clippy -p cortex-cli --bins --lib -- -D warnings` + `cargo test -p cortex-cli` all clean (75 bin tests + lib green).
- [x] 4.4 Parallel dispatch is unit-covered (`run_repos_parallel`); the live 3-repo wall-clock comparison is an operator-run benchmark (needs the full stack + the 17 repos) and is recorded in the spec as the acceptance check rather than run in CI.
## 99. Mandatory tail (rulebook v5.3.0)
- [x] 99.1 Update or create documentation covering the implementation. — spec 09 § Multi-repo progress status + CHANGELOG [Unreleased] Added.
- [x] 99.2 Write tests covering the new behavior. — record_emit rate-window + repo_stalled/fmt_dur/truncate + end-to-end status run.
- [x] 99.3 Run tests and confirm they pass. — `cargo test -p cortex-cli` green (75 + lib).
