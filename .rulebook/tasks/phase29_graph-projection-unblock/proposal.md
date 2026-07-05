# Proposal: phase29_graph-projection-unblock

Source: docs/analysis/graph/README.md (headline numbers); docs/analysis/cortex/11-platform-vision-analysis.md
(§2.2 Backend Integration Health, §5.4 Risk Register, Phase A1 "Fix
Nexus #12 sustained-write stall"); `.rulebook/decisions/027-graph-community-detection-in-process-rust-leiden-over-a-nexus-snapshot-gated-on-the-semantic-projection.md`;
`.rulebook/knowledge/anti-patterns/high-value-feature-gated-permanently-off-behind-an-unfixed-upstream-dependency-bug.md`
(tag `analysis:cortex-platform-2026-07`).

## Why

`CORTEX_GRAPH_PROJECTION_ENABLED` is hardcoded `"false"` for the
`cortex-graph-worker` service in `docker-compose.yml` (the code default
is `true`; the compose override is the actual live posture). The flag
gates step "3b" (semantic-edge projection — CALLS, IMPORTS, DEFINES,
RETURNS, SUPERSEDES, CONTRADICTS, EMITTED_BY, ABOUT, ANSWERED_BY,
CITES, MENTIONS_FILE, RELATES_TO) in
`crates/cortex-workers/src/graph/worker.rs`. It was flipped off in
phase15c (2026-06-09) after live testing showed that even
structural-only projection at scale pinned Nexus at 100% CPU during a
backlog-drain burst — the anchor-node + per-edge write volume trips
nexus#12, a sustained-write stall in the upstream Nexus graph DB.
ADR-027 (2026-06-21) confirms the live Nexus graph still has zero
architecture/semantic edges as a direct result, and
`phase27b_graph-community-detection`'s tasks.md §0/§2.5 record the
community-detection worker as built-and-unit-tested but explicitly "⏸
blocked" on this exact flag for its live cron/writeback;
`phase27c_graphrag-community-summaries` inherits the same block as
phase27b's direct prerequisite. (`phase27e_idf-graph-seed-selection`'s
own proposal lists "Prereq: none" — it is not formally gated the same
way, but its IDF seed-scoring has a far richer graph to rank over once
semantic edges are live.) This is already captured as a first-class
anti-pattern in the knowledge base
(`high-value-feature-gated-permanently-off-behind-an-unfixed-upstream-dependency-bug`,
2026-07-05): a shipped capability sitting dark behind an unowned
upstream fix with no committed timeline, with that same entry
recommending exactly the client-side mitigation this task proposes.

Two things have changed since the flag was flipped off that make a
client-side mitigation worth attempting now rather than continuing to
wait: the Nexus container is already pinned to `hivehub/nexus:2.3.4`
(`docker-compose.yml`), which per `docs/specs/07-graph-writer.md`
already carries the phase25 sequential-MATCH mitigation (avoids the
O(n²) cartesian-product edge lookup on Nexus ≥2.3.1) and the nexus#25
fix (inline relationship properties now persist through MERGE,
previously silently dropped on 2.3.2). Neither of those addresses
nexus#12 itself — the sustained-write-stall bug is still open and
undocumented as fixed anywhere in this repo — but they remove two
confounding failure modes that would otherwise make a rate-limited
retest hard to interpret. One open caveat to carry into the retest:
Nexus 2.3.4 does not persist `CREATE INDEX`-created indexes across a
container restart (nexus#11) — the graph worker's periodic 5-minute
schema re-ensure timer covers this operationally, but a test plan must
account for it rather than assume the phase25 indexes survive a Nexus
restart untouched.

`docs/analysis/graph/README.md` estimates that turning the semantic
layer live is a necessary step toward its projected 2-hop
`pre_change_context` hit-rate lift (~28%→~75%) and `decision_lookup`
doc-trail completeness lift (~10%→~80%) — but that analysis's target
numbers bundle in a separate, not-yet-built static-extraction pass
(Tree-sitter/Markdown-derived IMPORTS/CALLS/MENTIONS/LINKS_TO edges) on
top of the existing semantic projection this task unblocks. Unblocking
the flag is necessary but not sufficient to fully reach 75%/80%; this
task re-measures the real deltas rather than assuming the analysis's
target is hit outright.

## What Changes

- Design and implement a client-side rate-limited/backoff scheduler for
  the semantic-edge projection step (worker.rs step 3b), so write
  volume to Nexus stays under the threshold that triggers nexus#12,
  instead of waiting on an upstream fix with no committed timeline.
- Validate the scheduler against the live `hivehub/nexus:2.3.4`
  container under a sustained synthetic write load, accounting for the
  nexus#11 index-persistence caveat, and confirm no stall reproduces.
- Stage the rollout: dev/staging first with
  `CORTEX_GRAPH_PROJECTION_ENABLED=true` and the new rate limiter,
  monitor for nexus#12 symptoms, then promote to production
  (`docker-compose.yml`).
- Once live, re-verify `phase27b_graph-community-detection` §2.5 (cron)
  and §3 (surface), and check whether
  `phase27c_graphrag-community-summaries` /
  `phase27e_idf-graph-seed-selection` now produce non-trivial output
  against the populated graph.
- Re-measure the `docs/analysis/graph/README.md` 2-hop hit-rate and
  decision-trail-completeness metrics against the unblocked (but not
  yet statically-augmented) graph and record the actual deltas.

## Impact

- Affected specs: `docs/specs/07-graph-writer.md` (projection
  kill-switch section, worker wiring); new `graph` module spec added by
  this task.
- Affected code: `crates/cortex-workers/src/graph/worker.rs` (step 3b
  scheduling), `crates/cortex-config` (new rate-limit knobs alongside
  `projection_enabled`), `docker-compose.yml` (flag + rollout),
  `phase27b_graph-community-detection` §2.5/§3,
  `phase27c_graphrag-community-summaries`,
  `phase27e_idf-graph-seed-selection` (live-verification only, no code
  change expected there).
- Breaking change: NO — enables previously-shipped-but-dormant code
  behind an already-existing flag.
- User benefit: unblocks the phase27 GraphRAG/community-detection
  chain's live value; the single highest-leverage retrieval-quality
  lever identified in the current platform analysis.
