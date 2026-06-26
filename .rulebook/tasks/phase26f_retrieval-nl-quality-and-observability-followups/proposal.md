# Proposal: phase26f_retrieval-nl-quality-and-observability-followups

## Why

Source: docs/analysis/cortex/12-live-audit-2026-06-09.md

phase26e §1 removed ~90% of the Vectorizer additive bloat (stable vector id +
dedupe migration, ~169k→~15.7k cortex vectors) and §2/§3 made the bundle-cache
hit-rate and TRUE pre-thinking latency observable on the adapter `/healthz`.
That work surfaced four residual items that are genuinely larger or
operator-gated and were deferred out of phase26e:

- **Turns are still raw-JSON embedded.** `cortex-cortex-turns` (~33k vectors)
  was NOT dropped in the §1.3 dedupe because turns are session-derived, not
  bootstrap-refillable. Raw-cosine probing showed the top turn vectors are raw
  JSON (`"...2>&1\nfind ..."`) scoring ~0.23 — the NL projection never replaced
  them. Re-embedding turns cleanly needs a session-archive replay
  (`cortex-claude-archive`), then a drop+re-embed of the turns collection
  through the stable-id embedder.

- **Static summaries are raw-JSON-ish, capping retrieval quality.** The static
  classifier `summary` is `"{kind} in {location}: {120-char raw-JSON snippet}"`
  (observed live: `"artifact in cortex: {\"text\":...}"`). The embedding model
  caps at ~0.42–0.45 raw cosine for the audit query on this text, so dedupe
  alone cannot raise the top-1 score (phase26e §1.4 found 0.50 unreachable by
  purging). The real lever is clean NL summaries — either better static NL
  projection per kind, or LLM classification (CLI/SDK mode), which is currently
  blocked (`CORTEX_CLASSIFIER_MODE=static`, no logged-in CLI session wired into
  the worker).

- **GUI dashboard `pre_thinking_p95_ms` series still misleads.** phase26e §3
  exported the TRUE `pre_thinking_latency_ms{p50,p95,p99}` on the adapter
  `/healthz`, but the GUI series is still the p95 of generic envelope
  `duration_ms` (phase26d gap C). The cortex-api dashboard series + GUI need to
  repoint to the adapter health source.

- **Live confirmation of §2/§3 counters.** The phase26e §2/§3 surfaces are
  verified by unit test; live confirmation (cache_hit increments; latency p95
  <200ms under load) needs the host adapter daemon redeployed with the new
  build and real hook traffic — an operator step that would disrupt an active
  session if done mid-flight.

## What Changes

### Turns clean re-embed
- Trigger / wire a `cortex-claude-archive` replay of historical sessions, drop
  `cortex-cortex-turns`, and re-embed through the stable-id embedder so turn
  vectors carry NL projection instead of raw JSON.

### NL summary quality
- Improve the static NL projection per kind so the `summary` is human-readable
  (no raw-JSON snippet), OR wire the CLI-mode classifier (local logged-in
  `claude` CLI, never the API) so summaries are LLM-generated; re-embed.
- Metric: top vector cosine for the audit query rises above the ~0.45 static
  ceiling.

### Dashboard series repoint
- Repoint the cortex-api `pre_thinking_p95_ms` dashboard series (and the GUI)
  to the adapter `/healthz` `pre_thinking_latency_ms` source, or add a new
  series, so the dashboard shows real bundle-assembly latency.

### Live verification
- After the host adapter daemon is redeployed, confirm `pre_thinking_cache_hit_total`
  increments on a repeated query and `pre_thinking_latency_ms.p95` < 200ms under
  normal session load.

## Impact
- Affected specs: spec 05 (classifier), spec 06 (embedder), spec 12 (pre-thinking), spec 26 (dashboard series)
- Affected code: cortex-workers (classifier NL projection / CLI mode, claude_archive replay), cortex-api (dashboard series source), gui
- Breaking change: NO
- User benefit: turns become semantically searchable; retrieval scores clear the static-summary ceiling; the dashboard latency series is accurate; cache/latency observability confirmed live
