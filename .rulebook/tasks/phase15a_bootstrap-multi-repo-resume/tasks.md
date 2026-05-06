## 1. Multi-repo CLI
- [ ] 1.1 Extend `cortex-bootstrap` with `--repos <comma-sep-slugs>` flag. Default behaviour with no flag stays single-repo from `pwd`.
- [ ] 1.2 Internally dispatches one `EnvelopeProducer::produce` per repo over a Tokio task pool sized by `--parallelism` (default `num_cpus`).
- [ ] 1.3 Each repo's emit advances its own `producer_checkpoints` row keyed by `(producer="bootstrap", scope=repo_slug)`.

## 2. Resume
- [ ] 2.1 On startup, read `latest_checkpoint("bootstrap", repo)` for each repo and resume from that `last_event_id`.
- [ ] 2.2 Resume respects partial checkpoints (per-batch, not per-repo).
- [ ] 2.3 SIGKILL IT: kill at 30% emitted; restart; final count matches input corpus exactly across all repos.

## 3. Status command
- [ ] 3.1 New `cortex-ops bootstrap status` prints a table: per-repo `events_emitted`, `last_event_id`, `last_emit_at`, ETA.
- [ ] 3.2 ETA uses average emit rate over the last 60s window.
- [ ] 3.3 Exit 0 when all repos report progress within the last 5 min; exit 2 when any is stalled.

## 4. Tail (mandatory)
- [ ] 4.1 Update `docs/specs/03-bootstrap.md` + `CHANGELOG.md`.
- [ ] 4.2 Tests: §2.3 resume-after-kill IT + §3 status table snapshot.
- [ ] 4.3 `cargo check --workspace && cargo clippy -- -D warnings && cargo test -p cortex-cli` clean.
- [ ] 4.4 Live smoke: bootstrap 3 repos in parallel; total wall time < single-repo wall time × 1.2.
## 99. Mandatory tail (rulebook v5.3.0)
- [ ] 99.1 Update or create documentation covering the implementation.
- [ ] 99.2 Write tests covering the new behavior.
- [ ] 99.3 Run tests and confirm they pass.
