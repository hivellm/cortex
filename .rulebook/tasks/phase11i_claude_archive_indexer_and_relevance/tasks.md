## 1. cortex-claude-archive crate (week 1)

- [ ] 1.1 Create `crates/cortex-claude-archive/` skeleton (Cargo.toml, lib.rs, deps: cortex-core, cortex-storage, serde_json, notify, zstd, tokio, tracing, anyhow, ulid)
- [ ] 1.2 `src/reader.rs` — streaming JSONL parser tolerant to incomplete final line, out-of-order parentUuid, and unknown attachment subtypes (warn + drop; never panic)
- [ ] 1.3 `src/mapper.rs` — JSONL → Envelope: pair user↔assistant by parentUuid into Kind::Turn; emit Kind::ToolCall per assistant.tool_use + matching attachment.tool_result; route Agent invocations to Kind::AgentCall; sidecars (history.jsonl, todos/, plans/) to Kind::Memory or Kind::Artifact
- [ ] 1.4 `src/walker.rs` — directory traversal w/ excludes; CLI flags --root (default `~/.claude/projects/`), --projects-only, --sidecars, --codex
- [ ] 1.5 `src/emitter.rs` — two sinks: `synap` (publish to cortex.events.bootstrap via cortex-synap SDK) and `archive` (zstd-NDJSON parquet under `<CORTEX_ARCHIVE_ROOT>/events/year=YYYY/month=MM/day=DD/hour=HH/bootstrap-claude-NNNNN.parquet`)
- [ ] 1.6 `src/checkpoint.rs` — atomic write of `(project_dir, session_id, last_record_uuid, last_byte_offset)` every 5 s; `--resume` advances past records ≤ checkpoint
- [ ] 1.7 CLI binary `crates/cortex-cli/src/bin/cortex-claude-archive.rs` with subcommands `bootstrap`, `tail`, `estimate` and a progress bar (indicatif) for bootstrap
- [ ] 1.8 Extend `crates/cortex-core/src/redact.rs` with patterns for Anthropic `sk-ant-`, OpenAI `sk-`, GitHub `ghp_`, AWS `AKIA`, Google `AIza`, generic JWT
- [ ] 1.9 Unit tests: 8 in mapper.rs covering each JSONL record type (user, assistant text, assistant thinking, assistant tool_use, attachment tool_result, attachment hook, system local_command, file-history-snapshot)
- [ ] 1.10 Fixture-based ITs: tiny (12-line Rulebook session), medium (1.4 MB Cortex session), large (a representative UzEngine 100k-line session) — assert envelope shape, redaction applied, no panics
- [ ] 1.11 Watcher IT: spawn `tail` against fake project dir, write records, assert envelopes appear within 2 s
- [ ] 1.12 `cortex-claude-archive estimate --root C:/Users/Bolado/.claude/projects/` reports ≈ 9 835 files / ≈ 2.4 M envelopes; write the actual numbers into `docs/analysis/organize/findings.json`

## 2. Classifier + family wiring (week 2 day 1-2)

- [ ] 2.1 `crates/cortex-workers/src/classifier/kinds.rs:19` — extend `kind_from_bootstrap` with `"turn.claude-code"`, `"tool_call.claude-code"`, `"agent_call.claude-code"`
- [ ] 2.2 `crates/cortex-classifier/src/statics.rs` — topic rule: any envelope with `tool == "claude-code"` adds `topics.push("claude-code")`; mirror for `tool == "openai-codex"` → `topics.push("openai-codex")`
- [ ] 2.3 IT `crates/cortex-workers/tests/classifier_claude_archive_it.rs` asserts kind / family / topic stamping for one envelope of each shape

## 3. Relevance axes (week 2 day 3 → week 3)

