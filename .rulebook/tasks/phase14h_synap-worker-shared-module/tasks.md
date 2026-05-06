## 1. Shared module
- [ ] 1.1 New `crates/cortex-workers/src/synap_worker/` with `mod.rs`, `trait.rs`, `runtime.rs`, `metrics.rs`, `dead_letter.rs`.
- [ ] 1.2 `SynapWorker` trait per the proposal signature.
- [ ] 1.3 `run(worker)` loop: subscribe → poll → handle → ack → checkpoint. Retry on transient errors with exponential back-off (max 5 retries). Permanent errors land in `dead_letter`.
- [ ] 1.4 Cursor checkpointing via `producer_checkpoints` table from phase13b (re-uses the Phase A primitive).
- [ ] 1.5 8 unit tests covering the loop (happy path, transient retry, permanent dead-letter, checkpoint resume, graceful shutdown, double-subscribe rejected, lag metric, dead-letter counter).

## 2. Migrate 4 workers
- [ ] 2.1 Embedder consumer → `impl SynapWorker for EmbedderWorker`.
- [ ] 2.2 Fulltext consumer → `impl SynapWorker for FulltextWorker`.
- [ ] 2.3 Graph consumer → `impl SynapWorker for GraphWorker`.
- [ ] 2.4 Classifier consumer → `impl SynapWorker for ClassifierWorker`.
- [ ] 2.5 Per-worker IT exercising one envelope through the shared runtime.
- [ ] 2.6 Line-count assertion: every migrated file shrinks by >50% (CI grep gate).

## 3. Centralised metrics
- [ ] 3.1 `cortex_synap_worker_lag{worker}` gauge updated every poll.
- [ ] 3.2 `cortex_synap_worker_dead_letter_total{worker, reason}` counter.
- [ ] 3.3 Doctor `cortex-ops doctor synap-workers` prints lag + dead-letter rate per worker.

## 4. Tail (mandatory)
- [ ] 4.1 Update `docs/specs/00-architecture.md` + `CHANGELOG.md`.
- [ ] 4.2 Tests: §1.5 + §2.5 × 4 + §3.3 doctor IT.
- [ ] 4.3 `cargo check --workspace && cargo clippy -p cortex-workers -- -D warnings && cargo test -p cortex-workers` clean.
## 99. Mandatory tail (rulebook v5.3.0)
- [ ] 99.1 Update or create documentation covering the implementation.
- [ ] 99.2 Write tests covering the new behavior.
- [ ] 99.3 Run tests and confirm they pass.
