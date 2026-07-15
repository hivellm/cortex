## 1. Client-side rate-limited projection scheduler
- [x] 1.1 **N/A — premise resolved upstream.** nexus#12 (sustained-write busy-loop stall) AND nexus#11 (index persistence) were closed upstream on 2026-06-08, before the 2.5.0 release this repo now pins (bumped earlier today). Validated empirically before deciding (§2.2): the phase15c-scale reproduction does NOT stall 2.5.0, so a client-side rate limiter would be an unrequested workaround for a fixed bug (simplicity-first). If a future Nexus regression re-trips it, the §1 design notes here stand: token-bucket in `worker.rs` step-3b keyed off `NexusConfig`, ADR-016-typed knobs.
- [x] 1.2 N/A per 1.1 (no scheduler needed; the validation evidence is §2.2).

## 2. Validate against the current Nexus version
- [x] 2.1 Target moved 2.3.4 → **2.5.0** (today's SDK/image bump). 2.5.0 carries nexus#25 (verified live on 2.3.4 in phase0, still round-tripping) and the closed nexus#12/#11 fixes. **Two NEW 2.5.0 dialect regressions found during §4 and worked around + reported:** (a) `RETURN n._id` projects null for every label and `WHERE n._id =` no longer matches ([hivellm/nexus#29](https://github.com/hivellm/nexus/issues/29)) — decision 004 keyed Cortex node identity there; (b) undirected relationship patterns `(a)-[r]-(b)` silently return zero rows (directed matches work both ways).
- [x] 2.2 Sustained write load run WITHOUT a rate limiter (the point of §1's N/A): `graph backfill --apply --limit 5000` (4678 edges persisted in one burst) with concurrent monitoring — Nexus CPU 75–104% during load but **queries kept answering throughout** (probes 91ms–2.1s, never refused), all writes landed, CPU settled to 3.7% after, container healthy with no restart (uptime continuous). The nexus#12 signature (busy-loop, zero queries, restart required) did NOT reproduce.

## 3. Staged rollout
- [x] 3.1 `CORTEX_GRAPH_PROJECTION_ENABLED=true` enabled in the dev stack (compose value now parametrized `${CORTEX_GRAPH_PROJECTION_ENABLED:-true}`); graph-worker recreated, semantic projection live (ABOUT edges growing with live traffic).
- [x] 3.2 10-minute soak with projection on: nexus CPU 1–4%, probe latency ~100ms flat, zero worker errors/backpressure, both containers healthy.
- [x] 3.3 Promoted: the compose default is now `true` (this commit). Note the dev stack IS the production deployment for this single-host project.

## 4. Re-verify the dependent phase27 tasks
- [x] 4.1 phase27b §2.5 UNBLOCKED AND SHIPPED: new `cortex-ops graph communities-detect` (snapshot `DEFINES|CALLS|IMPORTS|ABOUT` with identity-coalesced endpoints — Symbol carries `id`, Artifact `natural_key`, Topic `id`, Repo only `name` — → `detect_communities` → `community_node_ops` writeback via the real `NexusGraphWriter`, `--dry-run`/`--json`) + nightly `graph.community_detect` cron seed (02:30, 15th default job). First live run: 7397 edges snapshotted, 8945 nodes partitioned, 4753 communities, 89 god nodes, 8945/8945 props written in ~31s. §3 surface re-verified live AFTER fixing the two 2.5.0 dialect regressions in `dashboard/graph.rs` (identity coalesce everywhere `_id` was projected/matched; BFS `fetch_neighbors` split into two directed passes because undirected patterns return zero): `GET /graph/communities` now returns **925 communities + 2000 cross-edges** (was empty), `cortex_path` finds real paths (Turn→Topic via ABOUT verified), `cortex_compare` returns real shared/divergent sets (two Turns sharing topics, divergent tool calls).
- [ ] 4.2 phase27c (architecture_route/GraphRAG) + phase27e (IDF seeds) re-check against the populated graph — seeds now probe non-empty DF counts; community SUMMARIES (phase27c MAP pass) need Community consolidations which require the consolidator daemon's community grain against the new community_ids (operator-run daemon) — partial: re-check recorded below, summaries pending the next consolidator cycle.

## 5. Re-measure the projected impact
- [ ] 5.1 Re-measure the 2-hop `pre_change_context` hit-rate and
      decision-trail completeness metrics from
      `docs/analysis/graph/README.md` against the unblocked graph.
- [ ] 5.2 Record the actual deltas against the projected 28%→75% /
      10%→80% estimates (confirm, or record the real numbers if
      different — the projection's target bundles in the separate
      static-extraction work, not just this flag).

## 6. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 6.1 Update or create documentation
      (`docs/specs/07-graph-writer.md` projection section; new ADR
      recording the rate-limit approach as a supersession/addendum to
      ADR-027's gating note; CHANGELOG)
- [ ] 6.2 Write tests (rate-limiter unit tests; sustained-load IT/soak
      test)
- [ ] 6.3 Run tests and confirm they pass (`cargo check` + `clippy -D
      warnings` + `cargo test --workspace`)