- [ ] 3.1 Recency decay — add `Scope.recency_decay: Option<f32>` to cortex-core types; implement `exp(-λ·days_old)` multiplier in `crates/cortex-api/src/fusion.rs`; per-intent defaults (pre_change_context: 0.02, decision_lookup: 0.005, law_check: 0.0); IT `relevance_recency_it.rs` with seeded turns at fixed offsets
- [ ] 3.2 Cross-repo boost — add `Scope.cross_repo_boost: f32`; orchestrator forks parallel lane scan when boost > 0; IT `relevance_cross_repo_it.rs` (in-repo first, cross-repo at reduced weight)
- [ ] 3.3 Author + model — bump `crates/cortex-workers/src/fulltext/settings.rs` to `settings.v2.json` adding `model` + `tool` to filterableAttributes; new flag `cortex-bootstrap --apply-settings-only` to push v2 to live indexes; add `Scope.models`, `Scope.tools`; alias table for model-name drift; IT `relevance_model_it.rs`
- [ ] 3.4 Session cohesion — add `session_id` to Meili filterable (settings v2 above); `Scope.session_id` + `Scope.session_cohort` in cortex-core; fusion: same-session ×2.0, cohort ×1.5; IT `relevance_session_it.rs`
- [ ] 3.5 Outcome signal — classifier worker derives `Turn.outcome` from child ToolCall outcomes + assistant `stop_reason`; emit on top-level Meili field + Vectorizer payload (settings v2); add `Scope.outcomes` + `Scope.exclude_outcomes`; fusion: success ×1.2, error ×0.5, blocked_by_law ×0.3; IT `relevance_outcome_it.rs`
- [ ] 3.6 Combined config — new file `crates/cortex-api/config/relevance.toml` with all multiplier defaults; `cortex-api` reads at boot, reloads on SIGHUP; IT `relevance_config_reload_it.rs`

## 4. Pre-thinking surfaces + measurement (week 3)

- [ ] 4.1 New section "Past sessions" in `crates/cortex-pre-thinking/src/render.rs` — one line per session: id, date, first user prompt (clipped 80 chars), turn count; top-3 by centroid similarity; respects 32 KiB clipper budget
- [ ] 4.2 Outcome glyph (`✓` / `✗` / `⚠`) on every turn + decision line in pre-thinking renderer
- [ ] 4.3 Update `docs/specs/12-pre-thinking-injection.md` §Output with the two new sections and example bundle
- [ ] 4.4 Hand-curate `crates/cortex-api/tests/fixtures/relevance-gold.json` with 30 questions, each w/ 1-3 acceptable result IDs covering: pre_change_context (10), decision_lookup (5), similar_problems (10), law_check (3), free_search (2)
- [ ] 4.5 IT `relevance_eval_it.rs` — boots daemon against fixture corpus, fires every gold question, computes `mrr@10` + `ndcg@10`; gated by `CORTEX_RELEVANCE_IT=1`; fails when `mrr@10 < 0.75`
- [ ] 4.6 `docs/cortex/relevance-tuning.md` — gold-set authoring process, how to add a question, how to interpret eval output, when to re-tune relevance.toml

## 5. Watcher daemon + ops (week 4 day 1-2)

- [ ] 5.1 `docker-compose.yml` — new service `cortex-claude-archive` running `cortex-claude-archive tail`, read-only bind mount `~/.claude/projects/` → `/data/claude-projects:ro`, depends_on synap + cortex-ingestion, restart unless-stopped
- [ ] 5.2 Health endpoint `:17030/healthz` returning last_flush_ts, files_watched, envelope_rate, rss_bytes; surface in `/v1/health/coverage` under a `archive_watchers` block
- [ ] 5.3 RSS hard cap ≤ 512 MiB enforced via assert in IT `cortex_claude_archive_memory_it.rs` (run watcher against 100 k-event fixture, sample RSS, fail if > cap)
- [ ] 5.4 Failure mode IT: corrupt JSONL line → warn + drop + counter increment; never panic; assert `envelopes_dropped` metric increments
- [ ] 5.5 README under `crates/cortex-claude-archive/README.md` documenting CLI subcommands, sinks, checkpoint format, expected resource footprint

## 6. Tail (mandatory — enforced by rulebook v5.3.0)

- [ ] 6.1 Update or create documentation covering the implementation — CHANGELOG entry for phase 11i, `docs/architecture.md` §6 (conversation archive listed alongside git repos), `docs/specs/16-dashboard.md` Memory view section surfacing claude-archive turns, `docs/cortex/relevance-tuning.md` (already produced by §4.6)
- [ ] 6.2 Write tests covering the new behavior — every IT named in §1-§5 lands; coverage ≥ 95 % on `crates/cortex-claude-archive/`; relevance gold-set IT is the headline acceptance gate
- [ ] 6.3 Run tests and confirm they pass — `cargo check`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, `cargo test --all-features`, full IT suite gated by `CORTEX_*_IT=1` (RELEVANCE, COVERAGE, ARCHIVE); all green
- [ ] 6.4 Capture learnings: `rulebook_learn_capture` for any non-obvious finding from the implementation (parser edge cases, RRF tuning lessons, etc.)
- [ ] 6.5 Capture decision: `rulebook_decision_create` for the recency-decay constants chosen (λ per intent) so future tuning has the original rationale to compare against
