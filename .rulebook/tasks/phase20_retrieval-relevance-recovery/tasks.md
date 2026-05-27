## 1. Audit baseline + acceptance harness
- [x] 1.1 Snapshot current `/v1/status.coverage` (Vectorizer + Meili) into `docs/analysis/phase20-baseline/coverage-pre.json` — `status-pre.json` + `coverage-pre.json` captured. Both backends report 99% missing.
- [x] 1.2 Snapshot current `cortex_consolidations_recent` (limit 50) into `docs/analysis/phase20-baseline/consolidations-pre.json` — `consolidations-cortex-pre.json` (30 hits: 28 auto haiku + 2 manual) + `consolidations-global-pre.json` (empty — global index absent live)
- [x] 1.3 Write `scripts/phase20_acceptance.sh` — runs all 10 success-criteria probes and fails fast on any miss. Baseline run: PASS=2 FAIL=8. PASS on §1 (query snippets) + §5 (auto-generated consolidations); FAIL on §2/3 (coverage), §4 (topic_search), §6 (costs), §7 (lineage), §8 (graph n.id), §9 (law_id filter), §10 (active_work).

## 2. Data plane backfill (Vectorizer + Meili + consolidator)
- [ ] 2.1 Diagnose why Vectorizer dropped from full → 2/567 collections on 2026-05-27 restart (vec-data volume mount + re-ingest trigger)
- [ ] 2.2 Write + run `cortex-ops vectorizer-reseed` for all 17 repos; verify ≥95% coverage post-run
- [ ] 2.3 Document the re-seed runbook in `docs/runbooks/vectorizer-reseed.md`
- [ ] 2.4 Audit the 402 missing Meili indexes; restart `cortex-fulltext-worker` with `--backfill-missing` (or equivalent one-shot)
- [ ] 2.5 Verify Meili coverage ≥95% post-backfill
- [ ] 2.6 Audit `cron_jobs.retention.consolidator_nightly` last 7 runs in the metadata DB
- [ ] 2.7 If consolidator daemon is silent, trace cron scheduler → producer → publisher and ship the missing wire
- [ ] 2.8 Run consolidator on-demand for every repo with ≥5 sessions; confirm ≥3 auto-generated docs land

## 3. Graph writer — stamp node properties
- [ ] 3.1 Enumerate every node label currently producing `keys(n) = []` via `MATCH (n:<lbl>) WHERE size(keys(n)) = 0 RETURN labels(n)[0], count(*)`
- [ ] 3.2 For each empty-keys label, extend the writer's property projector with `id`, `repo`, `kind`, `ts`, plus label-specific key
- [ ] 3.3 Adopt ADR-004 `_id` slot — `MATCH (n {id: $id})` resolves via Nexus external-id index, not full scan
- [ ] 3.4 Add property-projection unit tests per label family in `crates/cortex-workers/src/graph/projector_tests.rs`
- [ ] 3.5 Re-run graph backfill via `cortex-ops graph backfill`
- [ ] 3.6 Verify `cortex_graph_query?mode=neighbors` returns non-empty `n.id` for every label family

## 4. Topic cards — wire end-to-end
- [ ] 4.1 Trace `cortex-classifier-worker` topic-card emission path; confirm `topic_card` envelopes are actually published
- [ ] 4.2 If publishing stopped, fix the producer (taxonomy version drift, schema validation failure, etc.)
- [ ] 4.3 Provision the per-repo `cortex-<slug>-topic_cards` Meili index with the canonical settings (filterable: `topics`, `repo`; sortable: `ts`)
- [ ] 4.4 Seed via classifier worker re-ingest
- [ ] 4.5 Acceptance: `cortex_topic_search?topic_prefix=tool:claude-code` returns ≥1 card per active repo

## 5. Consolidation cost telemetry
- [ ] 5.1 Locate `apply_extensions(Kind::Consolidation)` in `crates/cortex-workers/src/fulltext/builders.rs`
- [ ] 5.2 Project `cost_cents`, `prompt_tokens`, `completion_tokens`, `model_name`, `ts` (envelope `occurred_at`) onto the Meili doc
- [ ] 5.3 Update the per-repo `consolidations` index schema: add `cost_cents`, `prompt_tokens`, `completion_tokens` to `filterableAttributes` + `sortableAttributes`
- [ ] 5.4 Backfill existing consolidations via a one-shot re-projection job (read source envelope → re-write Meili doc)
- [ ] 5.5 Acceptance: `cortex_consolidation_costs?group_by=["model","grain"]` returns non-empty buckets

