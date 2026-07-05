## 1. Client-side rate-limited projection scheduler
- [ ] 1.1 Design a rate-limited/backoff scheduler for the step-3b
      semantic-edge projection in
      `crates/cortex-workers/src/graph/worker.rs` (token-bucket or
      fixed-delay batch throttle; new typed config knobs alongside
      `NexusConfig.projection_enabled`, ADR-016 style).
- [ ] 1.2 Implement the scheduler so projection write volume stays
      under the threshold that trips nexus#12, without weakening the
      existing endpoint-anchor / idempotency guarantees (phase15c
      §1.3).

## 2. Validate against the current Nexus version
- [ ] 2.1 Confirm the target Nexus version (`hivehub/nexus:2.3.4`,
      already pinned in `docker-compose.yml`) carries the phase25
      sequential-MATCH mitigation and the nexus#25
      edge-properties-on-MERGE fix; note the still-open nexus#11
      index-persistence-across-restart gap in the test plan.
- [ ] 2.2 Run the rate-limited scheduler against a sustained synthetic
      write load (backlog-drain scale, matching the phase15c
      reproduction) and confirm no sustained-write stall reproduces.

## 3. Staged rollout
- [ ] 3.1 Enable `CORTEX_GRAPH_PROJECTION_ENABLED=true` with the new
      rate limiter in a dev/staging environment first.
- [ ] 3.2 Monitor for nexus#12 symptoms (sustained CPU pin, write
      latency spike) over a soak period before promoting.
- [ ] 3.3 Promote to production (`docker-compose.yml`) once staging is
      clean.

## 4. Re-verify the dependent phase27 tasks
- [ ] 4.1 Re-verify `phase27b_graph-community-detection` §2.5 (cron
      worker) and §3 (MCP tool + dashboard surface) now that the
      architecture subgraph is non-empty.
- [ ] 4.2 Check `phase27c_graphrag-community-summaries` and
      `phase27e_idf-graph-seed-selection` for non-trivial output
      against the now-populated graph.

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
