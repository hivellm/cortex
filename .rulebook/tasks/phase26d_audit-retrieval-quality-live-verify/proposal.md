# Proposal: phase26d_audit-retrieval-quality-live-verify

## Why
Phase26c fixed bugs #8 (classifier summaries), #9 (bundle cache), and #10 (ADR status re-emit)
at the code level. Six verification steps require a live running stack (Synap stream, embedder,
Meilisearch, bootstrap run) and could not be executed in the dev environment. This task captures
those steps so they are not lost and get executed on the next container deploy.

## What Changes
No code changes — operational verification only.

## Impact
- Affected specs: docs/analysis/cortex/12-live-audit-2026-06-09.md (mark verified)
- Affected code: none
- Breaking change: NO
- User benefit: Confirms that the phase26c code fixes actually resolved the live-stack symptoms
  (low vector scores, pre-thinking latency spike, stale ADR statuses).