## 6. Consolidation lineage — extend extractor
- [ ] 6.1 Add `extract_decisions_from_body` — regex-scan `body` / `summary_markdown` for `DEC-\d{3,}` mentions (already partial; extend to `(?:decision[: ])?\d{3,}`)
- [ ] 6.2 Add `extract_files_from_body` — regex-scan for `[label](path/with/slashes)` markdown links + bare ``code-fenced paths``
- [ ] 6.3 Add `extract_sessions_from_body` — match `session[: ]?<ULID>` patterns
- [ ] 6.4 Add `references` JSON nested extractor for docs that embed lineage in a structured side-channel
- [ ] 6.5 Acceptance: `cortex_consolidation_lineage` against `cons-ses-278bab11ad68aa5756df653d` returns non-empty decisions/files/sessions

## 7. Filterable attributes — finish the schema
- [ ] 7.1 Audit `cortex-<slug>-governance` index settings: confirm `law_id`, `severity`, `session_id` in `filterableAttributes`
- [ ] 7.2 If missing, ship a schema migration in `crates/cortex-storage/schemas/meili/governance.settings.v2.json`
- [ ] 7.3 Re-apply settings to all per-repo governance indexes
- [ ] 7.4 Acceptance: `cortex_law_violations?law_id=LAW-CORTEX-001` returns matching subset
- [ ] 7.5 Verify `decision_status` is filterable on every `cortex-<slug>-decisions` index (phase19 §1.4 coverage check)

## 8. Decision promotion workflow
- [ ] 8.1 Document the manual promotion path: `rulebook_decision_update --status accepted` lands the supersession + re-stamps `decision_status` on the envelope
- [ ] 8.2 Add a dashboard view "Proposed ADRs older than 30 days" so stuck decisions surface
- [ ] 8.3 (Optional) CI rule: when a feature commit references `DEC-NNN` and the ADR is `proposed`, prompt for promotion

## 9. Fusion — drop placeholder vector hits
- [ ] 9.1 Locate the RRF assembler in `crates/cortex-api/src/orchestrator.rs` (or `fusion.rs`)
- [ ] 9.2 Add a pre-fusion filter: drop vector hits where `text.trim().is_empty()` or `text.len() < 32`
- [ ] 9.3 Add a unit test in `fusion.rs::tests` that proves an empty-text hit is dropped before RRF
- [ ] 9.4 Acceptance: top-3 `cortex_query` results carry `text` ≥100 chars on 10 sample queries

## 10. Pre-thinking feedback capture
- [ ] 10.1 Add `cortex_feedback_record` MCP tool to `crates/cortex-mcp-server/src/tools.rs` (args: `query_id`, `helpful: bool`, `intent`, `note?`)
- [ ] 10.2 Add `POST /v1/feedback` handler in `cortex-api` that writes to the existing `pre_thinking_feedback` SQLite table
- [ ] 10.3 Wire the Claude Code plugin's post-thinking hook to invoke the new tool on every bundle
- [ ] 10.4 Acceptance: after one bundle + feedback call, `cortex_feedback_signals?limit=1` returns the row

## 11. Active work surfacing
- [ ] 11.1 Trace `cortex_active_work` (cortex-api `/v1/active-work` endpoint or equivalent) against `.rulebook/tasks/*/.metadata.json`
- [ ] 11.2 Fix the path resolution or cache-invalidation bug that hides `phase19` even though it is on disk
- [ ] 11.3 Acceptance: `cortex_active_work` returns the active task with the next unchecked checklist item

## 12. Graph lane budget hygiene
- [ ] 12.1 After §3 (graph properties), re-measure `query_explain` graph_ms — confirm seed lookup is O(1) hash via `_id` slot
- [ ] 12.2 If still over budget for some scopes, add a graph-lane bypass when `scope.repo + scope.topics` has no graph anchor
- [ ] 12.3 Acceptance: `query_explain` for the 10 sample queries shows `graph_ms` < 200ms on ≥8/10 runs, no `budget exceeded` for indexed seeds

## 13. Phase19 tail bugs
- [ ] 13.1 Fix `cortex_consolidations_by_entity?entity.kind=decision_id`: route through per-repo cascade when no `repo` is supplied (mirror `consolidations_search`)
- [ ] 13.2 Add an integration test against the per-repo `cortex-cortex-consolidations` index for `entity.kind=decision_id`
- [ ] 13.3 Re-validate `cortex_similar_sessions` post §2 (Vectorizer backfill) — expect non-empty results for known-good queries

## 14. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 14.1 Update or create documentation covering the implementation
- [ ] 14.2 Write tests covering the new behavior
- [ ] 14.3 Run tests and confirm they pass
