---
name: cortex-audit
description: Inspect local memory provenance and revisions when asked to audit Cortex evidence.
---

# cortex-audit

Use `cortex_audit_list` for bounded scoped receipts/actions, `cortex_audit_get`
for exact audit IDs, and `cortex_audit_metrics` for evidence denominators,
unknowns, errors and latency. List defaults to 20 (maximum 100); metrics default
to 10000 (maximum 10000). UTC windows and include_archived bound the view.
Follow next_cursor to paginate; disclose sampled/omitted counts and truncation.

Read referenced sources with `cortex_get` at the recorded revision. Preserve
ID, hash, source and authority. Prepared receipts do not prove consumption;
captured exact Stop citations do not prove causal use, action compliance or
productivity. Missing identifiers remain unknown. No automated compliance or
enforcement service is implied by this evidence inspection.

`cortex_audit_retain` requires a UTC cutoff and previews up to 100 deliveries
by default. Use apply=true only with explicit archival intent; it archives
visibility without deleting canonical files, sources or reclaiming disk.
