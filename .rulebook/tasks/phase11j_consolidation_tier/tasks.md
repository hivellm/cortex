## 1. Kind::Consolidation + payload

- [ ] 1.1 Add `Consolidation` variant to `Kind` enum in `crates/cortex-core/src/events.rs`
- [ ] 1.2 Add `ConsolidationPayload`, `ConsolidationGrain`, `ConsolidationDepth`, `ConsolidationScope`, `TimeSpan` types
- [ ] 1.3 Update `Kind::schema_stem()` and any Display/Debug impls covering the new variant
- [ ] 1.4 Add 4 unit tests in events.rs covering: serde round-trip, schema_stem mapping, grain discriminator, source_event_count clamp when source_event_ids exceeds inline cap
- [ ] 1.5 Update `crates/cortex-core/src/validate.rs` (or equivalent) to validate the new payload (title length, summary bounds, grain/scope discriminator match)

## 2. cortex-consolidator crate

- [ ] 2.1 Create `crates/cortex-consolidator/` skeleton (Cargo.toml, lib.rs, deps: cortex-core, cortex-storage, anthropic-sdk, hdbscan, tokio, serde, anyhow, ulid)
- [ ] 2.2 `src/templates/` — three prompt templates (session.md, topic.md, decision_trace.md); each declares input format, max output bytes, takeaways count
- [ ] 2.3 `src/summariser.rs` — abstract `Summariser` trait with `Haiku45` + `Opus47` impls; per-call cost tracking; retry on rate-limit
- [ ] 2.4 `src/producer/session.rs` — Session producer: input = all envelopes for a `session_id`; output = `Kind::Consolidation` with grain=Session
- [ ] 2.5 `src/producer/topic.rs` — Topic producer: HDBSCAN over turn vectors per repo (min_cluster_size=3); output = one consolidation per cluster
- [ ] 2.6 `src/producer/decision_trace.rs` — Decision-trace producer: walks `parent_event_id` chain up to 16 hops from a `Kind::Decision`; output = grain=DecisionTrace
- [ ] 2.7 `src/orchestrator.rs` — chooses producer based on trigger source; auto-promotes to Opus when grain=DecisionTrace OR session has outcome=success+high-impact
- [ ] 2.8 `src/cost_telemetry.rs` — emit per-grain $/consolidation + total monthly burn; gauge surfaces in `/v1/health/coverage`
- [ ] 2.9 CLI binary `crates/cortex-cli/src/bin/cortex-consolidator.rs` with subcommands: `run-session <session_id>`, `run-topic --repo <slug>`, `run-decision <decision_id>`, `nightly --dry-run`
- [ ] 2.10 8 unit tests covering: prompt rendering per grain, summariser fallback Opus→Haiku on quota, producer dedupe (idempotent re-run), cost guardrail breach
- [ ] 2.11 IT `crates/cortex-consolidator/tests/end_to_end_it.rs` — seeds 1 fake session with 30 envelopes, runs session producer with mocked summariser, asserts emitted Consolidation envelope shape + source_event_ids correctness

## 3. Family / collection / Meili routing

- [ ] 3.1 Add `"consolidations"` to `FAMILIES` array in `crates/cortex-workers/src/fulltext/routing.rs:213`
- [ ] 3.2 Map `Kind::Consolidation -> "consolidations"` in `crates/cortex-workers/src/fulltext/routing.rs:family_for`
- [ ] 3.3 Map `Kind::Consolidation -> "consolidations"` in `crates/cortex-workers/src/embedder/routing.rs:family_for`
- [ ] 3.4 Bump `crates/cortex-workers/src/fulltext/settings.rs` to v3: add `grain`, `depth`, `model`, `consolidation_id` to `filterableAttributes` and `sortableAttributes`; add `outcome_distribution` to `searchableAttributes`
- [ ] 3.5 Add Vectorizer collection declarations `cortex.consolidation.fp32` (hot tier) and `cortex.consolidation.pq` (warm) in `crates/cortex-storage/src/collections.rs`
- [ ] 3.6 Add global Meili index name constant `cortex_consolidations` in `crates/cortex-storage/src/names.rs`
- [ ] 3.7 Update `cortex-bootstrap --apply-settings-only` to recognise v2→v3 upgrades and apply non-destructively
- [ ] 3.8 Update `crates/cortex-api/src/archive_loader.rs:envelope_to_hit` with a `Kind::Consolidation` case rendering title + summary preview for the keyword lane
- [ ] 3.9 IT `crates/cortex-workers/tests/consolidation_routing_it.rs` asserts a Consolidation envelope lands in the right family + collection + global Meili index

