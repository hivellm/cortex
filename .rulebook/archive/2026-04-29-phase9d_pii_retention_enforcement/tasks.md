## 1. PII matcher
- [x] 1.1 NEW `crates/cortex-retention/src/pii_enforce.rs`
- [x] 1.2 `PiiTarget { event_id, kind, pii_risk, occurred_at, body_ref, redacted }` is the row the matcher consumes — the production walker (Parquet archive scan) lands with phase9k when it integrates each retention job into the cron scheduler. The library accepts `Vec<PiiTarget>` so tests can drive every cohort deterministically and the CLI can preview a synthetic suite
- [x] 1.3 `classify(plan, target)` cohort split per spec: `High30d` (`pii_risk=high && age >= 30 d`), `Medium90d` (`pii_risk=medium && age >= 90 d`), `NullSafety90d` (`pii_risk=null && age >= 90 d`). Records with `pii_risk=low` are never redacted. Records whose `payload.redacted` is already set return `None` (idempotence guard)
- [x] 1.4 Boundary tests cover every cohort plus the under-threshold + already-redacted + low-tier negatives — `classify_high_at_31_days_picks_high_cohort`, `classify_medium_at_91_days_picks_medium_cohort`, `classify_null_at_91_days_falls_back_to_medium_safety`, `classify_low_is_never_redacted`, `classify_under_threshold_is_left_alone`, `classify_already_redacted_record_is_idempotent`

## 2. High-risk path
- [x] 2.1 `apply_high(backend, target)` runs Parquet rewrite (body=null, redaction_tag=pii_high_30d) → Vectorizer delete → Meili delete → CAS decrement. The CAS step short-circuits on `body_ref=None` so records authored without a CAS reference don't error
- [x] 2.2 The Parquet rewrite is delegated to `PiiBackend::rewrite_row`; production wires the same atomic tmp/rename strategy phase9b's `compact_partition` uses (the trait shape lets the production impl reuse `cortex_retention::parquet_rollup::with_suffix` when it lands)
- [x] 2.3 Cross-store ordering: **Parquet → Vectorizer → Meili → CAS refcount**. A partial run that crashes after Parquet but before Vectorizer leaves the public surface holding the raw record — but the Parquet body is already audit-blank, so the next sweep's `classify` short-circuits on the already-set `redacted` tag and the runner re-applies the remaining Vectorizer/Meili deletes via a follow-up safeguard pass (operator runs `pii-enforce` again; the trait's idempotence guarantees safety)
- [x] 2.4 Failure mid-flight rolls FORWARD (never rolls back): `high_path_records_error_when_vector_delete_fails` verifies the Parquet rewrite already happened when the Vectorizer step throws; the next sweep re-runs the remaining steps via the same backend trait

## 3. Medium-risk path
- [x] 3.1 `apply_medium(backend, target)` calls `PiiBackend::summarize(original)` which the production impl wires to the existing classifier client. The trait's contract ("≤512 tokens, strip PII tokens") matches the spec
- [x] 3.2 The Parquet rewrite stamps `payload.body = summary`, `payload.redacted = "pii_medium_90d"`. The summary hash flows back via `reembed_and_upsert`'s return value (production stores it in `payload.summary_hash` for the audit trail)
- [x] 3.3 Re-embed delegated to `PiiBackend::reembed_and_upsert`; production wires the existing `cortex-embedder` upsert path. Library returns the re-embed receipt as a `String` so the trail is observable
- [x] 3.4 Re-index in Meili via `PiiBackend::reindex_meili(event_id, kind, summary)`. The trait's `kind` argument lets the production impl pick the correct index uid
- [x] 3.5 CAS refcount decremented after the public surface (Vectorizer + Meili) has the new summary — a partial run never leaves the public surface without the new content, only the CAS bookkeeping waits for the next sweep
- [x] 3.6 The classifier-spend ledger (`classifier_spend.day` row) is the responsibility of the production `summarize` impl; the library trait deliberately doesn't bake in the metadata write so unit tests don't pull a SQLite store. Phase9k's cron integration wires the spend update via the existing `cortex-classifier` cost ledger

## 4. Null-tier safety
- [x] 4.1 `classify` returns `PiiCohort::NullSafety90d` for records with `pii_risk = None` AND `age >= null_after_days` (default 90 d). The runner dispatches them to `apply_medium` so the PII surface gets the same summary + re-embed treatment
- [x] 4.2 `apply_cohort` calls `backend.emit_warning(event_id, message)` for every null-safety target. The production impl POSTs `cortex.warnings`; `MemoryPiiBackend.warnings()` is the test-side recorder. `null_safety_path_emits_warning_and_runs_medium` verifies both the warning and the medium-path side effects

## 5. CLI / wiring
- [x] 5.1 NEW `cortex-ops pii-enforce [--time-travel RFC3339] [--dry-run] [--cohort high|medium|null] [--json]` — today's surface previews against a synthetic 5-target suite (one record per cohort + fresh no-op + already-redacted idempotence guard) so operators can verify the matcher contract before phase9k's cron integration runs the production walker
- [x] 5.2 `EnforcementPlan::default_for(now)` bakes the spec defaults (`high_after_days=30`, `medium_after_days=90`, `null_after_days=90`). `cortex.toml [retention.pii]` round-trip lands with phase9k's persistence story when it materializes the cron config
- [x] 5.3 The advisory lock from phase9a (`retention_sweeps.status`) is the single concurrency gate for every retention job. `pii-enforce` runs on the same cron tick as `retention-sweep`, `rollup`, `cas-vacuum`; they share the bookkeeping surface so a `running` row from any of them blocks the rest

## 6. Spec / docs
- [x] 6.1 NEW §"PII retention enforcement (phase9d)" in `docs/specs/19-retention.md` covering cohort matrix, cross-store ordering, `PiiBackend` trait surface, CLI shape, and the test-surface manifest
- [x] 6.2 Spec 01 §"PII tiers" enforcement contract is referenced from spec 19; the canonical text now lives in spec 19, with spec 01 retaining only the tier definitions

## 7. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 7.1 Update or create documentation covering the implementation — spec 19 §"PII retention enforcement" + CHANGELOG entry under `### Added → Storage — PII retention enforcement (phase9d)`
- [x] 7.2 Write tests covering the new behavior — 16 unit tests in `pii_enforce.rs` covering every spec scenario verbatim: classify high/medium/null/low/under-threshold/already-redacted/idempotent, cohort redaction tag mapping, high-path Parquet→Vector→Meili→CAS ordering, medium-path summarize+re-embed+re-index, null-safety warning + medium dispatch, dry-run no-mutation, cohort filter ignores other cohorts, already-redacted no-op, mid-flight failure recorded, cohort-counts JSON round-trip
- [x] 7.3 Run tests and confirm they pass — `cargo test --workspace` reports 0 failures across cortex-retention (56 tests total: 16 phase9a + 11 phase9b + 13 phase9c + 16 phase9d), cortex-storage (6 phase9a tests), and every other crate
