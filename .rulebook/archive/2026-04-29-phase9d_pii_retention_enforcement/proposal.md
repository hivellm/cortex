# Proposal: phase9d_pii_retention_enforcement

## Why

Spec 01 §"PII tiers" defines three classes:

- `pii_risk = "high"` — drop raw payload at 30 d (CAS blob deleted,
  Parquet row blanked but kept for audit).
- `pii_risk = "medium"` — re-summarize at 90 d (replace raw with a
  compressed summary, drop CAS blob).
- `pii_risk = "low"` — keep indefinitely.

Spec 02 §"Quantization & tier sweep" reaffirms this in the same daily
sweep window. None of the enforcement is implemented. Today every event
is treated as `low` regardless of classifier output, which creates a
compliance gap (PII data is retained beyond its intended window) and a
cost gap (medium-risk records that could be summarized stay verbose).

This task lands the enforcement layer for both tiers, including the
LLM-driven summarization for medium-risk records (delegated to Sonnet
through the existing classifier client to keep the model contract in
one place).

## What Changes

1. NEW subcommand `cortex-retention pii-enforce`.
2. Reads classifier-tagged events from the Parquet archive + Synap +
   Vectorizer; matches on `payload.pii_risk`.
3. **High @ 30 d**:
   - delete CAS blob referenced by `payload.body_ref` (decrement
     refcount, leave the actual vacuum to 9c),
   - rewrite the Parquet row with `payload.body = null`,
     `payload.redacted = "pii_high_30d"`,
   - drop the Vectorizer record (any tier),
   - delete the Meili document.
4. **Medium @ 90 d**:
   - call Sonnet via the classifier client with a "compress to ≤512
     tokens, strip names/emails/IDs" prompt,
   - replace `payload.body` with the summary,
   - re-embed and replace the Vectorizer vector,
   - re-index in Meili with the summary,
   - keep an audit trail (`payload.redacted = "pii_medium_90d"`,
     `payload.summary_hash`).
5. Bookkeeping in `retention_sweeps.tier_transitions_json.pii_enforce`.
6. `--time-travel`, `--dry-run`, advisory lock — same shape as 9a/9b/9c.
7. Hard requirement: if the classifier never tagged a record (legacy
   data with `pii_risk = null`), the runner MUST treat it as `medium`
   and re-summarize, never as `low`. Rationale: defaulting to `low`
   would silently retain unclassified PII forever.

## Impact

- Affected specs: `docs/specs/01-event-schema.md` §"PII tiers"
  (clarify enforcement contract), `docs/specs/19-retention.md`.
- Affected code: NEW `crates/cortex-retention/src/pii_enforce.rs`,
  `crates/cortex-classifier/src/client.rs` (re-summarize call),
  Parquet rewriter helper.
- Breaking change: NO. Storage shape unchanged; record contents may
  shrink.
- User benefit: actually enforces the documented PII contract; closes
  a compliance hole; keeps medium-tier records useful via summary.
