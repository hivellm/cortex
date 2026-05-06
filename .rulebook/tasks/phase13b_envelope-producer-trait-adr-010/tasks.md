## 1. ADR-010
- [ ] 1.1 `rulebook_decision_create` ADR-010 — "EnvelopeProducer trait + accumulating checkpoint". Status `proposed`.
- [ ] 1.2 Trade-off: refactor cost ~3 days × 4 producers; gain is kill-resume correctness and zero-cost adapter onboarding.

## 2. Trait + checkpoint table
- [ ] 2.1 New module `crates/cortex-workers/src/producer/` with `mod.rs`, `trait.rs`, `ctx.rs`, `checkpoint.rs`.
- [ ] 2.2 `EnvelopeProducer` trait per the proposal signature. `ProducerCtx` carries handles to Synap, Meili, Nexus, Vectorizer plus the producer-name string.
- [ ] 2.3 New SQLite table `producer_checkpoints`. Migration in `cortex-storage::metadata::apply_phase13b_schema`.
- [ ] 2.4 `record_checkpoint(name, scope, last_event_id, last_occurred_at)` writes append-only rows. `latest_checkpoint(name, scope)` returns the most recent row.
- [ ] 2.5 Unit tests: 5 cases (no checkpoint, single checkpoint, multiple per scope, multiple per name, scope discrimination).

## 3. Migrate 4 producers
- [ ] 3.1 `bootstrap` (walker → `impl EnvelopeProducer`). Each emit advances the checkpoint.
- [ ] 3.2 `claude-archive` ingest → `impl EnvelopeProducer`.
- [ ] 3.3 `topic-cards emit` → `impl EnvelopeProducer`.
- [ ] 3.4 `consolidator emit` → `impl EnvelopeProducer`.
- [ ] 3.5 Per-producer IT confirms `producer_checkpoints` accumulates one row per emit batch.

## 4. Resume-after-kill IT
- [ ] 4.1 Spawn bootstrap against a fixture corpus of 10k events.
- [ ] 4.2 After 30% emitted, send SIGKILL.
- [ ] 4.3 Restart bootstrap with the same args; assert it resumes from the last checkpointed `event_id` (no duplicates, no gaps).
- [ ] 4.4 Final event count matches the input corpus exactly.

## 5. Tail (mandatory)
- [ ] 5.1 Update `docs/specs/03-bootstrap.md` + new `docs/specs/24-producer-trait.md` + `CHANGELOG.md`.
- [ ] 5.2 Tests: §2.5 + §3.5 × 4 + §4.
- [ ] 5.3 `cargo check --workspace && cargo clippy -- -D warnings && cargo test --workspace` clean.
## 99. Mandatory tail (rulebook v5.3.0)
- [ ] 99.1 Update or create documentation covering the implementation.
- [ ] 99.2 Write tests covering the new behavior.
- [ ] 99.3 Run tests and confirm they pass.
