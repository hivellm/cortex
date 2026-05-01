## 1. Kind::Consolidation + payload

- [x] 1.1 `Kind::Consolidation` variant landed in `crates/cortex-core/src/events.rs` with the phase11j docstring; `kind` enum on the wire-side `envelope.schema.json` extended with `consolidation` (and the previously-stale `knowledge` / `learning` entries — pre-existing drift fixed forward so the per-kind validator stops surfacing UnknownKind on those).
- [x] 1.2 New types in `events.rs`: `ConsolidationPayload`, `ConsolidationGrain` (Session/Topic/DecisionTrace), `ConsolidationDepth` (Shallow/Deep), `ConsolidationScope` (tagged union with `kind`/`value`), `TimeSpan` (start/end/duration ms with materialised duration). Carries `consolidation_id`, `title`, `summary_markdown`, `takeaways`, `source_event_ids` + `source_event_count` (count holds full count even when ids vec is clipped at the §1.5 inline cap), `model`, `depth`, `outcome_distribution`, `temporal_span`, `repos`, `tags`. Public constants `CONSOLIDATION_SOURCE_IDS_INLINE_CAP = 256`, `CONSOLIDATION_TITLE_MAX_CHARS = 80`, `CONSOLIDATION_SUMMARY_MIN_BYTES = 200`, `CONSOLIDATION_SUMMARY_MAX_BYTES = 2_000`.
- [x] 1.3 `Kind::schema_stem()` extended with `Kind::Consolidation => "consolidation"`. Kind derives Debug only — no manual Display impl exists in events.rs, so the Debug output picks up the new variant automatically.
- [x] 1.4 4 unit tests in `events.rs::consolidation_tests`: serde round-trip pinning the wire shape (`grain`/`scope.kind`/`depth` snake_case discriminators), `schema_stem` mapping, `ConsolidationGrain` snake_case serialisation for all three variants, and the `source_event_count >= ids.len()` invariant when the ids vec is clipped at the inline cap.
- [x] 1.5 New JSON Schema `crates/cortex-core/schemas/kinds/consolidation.schema.json` (consolidation kind wired into the `Validator::kinds` map) enforces title ≤ 80 chars, summary 200–2 000 bytes, scope discriminator enum, integer ranges. Cross-field rule (scope variant must match grain) is enforced by the new `validate_consolidation_payload(&ConsolidationPayload)` Rust helper because JSON Schema cannot express it cleanly. Helper also enforces `source_event_count >= ids.len()`, `temporal_span.end_ms >= start_ms`, and `duration_ms == end_ms - start_ms`. 8 unit tests in `validate.rs::consolidation_validate_tests` (5 Rust-validator scenarios — accept/reject grain-scope match, count-below-len, inverted span, inconsistent duration; 3 JSON-schema scenarios — minimal payload accepted, summary below floor rejected with `/payload/summary_markdown` path, unknown grain rejected).

## 2. cortex-consolidator crate

- [x] 2.1 New crate `crates/cortex-consolidator/` shipped + added to workspace members. Cargo.toml depends on cortex-core, cortex-storage, serde, serde_json, thiserror, chrono, anyhow, tokio, tracing, ulid, reqwest, async-trait. Note: `anthropic-sdk` is NOT a real Rust crate (Anthropic ships only Python/TS SDKs) — the live API path uses reqwest directly, the same pattern the rest of the workspace uses for HTTP. The `hdbscan` workspace dep lands alongside §2.5's topic producer body so the `cargo metadata` graph stays tight while §2.1 ships. lib.rs declares modules `templates`, `summariser`, `producer`, `orchestrator`, `cost_telemetry` + exposes them via `prelude`.
- [x] 2.2 Three real prompt templates land under `crates/cortex-consolidator/templates/` (`session.md`, `topic.md`, `decision_trace.md`). Each has a header section declaring inputs (`{{key}}` slots the producer fills), a "Source" block, and an output-contract block pinning the JSON shape (`title` / `summary_markdown` / `takeaways`). `templates.rs` resolves them via `Template::for_grain` and renders via plain `{{key}}` substitution; per-grain `max_output_bytes = 2_000` and `takeaways_count = 3 / 5 / 7` for Session / Topic / DecisionTrace match the proposal. 3 unit tests pin distinctness across grains, the takeaways-count contract, and the render fallback (untouched slots stay literal so a typo surfaces).
- [x] 2.3 `summariser.rs` ships the abstract `Summariser` trait + the live `AnthropicSummariser` impl that hits `POST {base}/v1/messages` (Anthropic Messages API) with `x-api-key` + `anthropic-version` headers and parses the `usage.input_tokens` / `usage.output_tokens` block. Pricing constants `HAIKU45_*` / `OPUS47_*` (USD micro-cents per 1k tokens) drive the `cost_cents()` helper that rounds half-up. Errors split into `RateLimited { retry_after_ms }` (429 → orchestrator backs off), `UpstreamUnavailable` (5xx → orchestrator retries once + falls back), `Upstream { status, body }` (other 4xx → terminal), `Transport`, `CostCeiling`. 4 unit tests pin cost scaling (Opus 1M+1M tokens = 9 000 cents = $90), micro-cent rounding edge case, base-URL override, and `SummariserKind` snake-case serde round-trip. `ANTHROPIC_API_URL` env var overrides the base for staging deployments.
- [ ] 2.4 `src/producer/session.rs` — Session producer: input = all envelopes for a `session_id`; output = `Kind::Consolidation` with grain=Session
- [ ] 2.5 `src/producer/topic.rs` — Topic producer: HDBSCAN over turn vectors per repo (min_cluster_size=3); output = one consolidation per cluster
- [ ] 2.6 `src/producer/decision_trace.rs` — Decision-trace producer: walks `parent_event_id` chain up to 16 hops from a `Kind::Decision`; output = grain=DecisionTrace
- [ ] 2.7 `src/orchestrator.rs` — chooses producer based on trigger source; auto-promotes to Opus when grain=DecisionTrace OR session has outcome=success+high-impact
- [x] 2.8 `cost_telemetry.rs` ships `GrainCost` (`consolidations` count + `cost_cents` total + `mean_cost_cents()`), `CostLedger` (per-grain `BTreeMap` keyed by snake-case grain label + `total_cents`; `record(grain_label, cost_cents)` + `reset()`), and `CostBudget` (default $1 000/month = 100 000 cents) with `remaining_cents()` + `can_afford(est_cents)`. Orchestrator constructs + holds the ledger; per-grain $/consolidation is derivable for the `/v1/health/coverage` block (live wiring drops in alongside §2.7 producer dispatch). 3 unit tests pin per-grain accumulation, reset, and the budget-cap gate.
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