## 4. Query API + pre-thinking renderer

- [ ] 4.1 Add a consolidations lane to the `pre_change_context` and `similar_problems` strategies in `crates/cortex-api/src/strategies.rs` — vector against `cortex.consolidation.fp32`, keyword against `cortex_consolidations`
- [ ] 4.2 Replace "Past sessions" section in `crates/cortex-pre-thinking/src/render.rs` with "Consolidated context" when ≥ 1 consolidation matches; format `grain/id · date · title · ✓|✗|⚠ outcome` (one line ≈ 120 bytes); top-3 by similarity
- [ ] 4.3 Fallback rule: when zero consolidations match, fall back to the raw "Past sessions" section from 11i §4.1
- [ ] 4.4 Update `docs/specs/12-pre-thinking-injection.md` §Output with the Consolidated context section + worked example
- [ ] 4.5 Extend the relevance gold set (`crates/cortex-api/tests/fixtures/relevance-gold.json`) with 10 questions whose acceptable answers include consolidation IDs; assert mrr@10 holds against gold (gate same as 11i §4.5)

## 5. Pruning daemon

- [ ] 5.1 Add `src/pruner.rs` to `crates/cortex-claude-archive/`: walks consolidations, for each `source_event_id` checks age + tier, demotes per the 0-7d / 7-90d / 90-365d / > 365d schedule
- [ ] 5.2 Demotion API: Vectorizer `move_to_collection` (hot→warm→cold); Meili `update_documents` reducing fields on cold-tier entries
- [ ] 5.3 Hard-purge path: callable from the new `/cortex forget <event_id>` MCP tool (cortex-mcp-server); requires confirmation token; cascades to all backends + Parquet archive
- [ ] 5.4 Cron schedule: prune runs nightly at 03:00 local time; configurable via `cortex.toml [cortex.consolidation] prune_at = "03:00"`
- [ ] 5.5 IT `crates/cortex-claude-archive/tests/pruner_it.rs` — seeds 100 raw events spanning 5 age buckets + 20 consolidations referencing them; asserts post-prune doc counts match expected per-tier targets
- [ ] 5.6 IT `pruner_safety_it.rs` — asserts no `source_event_id` referenced by an active consolidation is dropped before the consolidation itself expires
- [ ] 5.7 Surface pruner status in `/v1/health/coverage` under `pruner` block: last_run_ts, events_demoted_per_tier, events_purged

## 6. Tail (mandatory — enforced by rulebook v5.3.0)

- [ ] 6.1 Update or create documentation covering the implementation — CHANGELOG entry for phase 11j; update `docs/architecture.md` §6 (consolidation tier alongside raw + Parquet archive); `docs/cortex/consolidation-tuning.md` (cost guardrails, fidelity threshold tuning, prompt templates); update `docs/specs/16-dashboard.md` Memory view to surface consolidations as a filterable lane with grain + depth filters
- [ ] 6.2 Write tests covering the new behavior — every IT named in §1-§5 lands; coverage ≥ 95 % on `crates/cortex-consolidator/`; fidelity IT (`consolidation_fidelity_it`) samples 50 raw → consolidation pairs and asserts every `takeaways[]` entry has ≥ 1 supporting `source_event_id` (LLM-as-judge with Haiku 4.5; threshold ≥ 90 % shallow / ≥ 98 % deep)
- [ ] 6.3 Run tests and confirm they pass — `cargo check`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, `cargo test --all-features`, full IT suite gated by `CORTEX_*_IT=1` (CONSOLIDATION, FIDELITY, PRUNER); all green
- [ ] 6.4 Capture learnings: `rulebook_learn_capture` for any non-obvious finding from the implementation (HDBSCAN parameter tuning, prompt-template iterations, cost surprises)
- [ ] 6.5 Capture decision: `rulebook_decision_create` for the consolidation grain choice (Session / Topic / DecisionTrace) so future grain additions inherit the original rationale + fidelity threshold reasoning
