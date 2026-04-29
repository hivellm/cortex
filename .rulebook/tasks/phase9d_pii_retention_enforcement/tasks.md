## 1. PII matcher
- [ ] 1.1 NEW `crates/cortex-retention/src/pii_enforce.rs`
- [ ] 1.2 `enumerate_targets(now)` → iterator over `(event_id, kind, pii_risk, occurred_at)` straight from the Parquet archive (read-only)
- [ ] 1.3 Cohort split: `high && age >= 30d` vs `medium && age >= 90d` vs `null && age >= 90d` (treat-as-medium safety net)
- [ ] 1.4 Unit tests across the boundary days for each cohort

## 2. High-risk path
- [ ] 2.1 `redact_high(target)` performs: CAS refcount-- → Parquet row rewrite (`body=null`, `redacted="pii_high_30d"`) → Vectorizer delete → Meili delete
- [ ] 2.2 Parquet rewrite uses the same atomic tmp/rename strategy as 9b
- [ ] 2.3 Cross-store ordering: Parquet → Vectorizer → Meili → CAS refcount (so a partial run never leaves the public surface holding raw data)
- [ ] 2.4 Failure inside the cross-store sequence rolls forward, never back: re-running converges

## 3. Medium-risk path
- [ ] 3.1 `summarize_medium(target)` calls `cortex-classifier` with a `pii_compress` prompt (≤512 tokens, strip PII tokens)
- [ ] 3.2 Replaces `payload.body` with the summary, sets `payload.redacted = "pii_medium_90d"`, records `summary_hash`
- [ ] 3.3 Re-embed via the embedder worker (reuse `cortex-embedder` API), upsert into Vectorizer
- [ ] 3.4 Re-index in Meili with the summary
- [ ] 3.5 Decrement CAS refcount on the original body
- [ ] 3.6 Per-call cost ledger update (`classifier_spend.day` row)

## 4. Null-tier safety
- [ ] 4.1 Records with `pii_risk = null` and age ≥ 90 d enter the medium path
- [ ] 4.2 Emit a `cortex.warnings` event for every such record so the team can audit classifier coverage

## 5. CLI / wiring
- [ ] 5.1 `cortex-retention pii-enforce [--time-travel RFC3339] [--dry-run] [--limit N] [--cohort high|medium|null]`
- [ ] 5.2 `cortex.toml [retention.pii]` (`high_after_days=30`, `medium_after_days=90`, `null_after_days=90`)
- [ ] 5.3 Advisory lock keyed `("pii-enforce")`

## 6. Spec / docs
- [ ] 6.1 Add §"PII enforcement" to `docs/specs/19-retention.md`
- [ ] 6.2 Reference back from `docs/specs/01-event-schema.md` §PII tiers

## 7. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 7.1 Update or create documentation covering the implementation
- [ ] 7.2 Write tests covering the new behavior
- [ ] 7.3 Run tests and confirm they pass
