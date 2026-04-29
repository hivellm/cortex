## 1. Crate scaffolding
- [ ] 1.1 NEW `crates/cortex-retention/` (Cargo.toml, src/lib.rs, src/main.rs)
- [ ] 1.2 Add to workspace Cargo.toml members
- [ ] 1.3 Wire dependencies: `cortex-storage`, `cortex-core` (event types), Vectorizer SDK, `clap`, `tokio`, `tracing`, `chrono`, `ulid`, `serde_json`

## 2. Sweep engine
- [ ] 2.1 Define `SweepPlan { now: DateTime<Utc>, fp32_to_pq_age_days: i64, pq_to_binary_age_days: i64, batch_size: u32 }`
- [ ] 2.2 Define `TierTransition { event_id, kind, from_tier, to_tier, reason }` and emit on Synap `cortex.events.enriched` with `kind="retention.tier_transition"`
- [ ] 2.3 Implement `sweep_collection(client, source_collection, dest_collection, encoding, cutoff_ts) -> Result<SweepCounts>` that paginates via Vectorizer SDK, batches re-encode, upsert, delete
- [ ] 2.4 Idempotent guard: before delete, verify destination has `event_id` (Vectorizer `get_by_id`)
- [ ] 2.5 Per-collection error accounting: continue past per-record failures, capture into `errors[]`, fail the sweep only if error rate > 5%

## 3. CLI
- [ ] 3.1 `cortex-retention sweep [--time-travel RFC3339] [--dry-run] [--batch-size N] [--collection cortex.turn.fp32]`
- [ ] 3.2 `--dry-run` prints what would move without mutating anything
- [ ] 3.3 Default config from `cortex.toml` `[retention]` section (`fp32_to_pq_days = 30`, `pq_to_binary_days = 365`, `batch_size = 256`)
- [ ] 3.4 Exit code: 0 on success, 1 on hard failure, 2 if another sweep is already running

## 4. Bookkeeping
- [ ] 4.1 SQL: `INSERT INTO retention_sweeps(sweep_id, started_at, status='running')` at start
- [ ] 4.2 On completion: `UPDATE retention_sweeps SET finished_at, records_demoted, records_dropped, tier_transitions_json, status='success'`
- [ ] 4.3 On crash: orphan row left as `status='running'`; subsequent run detects via `started_at` older than 1 h and clears with `status='abandoned'`
- [ ] 4.4 Helper `metadata::list_recent_sweeps(limit)` for the dashboard (used by 9i)

## 5. Spec / docs
- [ ] 5.1 NEW `docs/specs/19-retention.md` documenting the sweep contract, config keys, and observability surface
- [ ] 5.2 Update `docs/specs/02-storage-layout.md` §"Quantization & tier sweep" to reference spec 19 and remove placeholder wording

## 6. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 6.1 Update or create documentation covering the implementation
- [ ] 6.2 Write tests covering the new behavior
- [ ] 6.3 Run tests and confirm they pass
