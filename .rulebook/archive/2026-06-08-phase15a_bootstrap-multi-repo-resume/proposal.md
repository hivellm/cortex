# Proposal: phase15a_bootstrap-multi-repo-resume

Source: `docs/analysis/rework/04-architecture.md` Phase C.1; `docs/analysis/rework/opus5.7/03-recommendation.md` Phase C.1.

## Why

Bootstrap today walks one repo at a time and overwrites its checkpoint. Adding the 17 HiveLLM repos to Cortex via the current path takes 4-8 hours and any kill/restart is a full rerun. Phase 13b shipped `EnvelopeProducer` + accumulating `producer_checkpoints`. This task uses that primitive to ship multi-repo bootstrap with resume-after-kill correctness.

## What Changes

- Bootstrap walker accepts `--repos <slug,slug,slug>` and writes one `producer_checkpoints` row per (repo, slug) pair.
- Per-repo parallelism via Tokio task pool (configurable, default = `num_cpus`).
- Resume from the last checkpointed `event_id` per repo on restart.
- New `cortex-ops bootstrap status` reports per-repo progress (events emitted, last_event_id, ETA).

## Impact

- Affected specs: `docs/specs/03-bootstrap.md` § Multi-repo + § Resume.
- Affected code: `crates/cortex-cli/src/bootstrap/walker.rs`, `crates/cortex-cli/src/bin/cortex-bootstrap.rs`, `crates/cortex-cli/src/bin/cortex-ops.rs`.
- Breaking change: NO. `--repos` is additive; single-repo invocation unchanged.
- User benefit: 17 repos bootstrap in <60 min on 16 cores; restart is cheap.
